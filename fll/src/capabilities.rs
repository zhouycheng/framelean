use fll_contracts::*;
use rusty_ffmpeg::ffi as av;
use serde_json::json;
pub(crate) fn snapshot() -> Capabilities {
    static SNAPSHOT: std::sync::OnceLock<Capabilities> = std::sync::OnceLock::new();
    SNAPSHOT.get_or_init(probe).clone()
}
fn probe() -> Capabilities {
    unsafe {
        let compatible = av::avcodec_version() >> 16 == av::LIBAVCODEC_VERSION_MAJOR
            && av::avformat_version() >> 16 == av::LIBAVFORMAT_VERSION_MAJOR
            && av::avutil_version() >> 16 == av::LIBAVUTIL_VERSION_MAJOR;
        let mut c = Capabilities {
            json_version: JSON_VERSION,
            build: concat!(env!("CARGO_PKG_VERSION"), "-media-2").into(),
            backend_versions: json!({"avcodec":av::avcodec_version(),"avformat":av::avformat_version(),"avutil":av::avutil_version(),"compatible":compatible,"compiled_majors":[av::LIBAVCODEC_VERSION_MAJOR,av::LIBAVFORMAT_VERSION_MAJOR,av::LIBAVUTIL_VERSION_MAJOR]}),
            h264_encoder: compatible && crate::backends::libav::compression_ready(),
            video_decoders: [
                ("h264", av::AV_CODEC_ID_H264),
                ("hevc", av::AV_CODEC_ID_HEVC),
            ]
            .into_iter()
            .filter(|(_, id)| compatible && !av::avcodec_find_decoder(*id).is_null())
            .map(|(name, _)| name.into())
            .collect(),
            output_containers: [("mp4", c"mp4"), ("mkv", c"matroska")]
                .into_iter()
                .filter(|(_, name)| {
                    let muxer =
                        av::av_guess_format(name.as_ptr(), std::ptr::null(), std::ptr::null());
                    compatible
                        && !muxer.is_null()
                        && av::avformat_query_codec(
                            muxer,
                            av::AV_CODEC_ID_H264,
                            av::FF_COMPLIANCE_NORMAL as i32,
                        ) > 0
                        && av::avformat_query_codec(
                            muxer,
                            av::AV_CODEC_ID_AAC,
                            av::FF_COMPLIANCE_NORMAL as i32,
                        ) > 0
                })
                .map(|(name, _)| name.into())
                .collect(),
            capability_digest: String::new(),
        };
        c.capability_digest = digest(&c);
        c
    }
}
