use fll_contracts::*;
use serde_json::json;
pub(crate) fn validate(
    input: &MediaFacts,
    output: &MediaFacts,
    plan: &ExecutionPlan,
) -> Result<ValidationReport> {
    let mut checks = Vec::new();
    let mut check =
        |rule: &str, passed: bool, expected: serde_json::Value, actual: serde_json::Value| {
            checks.push(ValidationCheck {
                rule: rule.into(),
                passed,
                expected,
                actual,
            })
        };
    check(
        "container",
        output
            .container
            .split(',')
            .any(|c| c == plan.parameters.container),
        json!(plan.parameters.container),
        json!(output.container),
    );
    check(
        "track_count",
        output.tracks.len() == plan.stream_map.len(),
        json!(plan.stream_map.len()),
        json!(output.tracks.len()),
    );
    let duration_ok = match (input.duration_us.get(), output.duration_us.get()) {
        (Some(a), Some(b)) => (*a - *b).abs() <= plan.validation.duration_tolerance_us,
        _ => false,
    };
    check(
        "duration",
        duration_ok,
        json!({"duration_us":input.duration_us,"tolerance_us":plan.validation.duration_tolerance_us}),
        json!(output.duration_us),
    );
    let input_video = input.tracks.iter().find(|t| t.kind == "video");
    let output_video = output.tracks.iter().find(|t| t.kind == "video");
    for m in &plan.stream_map {
        let a = input.tracks.iter().find(|t| t.index == m.input);
        let b = output.tracks.iter().find(|t| t.index == m.output);
        check(
            "stream_mapping",
            a.zip(b)
                .is_some_and(|(a, b)| a.kind == b.kind && b.codec.get() == Some(&m.codec)),
            json!(m),
            json!(b),
        );
        if let (Some(a), Some(b)) = (a, b) {
            if let (Some(av), Some(bv)) = (&a.video, &b.video) {
                check(
                    "display_size",
                    av.width == bv.width
                        && av.height == bv.height
                        && av.sample_aspect_ratio == bv.sample_aspect_ratio,
                    json!([av.width, av.height, av.sample_aspect_ratio]),
                    json!([bv.width, bv.height, bv.sample_aspect_ratio]),
                );
                check(
                    "pixel_format",
                    bv.pixel_format.get() == Some(&plan.parameters.pixel_format),
                    json!(plan.parameters.pixel_format),
                    json!(bv.pixel_format),
                );
                check(
                    "color_and_orientation",
                    av.color == bv.color
                        && av.display_matrix == bv.display_matrix
                        && av.rotate_tag == bv.rotate_tag,
                    json!([av.color, av.display_matrix, av.rotate_tag]),
                    json!([bv.color, bv.display_matrix, bv.rotate_tag]),
                );
            }
            if let (Some(aa), Some(ba)) = (&a.audio, &b.audio) {
                check("audio_parameters", aa == ba, json!(aa), json!(ba));
                let offset = |t: &TrackFacts, v: Option<&TrackFacts>| {
                    t.start_time_us
                        .get()
                        .zip(v.and_then(|v| v.start_time_us.get()))
                        .map(|(a, b)| a - b)
                };
                let ai = offset(a, input_video);
                let bi = offset(b, output_video);
                check(
                    "av_offset",
                    ai.zip(bi).is_some_and(|(a, b)| {
                        (a - b).abs() <= plan.validation.av_offset_tolerance_us
                    }),
                    json!({"offset_us":ai,"tolerance_us":plan.validation.av_offset_tolerance_us}),
                    json!(bi),
                );
            }
        }
    }
    let report = ValidationReport {
        passed: checks.iter().all(|c| c.passed),
        checks,
    };
    if !report.passed {
        return Err(
            MediaError::new("validation_failed", "validating", "输出没有满足处理方案")
                .details(json!(report)),
        );
    }
    Ok(report)
}
