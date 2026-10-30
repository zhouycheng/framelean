use crate::Operation;
use rusty_ffmpeg::ffi as av;
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString},
    path::Path,
    ptr,
};
type Result<T> = std::result::Result<T, String>;
fn cs(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| "路径包含 NUL 字符".into())
}
fn check(n: i32) -> Result<()> {
    if n >= 0 {
        return Ok(());
    }
    let mut b = [0i8; 256];
    unsafe { av::av_strerror(n, b.as_mut_ptr(), b.len()) };
    Err(unsafe { CStr::from_ptr(b.as_ptr()) }
        .to_string_lossy()
        .into_owned())
}
struct Input(*mut av::AVFormatContext);
impl Input {
    fn open(path: &str) -> Result<Self> {
        let mut p = ptr::null_mut();
        check(unsafe {
            av::avformat_open_input(&mut p, cs(path)?.as_ptr(), ptr::null(), ptr::null_mut())
        })?;
        let i = Self(p);
        check(unsafe { av::avformat_find_stream_info(p, ptr::null_mut()) })?;
        Ok(i)
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        unsafe { av::avformat_close_input(&mut self.0) }
    }
}
struct Codec(*mut av::AVCodecContext);
impl Drop for Codec {
    fn drop(&mut self) {
        unsafe { av::avcodec_free_context(&mut self.0) }
    }
}
struct Frame(*mut av::AVFrame);
impl Drop for Frame {
    fn drop(&mut self) {
        unsafe { av::av_frame_free(&mut self.0) }
    }
}
struct Packet(*mut av::AVPacket);
impl Drop for Packet {
    fn drop(&mut self) {
        unsafe { av::av_packet_free(&mut self.0) }
    }
}
struct Output(*mut av::AVFormatContext);
impl Drop for Output {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                if !(*self.0).pb.is_null() {
                    av::avio_closep(&mut (*self.0).pb);
                }
                av::avformat_free_context(self.0)
            }
        }
    }
}
fn frame() -> Result<Frame> {
    let p = unsafe { av::av_frame_alloc() };
    if p.is_null() {
        Err("frame allocation failed".into())
    } else {
        Ok(Frame(p))
    }
}
fn packet() -> Result<Packet> {
    let p = unsafe { av::av_packet_alloc() };
    if p.is_null() {
        Err("packet allocation failed".into())
    } else {
        Ok(Packet(p))
    }
}
fn codec(c: *const av::AVCodec) -> Result<Codec> {
    if c.is_null() {
        return Err("所需编解码器不可用".into());
    }
    let p = unsafe { av::avcodec_alloc_context3(c) };
    if p.is_null() {
        Err("codec allocation failed".into())
    } else {
        Ok(Codec(p))
    }
}
fn streams(i: &Input) -> &[*mut av::AVStream] {
    unsafe { std::slice::from_raw_parts((*i.0).streams, (*i.0).nb_streams as usize) }
}
pub fn versions() -> Value {
    unsafe {
        json!({"avcodec":av::avcodec_version(),"avformat":av::avformat_version(),"avutil":av::avutil_version(),"encoder":!av::avcodec_find_encoder_by_name(c"libx264".as_ptr()).is_null()})
    }
}
fn inspect(i: &Input) -> Result<Value> {
    unsafe {
        let format = CStr::from_ptr((*(*i.0).iformat).name).to_string_lossy();
        if !format.split(',').any(|x| x == "mov" || x == "mp4") {
            return Err("P0 仅支持 MP4/MOV 容器".into());
        }
        let mut video = None;
        let mut audio = None;
        for (idx, &s) in streams(i).iter().enumerate() {
            let p = &*(*s).codecpar;
            match p.codec_type {
                av::AVMEDIA_TYPE_VIDEO => {
                    if video.is_some()
                        || p.codec_id != av::AV_CODEC_ID_H264
                        || p.format != av::AV_PIX_FMT_YUV420P
                        || p.width <= 0
                        || p.height <= 0
                        || p.width % 2 != 0
                        || p.height % 2 != 0
                    {
                        return Err("P0 仅支持单轨 H.264 8-bit yuv420p 偶数尺寸视频".into());
                    }
                    if p.color_trc == av::AVCOL_TRC_SMPTE2084
                        || p.color_trc == av::AVCOL_TRC_ARIB_STD_B67
                        || p.color_primaries == av::AVCOL_PRI_BT2020
                    {
                        return Err("P0 暂不支持 HDR/BT.2020".into());
                    }
                    if !av::av_packet_side_data_get(
                        p.coded_side_data,
                        p.nb_coded_side_data,
                        av::AV_PKT_DATA_DISPLAYMATRIX,
                    )
                    .is_null()
                        || !av::av_dict_get((*s).metadata, c"rotate".as_ptr(), ptr::null(), 0)
                            .is_null()
                    {
                        return Err("P0 暂不支持带旋转/显示矩阵的素材".into());
                    }
                    if p.sample_aspect_ratio.num > 0
                        && p.sample_aspect_ratio.num != p.sample_aspect_ratio.den
                    {
                        return Err("P0 暂不支持非方形像素".into());
                    }
                    video = Some(
                        json!({"index":idx,"codec":"h264","width":p.width,"height":p.height,"pixel_format":"yuv420p"}),
                    );
                }
                av::AVMEDIA_TYPE_AUDIO => {
                    if audio.is_some() || p.codec_id != av::AV_CODEC_ID_AAC {
                        return Err("P0 仅支持无音轨或单 AAC 音轨".into());
                    }
                    audio = Some(
                        json!({"index":idx,"codec":"aac","sample_rate":p.sample_rate,"channels":p.ch_layout.nb_channels}),
                    );
                }
                _ => return Err("P0 暂不支持字幕、附件或数据轨".into()),
            }
        }
        let video = video.ok_or("未找到视频轨")?;
        if (*i.0).duration <= 0 {
            return Err("无法确定视频时长".into());
        }
        Ok(json!({"duration_us":(*i.0).duration,"video":video,"audio":audio,"container":"mp4/mov"}))
    }
}
pub fn run(r: &Value, op: &Operation) -> Result<Value> {
    let path = r["input"].as_str().ok_or("缺少输入路径")?;
    if !Path::new(path).is_file() {
        return Err("输入文件不存在".into());
    }
    let input = Input::open(path)?;
    let info = inspect(&input)?;
    op.check()?;
    if r["kind"] == "analyze" {
        return Ok(info);
    }
    if r["kind"] != "execute" {
        return Err("未知操作".into());
    }
    let out = r["temp"].as_str().ok_or("缺少输出路径")?;
    transcode(&input, &info, out, op)?;
    op.check()?;
    op.update(json!({"state":"validating","progress":0.99}));
    let result = inspect(&Input::open(out)?)?;
    let duration = info["duration_us"].as_i64().unwrap();
    if (result["duration_us"].as_i64().unwrap() - duration).abs() > 250_000
        || result["video"]["width"] != info["video"]["width"]
        || result["video"]["height"] != info["video"]["height"]
        || result["audio"].is_null() != info["audio"].is_null()
    {
        return Err("输出校验失败：时长、尺寸或轨道不一致".into());
    }
    op.check()?;
    Ok(result)
}
fn transcode(input: &Input, info: &Value, path: &str, op: &Operation) -> Result<()> {
    unsafe {
        let vi = info["video"]["index"].as_u64().unwrap() as usize;
        let audio_idx = info["audio"]["index"].as_u64().map(|x| x as usize);
        let vs = streams(input)[vi];
        let vp = &*(*vs).codecpar;
        let dc = av::avcodec_find_decoder(vp.codec_id);
        let decoder = codec(dc)?;
        check(av::avcodec_parameters_to_context(decoder.0, (*vs).codecpar))?;
        (*decoder.0).pkt_timebase = (*vs).time_base;
        (*decoder.0).thread_count = 2;
        check(av::avcodec_open2(decoder.0, dc, ptr::null_mut()))?;
        let ec = av::avcodec_find_encoder_by_name(c"libx264".as_ptr());
        let encoder = codec(ec)?;
        (*encoder.0).width = vp.width;
        (*encoder.0).height = vp.height;
        (*encoder.0).pix_fmt = av::AV_PIX_FMT_YUV420P;
        (*encoder.0).time_base = (*vs).time_base;
        (*encoder.0).framerate = av::av_guess_frame_rate(input.0, vs, ptr::null_mut());
        (*encoder.0).sample_aspect_ratio = av::AVRational { num: 1, den: 1 };
        (*encoder.0).color_primaries = vp.color_primaries;
        (*encoder.0).color_trc = vp.color_trc;
        (*encoder.0).colorspace = vp.color_space;
        (*encoder.0).color_range = vp.color_range;
        (*encoder.0).thread_count = 2;
        let mut out = ptr::null_mut();
        check(av::avformat_alloc_output_context2(
            &mut out,
            ptr::null(),
            c"mp4".as_ptr(),
            cs(path)?.as_ptr(),
        ))?;
        let output = Output(out);
        if (*(*out).oformat).flags & av::AVFMT_GLOBALHEADER as i32 != 0 {
            (*encoder.0).flags |= av::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
        check(av::av_opt_set(
            (*encoder.0).priv_data,
            c"crf".as_ptr(),
            c"23".as_ptr(),
            0,
        ))?;
        check(av::av_opt_set(
            (*encoder.0).priv_data,
            c"preset".as_ptr(),
            c"medium".as_ptr(),
            0,
        ))?;
        check(av::avcodec_open2(encoder.0, ec, ptr::null_mut()))?;
        let ov = av::avformat_new_stream(out, ptr::null());
        if ov.is_null() {
            return Err("output stream allocation failed".into());
        }
        check(av::avcodec_parameters_from_context(
            (*ov).codecpar,
            encoder.0,
        ))?;
        (*ov).time_base = (*vs).time_base;
        let oa = if let Some(ai) = audio_idx {
            let a = av::avformat_new_stream(out, ptr::null());
            if a.is_null() {
                return Err("audio allocation failed".into());
            }
            check(av::avcodec_parameters_copy(
                (*a).codecpar,
                (*streams(input)[ai]).codecpar,
            ))?;
            (*(*a).codecpar).codec_tag = 0;
            (*a).time_base = (*streams(input)[ai]).time_base;
            Some(a)
        } else {
            None
        };
        check(av::avio_open(
            &mut (*out).pb,
            cs(path)?.as_ptr(),
            av::AVIO_FLAG_WRITE as i32,
        ))?;
        check(av::avformat_write_header(out, ptr::null_mut()))?;
        let pkt = packet()?;
        let encoded = packet()?;
        let frame = frame()?;
        let drain_encoder = || -> Result<()> {
            loop {
                let n = av::avcodec_receive_packet(encoder.0, encoded.0);
                if n == -35 || n == -11 || n == av::AVERROR_EOF {
                    break;
                }
                check(n)?;
                av::av_packet_rescale_ts(encoded.0, (*encoder.0).time_base, (*ov).time_base);
                (*encoded.0).stream_index = (*ov).index;
                check(av::av_interleaved_write_frame(out, encoded.0))?;
                av::av_packet_unref(encoded.0);
            }
            Ok(())
        };
        let drain_decoder = || -> Result<()> {
            loop {
                op.check()?;
                let n = av::avcodec_receive_frame(decoder.0, frame.0);
                if n == -35 || n == -11 || n == av::AVERROR_EOF {
                    break;
                }
                check(n)?;
                if (*frame.0).format != av::AV_PIX_FMT_YUV420P
                    || (*frame.0).width != vp.width
                    || (*frame.0).height != vp.height
                {
                    return Err("视频中途改变格式或尺寸".into());
                }
                (*frame.0).pts = (*frame.0).best_effort_timestamp;
                if (*frame.0).pts == av::AV_NOPTS_VALUE {
                    return Err("视频帧缺少时间戳".into());
                }
                (*frame.0).pict_type = av::AV_PICTURE_TYPE_NONE;
                let t = av::av_rescale_q(
                    (*frame.0).pts,
                    (*vs).time_base,
                    av::AVRational {
                        num: 1,
                        den: 1_000_000,
                    },
                );
                let start = if (*vs).start_time == av::AV_NOPTS_VALUE {
                    0
                } else {
                    av::av_rescale_q(
                        (*vs).start_time,
                        (*vs).time_base,
                        av::AVRational {
                            num: 1,
                            den: 1_000_000,
                        },
                    )
                };
                op.update(json!({"state":"running","progress":((t-start) as f64/info["duration_us"].as_f64().unwrap()).clamp(0.0,0.98)}));
                check(av::avcodec_send_frame(encoder.0, frame.0))?;
                drain_encoder()?;
                av::av_frame_unref(frame.0);
            }
            Ok(())
        };
        loop {
            op.check()?;
            let n = av::av_read_frame(input.0, pkt.0);
            if n == av::AVERROR_EOF {
                break;
            }
            check(n)?;
            let idx = (*pkt.0).stream_index as usize;
            if idx == vi {
                check(av::avcodec_send_packet(decoder.0, pkt.0))?;
                drain_decoder()?;
            } else if Some(idx) == audio_idx {
                let a = oa.unwrap();
                av::av_packet_rescale_ts(pkt.0, (*streams(input)[idx]).time_base, (*a).time_base);
                (*pkt.0).stream_index = (*a).index;
                (*pkt.0).pos = -1;
                check(av::av_interleaved_write_frame(out, pkt.0))?;
            }
            av::av_packet_unref(pkt.0);
        }
        check(av::avcodec_send_packet(decoder.0, ptr::null()))?;
        drain_decoder()?;
        check(av::avcodec_send_frame(encoder.0, ptr::null()))?;
        drain_encoder()?;
        check(av::av_write_trailer(out))?;
        drop(output);
        Ok(())
    }
}
