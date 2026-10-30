use crate::Operation;
use fll_contracts::{ExecutionPlan, MediaFacts, ResourceGrant};
use rusty_ffmpeg::ffi as av;
use std::{
    ffi::{CStr, CString},
    ptr,
};
pub(crate) type Result<T> = std::result::Result<T, String>;
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
pub(crate) struct Input(pub(crate) *mut av::AVFormatContext);
impl Input {
    pub(crate) fn open(path: &str, op: &Operation) -> Result<Self> {
        let path = cs(path)?;
        let mut p = unsafe { av::avformat_alloc_context() };
        if p.is_null() {
            return Err("format allocation failed".into());
        }
        unsafe {
            (*p).interrupt_callback = av::AVIOInterruptCB {
                callback: Some(interrupt),
                opaque: (&op.cancel as *const std::sync::atomic::AtomicBool)
                    .cast_mut()
                    .cast(),
            };
        }
        let mut options = ptr::null_mut();
        unsafe {
            av::av_dict_set(
                &mut options,
                c"protocol_whitelist".as_ptr(),
                c"file".as_ptr(),
                0,
            );
        }
        let opened =
            unsafe { av::avformat_open_input(&mut p, path.as_ptr(), ptr::null(), &mut options) };
        unsafe {
            av::av_dict_free(&mut options);
        }
        if opened < 0 {
            if !p.is_null() {
                unsafe {
                    av::avformat_close_input(&mut p);
                }
            }
            check(opened)?;
        }
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
pub(crate) fn compression_ready() -> bool {
    unsafe {
        let c = av::avcodec_find_encoder_by_name(c"libx264".as_ptr());
        let Ok(context) = codec(c) else {
            return false;
        };
        (*context.0).width = 16;
        (*context.0).height = 16;
        (*context.0).pix_fmt = av::AV_PIX_FMT_YUV420P;
        (*context.0).time_base = av::AVRational { num: 1, den: 24 };
        (*context.0).thread_count = 1;
        av::avcodec_open2(context.0, c, ptr::null_mut()) >= 0
    }
}
pub(crate) fn streams(i: &Input) -> &[*mut av::AVStream] {
    unsafe { std::slice::from_raw_parts((*i.0).streams, (*i.0).nb_streams as usize) }
}
unsafe extern "C" fn interrupt(opaque: *mut std::ffi::c_void) -> i32 {
    let cancel = unsafe { &*opaque.cast::<std::sync::atomic::AtomicBool>() };
    i32::from(cancel.load(std::sync::atomic::Ordering::Relaxed))
}
pub(crate) fn transcode(
    input: &Input,
    media: &MediaFacts,
    plan: &ExecutionPlan,
    grant: &ResourceGrant,
    path: &str,
    op: &Operation,
) -> Result<()> {
    unsafe {
        let vi = plan
            .stream_map
            .iter()
            .find(|m| m.mode == "transcode")
            .ok_or("missing video mapping")?
            .input as usize;
        let audio_idx = plan
            .stream_map
            .iter()
            .find(|m| m.mode == "copy")
            .map(|m| m.input as usize);
        let vs = streams(input)[vi];
        let vp = &*(*vs).codecpar;
        let dc = av::avcodec_find_decoder(vp.codec_id);
        let decoder = codec(dc)?;
        check(av::avcodec_parameters_to_context(decoder.0, (*vs).codecpar))?;
        (*decoder.0).pkt_timebase = (*vs).time_base;
        (*decoder.0).thread_count = grant.threads as i32;
        (*decoder.0).err_recognition =
            (av::AV_EF_CRCCHECK | av::AV_EF_BITSTREAM | av::AV_EF_BUFFER | av::AV_EF_EXPLODE)
                as i32;
        check(av::avcodec_open2(decoder.0, dc, ptr::null_mut()))?;
        let ec = av::avcodec_find_encoder_by_name(cs(&plan.parameters.encoder)?.as_ptr());
        let encoder = codec(ec)?;
        (*encoder.0).width = plan.parameters.width;
        (*encoder.0).height = plan.parameters.height;
        (*encoder.0).pix_fmt = av::av_get_pix_fmt(cs(&plan.parameters.pixel_format)?.as_ptr());
        (*encoder.0).time_base = (*vs).time_base;
        (*encoder.0).framerate = av::av_guess_frame_rate(input.0, vs, ptr::null_mut());
        (*encoder.0).sample_aspect_ratio = av::AVRational { num: 1, den: 1 };
        (*encoder.0).color_primaries = vp.color_primaries;
        (*encoder.0).color_trc = vp.color_trc;
        (*encoder.0).colorspace = vp.color_space;
        (*encoder.0).color_range = vp.color_range;
        (*encoder.0).thread_count = grant.threads as i32;
        // MP4 needs the final frame's duration to retain it in the presentation timeline.
        (*encoder.0).flags |= av::AV_CODEC_FLAG_FRAME_DURATION as i32;
        let mut out = ptr::null_mut();
        check(av::avformat_alloc_output_context2(
            &mut out,
            ptr::null(),
            cs(&plan.parameters.container)?.as_ptr(),
            cs(path)?.as_ptr(),
        ))?;
        let output = Output(out);
        if (*(*out).oformat).flags & av::AVFMT_GLOBALHEADER as i32 != 0 {
            (*encoder.0).flags |= av::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
        check(av::av_opt_set(
            (*encoder.0).priv_data,
            c"crf".as_ptr(),
            cs(&plan.parameters.crf.to_string())?.as_ptr(),
            0,
        ))?;
        check(av::av_opt_set(
            (*encoder.0).priv_data,
            c"preset".as_ptr(),
            cs(&plan.parameters.preset)?.as_ptr(),
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
                op.check().map_err(|e| e.message)?;
                let n = av::avcodec_receive_frame(decoder.0, frame.0);
                if n == -35 || n == -11 || n == av::AVERROR_EOF {
                    break;
                }
                check(n)?;
                if (*frame.0).decode_error_flags != 0
                    || (*frame.0).flags & av::AV_FRAME_FLAG_CORRUPT as i32 != 0
                {
                    return Err("视频帧损坏".into());
                }
                if (*frame.0).format != av::AV_PIX_FMT_YUV420P
                    || (*frame.0).width != vp.width
                    || (*frame.0).height != vp.height
                {
                    return Err("视频中途改变格式或尺寸".into());
                }
                if (*frame.0).color_primaries != vp.color_primaries
                    || (*frame.0).color_trc != vp.color_trc
                    || (*frame.0).colorspace != vp.color_space
                    || (*frame.0).color_range != vp.color_range
                {
                    return Err("视频帧色彩信息与分析结果不一致".into());
                }
                (*frame.0).pts = (*frame.0).best_effort_timestamp;
                if (*frame.0).pts == av::AV_NOPTS_VALUE {
                    return Err("视频帧缺少时间戳".into());
                }
                if (*frame.0).duration <= 0 {
                    return Err("视频帧缺少有效时长，无法保证完整时间线".into());
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
                op.progress(
                    "running",
                    Some(
                        ((t - start) as f64
                            / *media.duration_us.get().ok_or("duration unknown")? as f64)
                            .clamp(0.0, 0.98),
                    ),
                    Some(t - start),
                );
                check(av::avcodec_send_frame(encoder.0, frame.0))?;
                drain_encoder()?;
                av::av_frame_unref(frame.0);
            }
            Ok(())
        };
        loop {
            op.check().map_err(|e| e.message)?;
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
