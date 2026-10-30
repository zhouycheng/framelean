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
            build: concat!(env!("CARGO_PKG_VERSION"), "-media-1").into(),
            backend_versions: json!({"avcodec":av::avcodec_version(),"avformat":av::avformat_version(),"avutil":av::avutil_version(),"compatible":compatible,"compiled_majors":[av::LIBAVCODEC_VERSION_MAJOR,av::LIBAVFORMAT_VERSION_MAJOR,av::LIBAVUTIL_VERSION_MAJOR]}),
            compression_ready: compatible
                && crate::backends::libav::compression_ready()
                && !av::avcodec_find_decoder(av::AV_CODEC_ID_H264).is_null()
                && !av::av_guess_format(c"mp4".as_ptr(), std::ptr::null(), std::ptr::null())
                    .is_null(),
            capability_digest: String::new(),
        };
        c.capability_digest = digest(&c);
        c
    }
}
