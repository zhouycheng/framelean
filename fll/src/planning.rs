use crate::{adaptation, capabilities};
use fll_contracts::*;
use serde_json::json;
pub const PLANNER: &str = "compress-1";
pub(crate) fn default_selection() -> Selection {
    Selection {
        preset: "h264_crf23".into(),
        container: "mp4".into(),
    }
}
pub(crate) fn options(media: &MediaFacts, intent: &Intent) -> Result<Choices> {
    let mut check = media.clone();
    check.facts_digest.clear();
    if media.schema_version != FACTS_VERSION || digest(&check) != media.facts_digest {
        return Err(MediaError::new(
            "analysis_stale",
            "planning",
            "分析版本或摘要不匹配，请重新分析",
        ));
    }
    let caps = capabilities::snapshot();
    let mut diagnostics = adaptation::evaluate(media);
    if intent != &Intent::default() {
        diagnostics.push(Diagnostic {
            code: "intent_unsupported".into(),
            message: "当前仅支持保留媒体语义的视频压缩".into(),
            required_facts: vec![],
        });
    }
    if !caps.compression_ready {
        diagnostics.push(Diagnostic {
            code: "component_unavailable".into(),
            message: "当前构建的压缩链不可用".into(),
            required_facts: vec![],
        });
    }
    Ok(Choices {
        context_digest: digest(&(
            media.facts_digest.clone(),
            caps.capability_digest,
            PLANNER,
            adaptation::RULES_VERSION,
            intent,
        )),
        available_choices: if diagnostics.is_empty() {
            vec![default_selection()]
        } else {
            vec![]
        },
        diagnostics,
    })
}
pub(crate) fn plan(
    media: &MediaFacts,
    intent: &Intent,
    selection: &Selection,
) -> Result<ExecutionPlan> {
    let choices = options(media, intent)?;
    if !choices.available_choices.contains(selection) {
        return Err(
            MediaError::new("no_viable_plan", "planning", "该配置没有完整可执行方案")
                .details(json!(choices)),
        );
    }
    let video = media
        .tracks
        .iter()
        .find(|t| t.kind == "video")
        .ok_or_else(|| MediaError::new("no_video", "planning", "没有视频轨"))?;
    let v = video
        .video
        .as_ref()
        .ok_or_else(|| MediaError::new("no_video", "planning", "缺少视频事实"))?;
    let mut map = vec![StreamMap {
        input: video.index,
        output: 0,
        mode: "transcode".into(),
        codec: "h264".into(),
    }];
    if let Some(a) = media.tracks.iter().find(|t| t.kind == "audio") {
        map.push(StreamMap {
            input: a.index,
            output: 1,
            mode: "copy".into(),
            codec: "aac".into(),
        });
    }
    let mut p = ExecutionPlan {
        schema_version: PLAN_VERSION,
        planner_version: PLANNER.into(),
        rules_version: adaptation::RULES_VERSION.into(),
        facts_digest: media.facts_digest.clone(),
        source_stamp: media.source_stamp.clone(),
        capability_digest: capabilities::snapshot().capability_digest,
        intent: intent.clone(),
        selection: selection.clone(),
        stream_map: map,
        steps: vec![
            "demux".into(),
            "decode_video".into(),
            "encode_video".into(),
            "copy_audio_if_present".into(),
            "mux".into(),
        ],
        parameters: EncodingParameters {
            encoder: "libx264".into(),
            crf: 23,
            preset: "medium".into(),
            pixel_format: "yuv420p".into(),
            container: "mp4".into(),
            width: *v.width.get().unwrap(),
            height: *v.height.get().unwrap(),
        },
        effects: vec![
            "preserve_display_size".into(),
            "preserve_relative_timing".into(),
            "copy_audio".into(),
        ],
        validation: ValidationRules {
            duration_tolerance_us: 250_000,
            av_offset_tolerance_us: 50_000,
        },
        resource_hint: ResourceHint {
            max_threads: 2,
            output_bytes: None,
            work: "video_transcode".into(),
        },
        plan_digest: String::new(),
    };
    p.plan_digest = digest(&p);
    Ok(p)
}
