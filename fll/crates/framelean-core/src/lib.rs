use std::error::Error;
use std::fmt::{self, Display, Formatter};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidIdentifier,
    InvalidTaskState,
    InvalidArgument,
    Media,
    Processor,
    Pipeline,
    Plugin,
    Runtime,
    Analysis,
    Environment,
    NativeLibrary,
    Capability,
    Recommendation,
    Estimation,
    Snapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EngineErrorCode {
    InvalidIdentifier,
    InvalidTaskState,
    InvalidArgument,
    InternalMediaError,
    InternalProcessorError,
    InternalPipelineError,
    InternalPluginError,
    InternalRuntimeError,
    InternalAnalysisError,
    InternalEnvironmentError,
    InternalNativeLibraryError,
    InternalCapabilityError,
    InternalRecommendationError,
    InternalEstimationError,
    InternalSnapshotError,
    MediaFileNotFound,
    MediaPermissionDenied,
    MediaInvalidFormat,
    MediaInfoReadFailed,
    NativeLibraryUnavailable,
    AnalysisSourceChanged,
    AnalysisSnapshotExpired,
    AnalysisRevisionConflict,
    QueueRevisionConflict,
    EngineExecutionChainNotReady,
    MediaCapabilityIncompatible,
    PresetNotApplicable,
    TargetSizeUnachievable,
    OutputContainerNotWritable,
    MediaDurationUnavailable,
    MediaStreamUnrecognized,
    MediaProfileUnavailable,
    MediaPixelFormatUnavailable,
    MediaBitDepthUnavailable,
    MediaHdrStateIncomplete,
    MediaAnimationStateNotProbed,
}

impl EngineErrorCode {
    fn for_kind(kind: ErrorKind) -> Self {
        match kind {
            ErrorKind::InvalidIdentifier => Self::InvalidIdentifier,
            ErrorKind::InvalidTaskState => Self::InvalidTaskState,
            ErrorKind::InvalidArgument => Self::InvalidArgument,
            ErrorKind::Media => Self::InternalMediaError,
            ErrorKind::Processor => Self::InternalProcessorError,
            ErrorKind::Pipeline => Self::InternalPipelineError,
            ErrorKind::Plugin => Self::InternalPluginError,
            ErrorKind::Runtime => Self::InternalRuntimeError,
            ErrorKind::Analysis => Self::InternalAnalysisError,
            ErrorKind::Environment => Self::InternalEnvironmentError,
            ErrorKind::NativeLibrary => Self::InternalNativeLibraryError,
            ErrorKind::Capability => Self::InternalCapabilityError,
            ErrorKind::Recommendation => Self::InternalRecommendationError,
            ErrorKind::Estimation => Self::InternalEstimationError,
            ErrorKind::Snapshot => Self::InternalSnapshotError,
        }
    }

    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::MediaInfoReadFailed
                | Self::AnalysisRevisionConflict
                | Self::QueueRevisionConflict
        )
    }
}

impl Display for ErrorKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::InvalidIdentifier => "invalid identifier",
            Self::InvalidTaskState => "invalid task state",
            Self::InvalidArgument => "invalid argument",
            Self::Media => "media",
            Self::Processor => "processor",
            Self::Pipeline => "pipeline",
            Self::Plugin => "plugin",
            Self::Runtime => "runtime",
            Self::Analysis => "analysis",
            Self::Environment => "environment",
            Self::NativeLibrary => "native library",
            Self::Capability => "capability",
            Self::Recommendation => "recommendation",
            Self::Estimation => "estimation",
            Self::Snapshot => "snapshot",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug)]
pub struct EngineError {
    kind: ErrorKind,
    code: EngineErrorCode,
    message: String,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl EngineError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: EngineErrorCode::for_kind(kind),
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source<E>(kind: ErrorKind, message: impl Into<String>, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            kind,
            code: EngineErrorCode::for_kind(kind),
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn with_code(kind: ErrorKind, code: EngineErrorCode, message: impl Into<String>) -> Self {
        Self {
            kind,
            code,
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source_code<E>(
        kind: ErrorKind,
        code: EngineErrorCode,
        message: impl Into<String>,
        source: E,
    ) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            kind,
            code,
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn invalid_identifier(kind: &str) -> Self {
        Self::new(
            ErrorKind::InvalidIdentifier,
            format!("{kind} identifier cannot be empty"),
        )
    }

    pub fn invalid_task_state(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidTaskState, message)
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidArgument, message)
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn code(&self) -> EngineErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for EngineError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.kind, self.message)
    }
}

impl Error for EngineError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source.as_ref() as &(dyn Error + 'static))
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

macro_rules! define_id {
    ($name:ident, $label:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(EngineError::invalid_identifier($label));
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

define_id!(TaskId, "task");
define_id!(NodeId, "node");
define_id!(ProcessorId, "processor");
define_id!(AnalysisId, "analysis");
define_id!(BackendId, "backend");

impl AnalysisId {
    pub fn generate() -> Self {
        Self(format!("analysis-{}", uuid::Uuid::new_v4()))
    }
}

macro_rules! define_u64_unit {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn value(self) -> u64 {
                self.0
            }
        }
    };
}

define_u64_unit!(FileSizeBytes);
define_u64_unit!(MemoryBytes);
define_u64_unit!(BitRateBps);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    Detected,
    Absent,
    Unavailable,
    NotProbed,
    Unsupported,
    Failed,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Observed<T> {
    pub status: ObservationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl<T> Observed<T> {
    pub fn detected(value: T, source: impl Into<String>) -> Self {
        Self {
            status: ObservationStatus::Detected,
            value: Some(value),
            source: Some(source.into()),
            reason: None,
        }
    }

    pub fn with_status(status: ObservationStatus, reason: impl Into<String>) -> Self {
        Self {
            status,
            value: None,
            source: None,
            reason: Some(reason.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn identifiers_reject_empty_values() {
        let error = ProcessorId::new("  ").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidIdentifier);
        assert_eq!(error.code(), EngineErrorCode::InvalidIdentifier);
        assert!(TaskId::new("").is_err());
        assert!(NodeId::new("\n\t").is_err());
    }

    #[test]
    fn structured_error_code_is_independent_from_message() {
        let error = EngineError::with_code(
            ErrorKind::Analysis,
            EngineErrorCode::MediaInvalidFormat,
            "localized display text",
        );

        assert_eq!(error.code(), EngineErrorCode::MediaInvalidFormat);
        assert_eq!(error.message(), "localized display text");
    }

    #[test]
    fn retryability_is_derived_from_stable_error_code() {
        assert!(EngineErrorCode::MediaInfoReadFailed.is_retryable());
        assert!(EngineErrorCode::AnalysisRevisionConflict.is_retryable());
        assert!(!EngineErrorCode::MediaInvalidFormat.is_retryable());
        assert!(!EngineErrorCode::AnalysisSourceChanged.is_retryable());
    }

    #[test]
    fn identifiers_are_displayable_hash_map_keys() {
        let id = ProcessorId::new("processor.example").unwrap();
        let mut values = HashMap::new();
        values.insert(id.clone(), 1);

        assert_eq!(id.to_string(), "processor.example");
        assert_eq!(values.get(&id), Some(&1));
    }
}
