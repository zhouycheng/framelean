use schemars::Schema;

use crate::{
    AnalysisSnapshotView, AnalyzeMediaResponse, ExecutionSubmissionRequest,
    ExecutionSubmissionResult, RecalculateConfigurationResponse,
};

pub fn analyze_media_response_schema() -> Schema {
    schemars::schema_for!(AnalyzeMediaResponse)
}

pub fn recalculate_configuration_response_schema() -> Schema {
    schemars::schema_for!(RecalculateConfigurationResponse)
}

pub fn analysis_snapshot_view_schema() -> Schema {
    schemars::schema_for!(AnalysisSnapshotView)
}

pub fn execution_submission_request_schema() -> Schema {
    schemars::schema_for!(ExecutionSubmissionRequest)
}

pub fn execution_submission_result_schema() -> Schema {
    schemars::schema_for!(ExecutionSubmissionResult)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn aggregate_schema_is_owned_by_runtime() {
        let schema = analyze_media_response_schema().to_value();
        let properties = schema
            .pointer("/properties")
            .and_then(serde_json::Value::as_object)
            .unwrap();
        for field in [
            "media",
            "source_fingerprint",
            "requirements",
            "environment_summary",
            "engine_backend_summary",
            "capabilities",
            "configuration_options",
            "recommendation",
            "presets",
            "custom_target_size",
        ] {
            assert!(
                properties.contains_key(field),
                "missing schema field {field}"
            );
        }

        let recalculate = recalculate_configuration_response_schema().to_value();
        let properties = recalculate
            .pointer("/properties")
            .and_then(serde_json::Value::as_object)
            .unwrap();
        assert!(properties.contains_key("resolved_configuration"));
    }

    #[test]
    fn checked_in_schemas_match_runtime_dtos() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
        assert_schema_matches(
            root.join("analyze-media-response-v1.schema.json"),
            analyze_media_response_schema(),
        );
        assert_schema_matches(
            root.join("recalculate-configuration-response-v1.schema.json"),
            recalculate_configuration_response_schema(),
        );
        assert_schema_matches(
            root.join("analysis-snapshot-v1.schema.json"),
            analysis_snapshot_view_schema(),
        );
        assert_schema_matches(
            root.join("execution-submission-request-v1.schema.json"),
            execution_submission_request_schema(),
        );
        assert_schema_matches(
            root.join("execution-submission-result-v1.schema.json"),
            execution_submission_result_schema(),
        );
    }

    fn assert_schema_matches(path: PathBuf, expected: Schema) {
        let checked_in: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(checked_in, expected.to_value());
    }
}
