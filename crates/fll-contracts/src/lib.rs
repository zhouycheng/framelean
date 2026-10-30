//! Serializable media contracts. This crate has no native media or runtime dependencies.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const ABI_VERSION: u32 = 1;
pub const JSON_VERSION: u32 = 2;
pub const HTTP_VERSION: u32 = 3;
pub const FACTS_VERSION: u32 = 1;
pub const PLAN_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Knowledge {
    Known,
    Unknown,
    Estimated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fact<T> {
    pub state: Knowledge,
    pub value: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
impl<T> Fact<T> {
    pub fn known(value: T) -> Self {
        Self {
            state: Knowledge::Known,
            value: Some(value),
            reason: None,
        }
    }
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self {
            state: Knowledge::Unknown,
            value: None,
            reason: Some(reason.into()),
        }
    }
    pub fn get(&self) -> Option<&T> {
        if self.state == Knowledge::Known {
            self.value.as_ref()
        } else {
            None
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rational {
    pub num: i32,
    pub den: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceStamp {
    pub version: u32,
    pub identity: String,
    pub size: u64,
    pub modified_ns: String,
    pub sample_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColorFacts {
    pub primaries: Fact<String>,
    pub transfer: Fact<String>,
    pub space: Fact<String>,
    pub range: Fact<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VideoFacts {
    pub width: Fact<i32>,
    pub height: Fact<i32>,
    pub pixel_format: Fact<String>,
    pub bit_depth: Fact<i32>,
    pub sample_aspect_ratio: Fact<Rational>,
    pub frame_rate: Fact<Rational>,
    pub display_matrix: Fact<Vec<i32>>,
    pub rotate_tag: Option<String>,
    pub color: ColorFacts,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioFacts {
    pub sample_rate: Fact<i32>,
    pub channels: Fact<i32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrackFacts {
    pub index: u32,
    pub kind: String,
    pub codec: Fact<String>,
    pub profile: Fact<String>,
    pub time_base: Fact<Rational>,
    pub start_time_us: Fact<i64>,
    pub duration_us: Fact<i64>,
    pub video: Option<VideoFacts>,
    pub audio: Option<AudioFacts>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaFacts {
    pub schema_version: u32,
    pub analyzer_version: String,
    pub source_stamp: SourceStamp,
    pub read_status: String,
    pub container: String,
    pub duration_us: Fact<i64>,
    pub tracks: Vec<TrackFacts>,
    pub warnings: Vec<String>,
    pub facts_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Capabilities {
    pub json_version: u32,
    pub build: String,
    pub backend_versions: Value,
    pub compression_ready: bool,
    pub capability_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub preset: String,
    pub container: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub operation: String,
    pub preservation: String,
}
impl Default for Intent {
    fn default() -> Self {
        Self {
            operation: "compress".into(),
            preservation: "preserve".into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub required_facts: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Choices {
    pub context_digest: String,
    pub available_choices: Vec<Selection>,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamMap {
    pub input: u32,
    pub output: u32,
    pub mode: String,
    pub codec: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncodingParameters {
    pub encoder: String,
    pub crf: u8,
    pub preset: String,
    pub pixel_format: String,
    pub container: String,
    pub width: i32,
    pub height: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationRules {
    pub duration_tolerance_us: i64,
    pub av_offset_tolerance_us: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceHint {
    pub max_threads: u32,
    pub output_bytes: Option<u64>,
    pub work: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionPlan {
    pub schema_version: u32,
    pub planner_version: String,
    pub rules_version: String,
    pub facts_digest: String,
    pub source_stamp: SourceStamp,
    pub capability_digest: String,
    pub intent: Intent,
    pub selection: Selection,
    pub stream_map: Vec<StreamMap>,
    pub steps: Vec<String>,
    pub parameters: EncodingParameters,
    pub effects: Vec<String>,
    pub validation: ValidationRules,
    pub resource_hint: ResourceHint,
    pub plan_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceGrant {
    pub threads: u32,
}
impl Default for ResourceGrant {
    fn default() -> Self {
        Self { threads: 2 }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationCheck {
    pub rule: String,
    pub passed: bool,
    pub expected: Value,
    pub actual: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub checks: Vec<ValidationCheck>,
    pub passed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub media: MediaFacts,
    pub validation: ValidationReport,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaError {
    pub code: String,
    pub stage: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default)]
    pub details: Value,
}
impl MediaError {
    pub fn new(code: &str, stage: &str, message: impl ToString) -> Self {
        Self {
            code: code.into(),
            stage: stage.into(),
            message: message.to_string(),
            retryable: false,
            details: Value::Null,
        }
    }
    pub fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
}
impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for MediaError {}
pub type Result<T> = std::result::Result<T, MediaError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FllRequest {
    pub schema_version: u32,
    pub operation_id: String,
    pub operation: FllCommand,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FllCommand {
    Capabilities {},
    Analyze {
        input: String,
        #[serde(default)]
        cached: Option<MediaFacts>,
    },
    Options {
        media: MediaFacts,
        #[serde(default)]
        intent: Intent,
    },
    Plan {
        media: MediaFacts,
        #[serde(default)]
        intent: Intent,
        selection: Selection,
    },
    Execute {
        input: String,
        temp: String,
        media: MediaFacts,
        plan: ExecutionPlan,
        grant: ResourceGrant,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationEvent {
    pub operation_id: String,
    pub state: String,
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed_time_us: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<MediaError>,
}
impl OperationEvent {
    pub fn terminal(&self) -> bool {
        matches!(self.state.as_str(), "succeeded" | "failed" | "canceled")
    }
}
pub fn digest<T: Serialize>(value: &T) -> String {
    fn sorted(v: Value) -> Value {
        match v {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(k, v)| (k, sorted(v)))
                    .collect::<std::collections::BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.into_iter().map(sorted).collect()),
            other => other,
        }
    }
    let bytes = serde_json::to_vec(&sorted(
        serde_json::to_value(value).expect("serializable contract"),
    ))
    .expect("serializable JSON");
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_is_not_zero_and_digest_is_order_independent() {
        let fact: Fact<i32> = Fact::unknown("not reported");
        assert!(fact.get().is_none());
        assert_eq!(serde_json::to_value(fact).unwrap()["value"], Value::Null);
        let a: Value = serde_json::from_str(r#"{"b":2,"a":{"d":4,"c":3}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":{"c":3,"d":4},"b":2}"#).unwrap();
        assert_eq!(digest(&a), digest(&b));
    }
    #[test]
    fn unknown_operation_fields_are_rejected() {
        assert!(serde_json::from_value::<FllRequest>(serde_json::json!({"schema_version":2,"operation_id":"x","operation":{"kind":"capabilities","shell":"bad"}})).is_err());
    }
}
