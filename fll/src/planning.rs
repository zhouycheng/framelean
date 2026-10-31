use crate::{adaptation, capabilities};
use fll_contracts::*;
use serde_json::json;

pub const PLANNER: &str = "video-2";
fn preset(id: &str) -> Option<(u8, &str)> {
    match id {
        "quality" => Some((18, "画质优先")),
        "balanced" | "h264_crf23" => Some((23, "均衡")),
        "small" => Some((28, "体积优先")),
        _ => None,
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
    if !matches!(intent.operation.as_str(), "compress" | "convert")
        || intent.preservation != "preserve"
    {
        diagnostics.push(Diagnostic {
            code: "intent_unsupported".into(),
            message: "当前支持保留媒体语义的压缩和转换".into(),
            required_facts: vec![],
        });
    }
    let codec = media
        .tracks
        .iter()
        .find(|t| t.kind == "video")
        .and_then(|t| t.codec.get());
    let remux = intent.operation == "convert" && codec.is_some_and(|c| c == "h264");
    if !codec.is_some_and(|c| caps.video_decoders.contains(c))
        || (!remux && !caps.h264_encoder)
        || caps.output_containers.is_empty()
    {
        diagnostics.push(Diagnostic {
            code: "component_unavailable".into(),
            message: "当前构建缺少所需解码、编码或封装能力".into(),
            required_facts: vec![],
        });
    }
    let presets = if intent.operation == "convert" {
        vec![Preset {
            id: "auto".into(),
            label: "通用 H.264".into(),
            aliases: vec![],
        }]
    } else {
        ["balanced", "quality", "small"]
            .into_iter()
            .map(|id| Preset {
                id: id.into(),
                label: preset(id).unwrap().1.into(),
                aliases: if id == "balanced" {
                    vec!["h264_crf23".into()]
                } else {
                    vec![]
                },
            })
            .collect()
    };
    let mut available_choices = vec![];
    if diagnostics.is_empty() {
        for container in &caps.output_containers {
            if let Some(reason) = adaptation::output_diagnostic(media, container) {
                diagnostics.push(reason);
                continue;
            }
            for p in &presets {
                available_choices.push(Selection {
                    preset: p.id.clone(),
                    container: container.clone(),
                });
            }
        }
    }
    Ok(Choices {
        context_digest: digest(&(
            media.facts_digest.clone(),
            caps.capability_digest,
            PLANNER,
            adaptation::RULES_VERSION,
            intent,
        )),
        default_selection: available_choices.first().cloned(),
        available_choices,
        presets,
        diagnostics,
    })
}
pub(crate) fn plan(
    media: &MediaFacts,
    intent: &Intent,
    selection: &Selection,
) -> Result<ExecutionPlan> {
    let selection = Selection {
        preset: if selection.preset == "h264_crf23" {
            "balanced".into()
        } else {
            selection.preset.clone()
        },
        container: selection.container.clone(),
    };
    let choices = options(media, intent)?;
    if !choices.available_choices.contains(&selection) {
        return Err(
            MediaError::new("no_viable_plan", "planning", "该配置没有完整可执行方案")
                .details(json!(choices)),
        );
    }
    let video = media.tracks.iter().find(|t| t.kind == "video").unwrap();
    let v = video.video.as_ref().unwrap();
    let remux = intent.operation == "convert" && video.codec.get().is_some_and(|c| c == "h264");
    let mut map = vec![StreamMap {
        input: video.index,
        output: 0,
        mode: if remux { "copy" } else { "transcode" }.into(),
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
    let output = OutputSpec {
        muxer: if selection.container == "mkv" {
            "matroska"
        } else {
            "mp4"
        }
        .into(),
        extension: selection.container.clone(),
        container: selection.container.clone(),
    };
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
        steps: if remux {
            vec!["demux", "copy_streams", "mux"]
        } else {
            vec![
                "demux",
                "decode_video",
                "encode_video",
                "copy_audio_if_present",
                "mux",
            ]
        }
        .into_iter()
        .map(str::to_owned)
        .collect(),
        parameters: if remux {
            ProcessingParameters::Remux {}
        } else {
            ProcessingParameters::Transcode {
                video: EncodingParameters {
                    encoder: "libx264".into(),
                    crf: preset(if selection.preset == "auto" {
                        "balanced"
                    } else {
                        &selection.preset
                    })
                    .unwrap()
                    .0,
                    preset: "medium".into(),
                    pixel_format: "yuv420p".into(),
                    width: *v.width.get().unwrap(),
                    height: *v.height.get().unwrap(),
                },
            }
        },
        output,
        effects: vec![
            "preserve_display_size",
            "preserve_relative_timing",
            "copy_audio",
            if remux {
                "copy_video"
            } else {
                "reencode_video"
            },
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        validation: ValidationRules {
            duration_tolerance_us: 250_000,
            av_offset_tolerance_us: 50_000,
        },
        resource_hint: ResourceHint {
            max_threads: 2,
            output_bytes: None,
            work: if remux { "remux" } else { "video_transcode" }.into(),
        },
        plan_digest: String::new(),
    };
    p.plan_digest = digest(&p);
    Ok(p)
}
