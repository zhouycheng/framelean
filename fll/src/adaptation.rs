//! Central media constraints; diagnostics are separate from probe success.
use fll_contracts::*;
pub const RULES_VERSION: &str = "preserve-1";
pub(crate) fn evaluate(media: &MediaFacts) -> Vec<Diagnostic> {
    let mut result = Vec::new();
    let mut reject = |code: &str, message: &str, required: Vec<String>| {
        result.push(Diagnostic {
            code: code.into(),
            message: message.into(),
            required_facts: required,
        })
    };
    if !media
        .container
        .split(',')
        .any(|c| matches!(c, "mov" | "mp4"))
    {
        reject(
            "container_unsupported",
            "当前压缩链需要 MP4/MOV 容器",
            vec![],
        );
    }
    if media.duration_us.get().is_none_or(|d| *d <= 0) {
        reject("needs_analysis", "需要有效时长", vec!["duration_us".into()]);
    }
    let video = media
        .tracks
        .iter()
        .filter(|t| t.kind == "video")
        .collect::<Vec<_>>();
    let audio = media
        .tracks
        .iter()
        .filter(|t| t.kind == "audio")
        .collect::<Vec<_>>();
    if video.len() != 1 || audio.len() > 1 || media.tracks.len() != video.len() + audio.len() {
        reject(
            "track_mapping_unavailable",
            "当前压缩链只支持单视频与零或单 AAC 音轨",
            vec![],
        );
    }
    for t in &media.tracks {
        if t.time_base.get().is_none() {
            reject(
                "needs_analysis",
                "需要轨道时间基",
                vec![format!("tracks.{}.time_base", t.index)],
            );
        }
        if matches!(t.kind.as_str(), "video" | "audio") && t.start_time_us.get().is_none() {
            reject(
                "needs_analysis",
                "需要音视频起始时间",
                vec![format!("tracks.{}.start_time_us", t.index)],
            );
        }
    }
    for t in video {
        if t.codec.get().map(String::as_str) != Some("h264") {
            reject(
                "video_codec_unsupported",
                "当前压缩链需要 H.264 视频",
                vec![],
            );
        }
        if let Some(v) = &t.video {
            if v.pixel_format.get().map(String::as_str) != Some("yuv420p")
                || v.bit_depth.get() != Some(&8)
            {
                reject(
                    "pixel_format_unsupported",
                    "当前压缩链需要 8-bit yuv420p",
                    vec![],
                );
            }
            if v.width.get().is_none_or(|v| *v <= 0 || v % 2 != 0)
                || v.height.get().is_none_or(|v| *v <= 0 || v % 2 != 0)
            {
                reject(
                    "dimensions_unsupported",
                    "保持尺寸的压缩需要已知偶数宽高",
                    vec![],
                );
            }
            if v.display_matrix.get().is_none_or(|m| !m.is_empty()) || v.rotate_tag.is_some() {
                reject("orientation_unavailable", "方向保留处理链尚未接通", vec![]);
            }
            if v.sample_aspect_ratio.get().is_none_or(|r| r.num != r.den) {
                reject(
                    "aspect_ratio_unavailable",
                    "当前处理链需要已确认的方形像素",
                    vec![],
                );
            }
            if v.color.primaries.get().is_some_and(|c| c == "bt2020")
                || v.color
                    .transfer
                    .get()
                    .is_some_and(|c| matches!(c.as_str(), "smpte2084" | "arib-std-b67"))
            {
                reject("hdr_unavailable", "HDR/BT.2020 保留处理链尚未接通", vec![]);
            }
        } else {
            reject(
                "needs_analysis",
                "缺少视频事实",
                vec![format!("tracks.{}.video", t.index)],
            );
        }
    }
    for t in audio {
        if t.codec.get().map(String::as_str) != Some("aac") {
            reject(
                "audio_codec_unsupported",
                "当前处理链仅复制 AAC 音轨",
                vec![],
            );
        }
    }
    result
}
