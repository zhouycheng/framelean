use std::collections::HashSet;

use framelean_core::{BackendId, EngineError, Observed, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", content = "values", rename_all = "snake_case")]
pub enum CapabilityConstraint<T> {
    Unknown,
    Unrestricted,
    Restricted(Vec<T>),
    Unsupported,
}

impl<T: PartialEq> CapabilityConstraint<T> {
    pub fn allows(&self, value: &T) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::Restricted(values) => values.contains(value),
            Self::Unknown | Self::Unsupported => false,
        }
    }

    pub fn values(&self) -> Option<&[T]> {
        match self {
            Self::Restricted(values) => Some(values),
            _ => None,
        }
    }

    pub fn validate(&self, field: &str) -> Result<()> {
        if matches!(self, Self::Restricted(values) if values.is_empty()) {
            return Err(EngineError::invalid_argument(format!(
                "restricted capability {field} cannot be empty"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Demuxer,
    Decoder,
    Processor,
    Encoder,
    Muxer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    Data,
    Attachment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HdrMode {
    Sdr,
    Hdr10,
    Hlg,
    DolbyVision,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HdrOperation {
    Preserve,
    ToneMapToSdr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeSupportStatus {
    NotProbed,
    NativeNotRequired,
    NativeDiscovered,
    NativeInitializable,
    NativeUnavailable,
    NativeInitializationFailed,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EngineRegistrationStatus {
    NotRegistered,
    EngineRegistered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EngineExecutionReadiness {
    NotReady,
    EngineExecutionReady,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackendAvailability {
    pub native_support: NativeSupportStatus,
    pub engine_registration: EngineRegistrationStatus,
    pub engine_execution_readiness: EngineExecutionReadiness,
    pub reason: Option<String>,
}

impl BackendAvailability {
    pub fn native_only(native_support: NativeSupportStatus) -> Self {
        Self {
            native_support,
            engine_registration: EngineRegistrationStatus::NotRegistered,
            engine_execution_readiness: EngineExecutionReadiness::NotReady,
            reason: Some("ENGINE_BACKEND_NOT_REGISTERED".to_owned()),
        }
    }

    pub fn execution_ready(native_support: NativeSupportStatus) -> Self {
        Self {
            native_support,
            engine_registration: EngineRegistrationStatus::EngineRegistered,
            engine_execution_readiness: EngineExecutionReadiness::EngineExecutionReady,
            reason: None,
        }
    }

    pub fn is_execution_ready(&self) -> bool {
        matches!(
            self.native_support,
            NativeSupportStatus::NativeInitializable | NativeSupportStatus::NativeNotRequired
        ) && self.engine_registration == EngineRegistrationStatus::EngineRegistered
            && self.engine_execution_readiness == EngineExecutionReadiness::EngineExecutionReady
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DemuxerCapability {
    pub input_formats: Vec<String>,
    pub stream_types: Vec<StreamKind>,
    pub codec_restrictions: CapabilityConstraint<String>,
    pub supports_multiple_streams: Observed<bool>,
    pub requires_seek: Observed<bool>,
    pub supports_custom_io: Observed<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecoderCapability {
    pub stream_type: StreamKind,
    pub codecs: Vec<String>,
    pub profiles: CapabilityConstraint<String>,
    pub pixel_or_sample_formats: CapabilityConstraint<String>,
    pub bit_depths: CapabilityConstraint<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProcessorCapability {
    pub stream_type: StreamKind,
    pub input_formats: CapabilityConstraint<String>,
    pub output_formats: CapabilityConstraint<String>,
    pub bit_depths: CapabilityConstraint<u8>,
    pub hdr_operations: CapabilityConstraint<HdrOperation>,
    pub operations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EncoderCapability {
    pub stream_type: StreamKind,
    pub codecs: Vec<String>,
    pub profiles: CapabilityConstraint<String>,
    pub pixel_or_sample_formats: CapabilityConstraint<String>,
    pub bit_depths: CapabilityConstraint<u8>,
    pub hdr_modes: CapabilityConstraint<HdrMode>,
    pub rate_control_modes: CapabilityConstraint<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct MuxerCodecCombination {
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MuxerCapability {
    pub output_formats: Vec<String>,
    pub video_codecs: Vec<String>,
    pub audio_codecs: Vec<String>,
    pub supports_subtitles: Observed<bool>,
    pub supports_data: Observed<bool>,
    pub supports_attachments: Observed<bool>,
    pub supports_multiple_streams: Observed<bool>,
    pub codec_combinations: CapabilityConstraint<MuxerCodecCombination>,
    pub requires_seek: Observed<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackendEnvironmentRequirements {
    pub operating_systems: CapabilityConstraint<String>,
    pub architectures: CapabilityConstraint<String>,
    pub requires_gpu: CapabilityConstraint<bool>,
    pub native_frameworks: CapabilityConstraint<String>,
}

impl BackendEnvironmentRequirements {
    pub fn unrestricted() -> Self {
        Self {
            operating_systems: CapabilityConstraint::Unrestricted,
            architectures: CapabilityConstraint::Unrestricted,
            requires_gpu: CapabilityConstraint::Unrestricted,
            native_frameworks: CapabilityConstraint::Unrestricted,
        }
    }

    fn validate(&self) -> Result<()> {
        self.operating_systems.validate("operating_systems")?;
        self.architectures.validate("architectures")?;
        self.requires_gpu.validate("requires_gpu")?;
        self.native_frameworks.validate("native_frameworks")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "capability", rename_all = "snake_case")]
pub enum BackendCapability {
    Demuxer(DemuxerCapability),
    Decoder(DecoderCapability),
    Processor(ProcessorCapability),
    Encoder(EncoderCapability),
    Muxer(MuxerCapability),
}

impl BackendCapability {
    pub fn kind(&self) -> BackendKind {
        match self {
            Self::Demuxer(_) => BackendKind::Demuxer,
            Self::Decoder(_) => BackendKind::Decoder,
            Self::Processor(_) => BackendKind::Processor,
            Self::Encoder(_) => BackendKind::Encoder,
            Self::Muxer(_) => BackendKind::Muxer,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackendDescriptor {
    pub id: BackendId,
    pub provider: String,
    pub version: Option<String>,
    pub availability: BackendAvailability,
    pub environment: BackendEnvironmentRequirements,
    pub capability: BackendCapability,
    pub source: String,
}

impl BackendDescriptor {
    pub fn validate(&self) -> Result<()> {
        if self.provider.trim().is_empty() {
            return Err(EngineError::invalid_argument(
                "backend provider cannot be empty",
            ));
        }
        self.environment.validate()?;
        match &self.capability {
            BackendCapability::Demuxer(value) => {
                value.codec_restrictions.validate("codec_restrictions")?;
            }
            BackendCapability::Decoder(value) => {
                value.profiles.validate("profiles")?;
                value
                    .pixel_or_sample_formats
                    .validate("pixel_or_sample_formats")?;
                value.bit_depths.validate("bit_depths")?;
            }
            BackendCapability::Processor(value) => {
                value.input_formats.validate("input_formats")?;
                value.output_formats.validate("output_formats")?;
                value.bit_depths.validate("bit_depths")?;
                value.hdr_operations.validate("hdr_operations")?;
            }
            BackendCapability::Encoder(value) => {
                value.profiles.validate("profiles")?;
                value
                    .pixel_or_sample_formats
                    .validate("pixel_or_sample_formats")?;
                value.bit_depths.validate("bit_depths")?;
                value.hdr_modes.validate("hdr_modes")?;
                value.rate_control_modes.validate("rate_control_modes")?;
            }
            BackendCapability::Muxer(value) => {
                value.codec_combinations.validate("codec_combinations")?;
            }
        }
        if self.capability.kind() == BackendKind::Processor
            && matches!(self.capability, BackendCapability::Processor(ref value) if value.operations.is_empty())
        {
            return Err(EngineError::invalid_argument(
                "processor capability must declare at least one operation",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackendCatalog {
    pub revision: u64,
    pub backends: Vec<BackendDescriptor>,
}

impl BackendCatalog {
    pub fn from_backends(backends: Vec<BackendDescriptor>) -> Result<Self> {
        let mut catalog = Self {
            revision: 0,
            backends,
        };
        catalog.validate()?;
        catalog.revision = stable_catalog_revision(&catalog.backends)?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        for backend in &self.backends {
            backend.validate()?;
            if !ids.insert(backend.id.clone()) {
                return Err(EngineError::invalid_argument(format!(
                    "duplicate backend id {}",
                    backend.id
                )));
            }
        }
        Ok(())
    }

    pub fn execution_ready(&self, kind: BackendKind) -> impl Iterator<Item = &BackendDescriptor> {
        self.backends.iter().filter(move |backend| {
            backend.capability.kind() == kind && backend.availability.is_execution_ready()
        })
    }
}

#[derive(Serialize)]
struct StableBackendDescriptor<'a> {
    id: &'a BackendId,
    provider: &'a str,
    version: &'a Option<String>,
    availability: &'a BackendAvailability,
    environment: &'a BackendEnvironmentRequirements,
    capability: &'a BackendCapability,
}

fn stable_catalog_revision(backends: &[BackendDescriptor]) -> Result<u64> {
    let mut sorted: Vec<_> = backends.iter().collect();
    sorted.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    let stable: Vec<_> = sorted
        .into_iter()
        .map(|backend| StableBackendDescriptor {
            id: &backend.id,
            provider: &backend.provider,
            version: &backend.version,
            availability: &backend.availability,
            environment: &backend.environment,
            capability: &backend.capability,
        })
        .collect();
    let bytes = serde_json::to_vec(&stable).map_err(|error| {
        EngineError::with_source(
            framelean_core::ErrorKind::Capability,
            "failed to serialize stable backend catalog",
            error,
        )
    })?;
    let digest = blake3::hash(&bytes);
    Ok(u64::from_le_bytes(
        digest.as_bytes()[..8]
            .try_into()
            .expect("BLAKE3 digest contains eight bytes"),
    ))
}

pub trait BackendCatalogProvider: Send + Sync {
    fn backend_catalog(&self) -> Result<BackendCatalog>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_only_backend_is_not_execution_ready() {
        let availability =
            BackendAvailability::native_only(NativeSupportStatus::NativeInitializable);
        assert!(!availability.is_execution_ready());
    }

    #[test]
    fn native_discovery_cannot_be_marked_execution_ready() {
        let availability =
            BackendAvailability::execution_ready(NativeSupportStatus::NativeDiscovered);
        assert!(!availability.is_execution_ready());
    }

    #[test]
    fn backend_id_comes_from_core() {
        let id = BackendId::new("ffmpeg.demuxer.mov").unwrap();
        assert_eq!(id.as_str(), "ffmpeg.demuxer.mov");
    }

    #[test]
    fn restricted_constraint_rejects_an_empty_value_set() {
        let constraint = CapabilityConstraint::<String>::Restricted(Vec::new());
        assert!(constraint.validate("profiles").is_err());
        assert!(!CapabilityConstraint::<String>::Unknown.allows(&"main".to_owned()));
        assert!(CapabilityConstraint::<String>::Unrestricted.allows(&"main".to_owned()));
    }

    #[test]
    fn catalog_revision_changes_with_stable_capability_content() {
        let mut backend = test_demuxer("revision");
        let first = BackendCatalog::from_backends(vec![backend.clone()]).unwrap();
        let BackendCapability::Demuxer(capability) = &mut backend.capability else {
            unreachable!();
        };
        capability.input_formats.push("other".to_owned());
        let second = BackendCatalog::from_backends(vec![backend]).unwrap();

        assert_ne!(first.revision, second.revision);
    }

    #[test]
    fn catalog_rejects_duplicate_backend_ids() {
        let backend = test_demuxer("duplicate");
        let catalog = BackendCatalog {
            revision: 1,
            backends: vec![backend.clone(), backend],
        };
        assert!(catalog.validate().is_err());
    }

    fn test_demuxer(id: &str) -> BackendDescriptor {
        BackendDescriptor {
            id: BackendId::new(id).unwrap(),
            provider: "test".to_owned(),
            version: None,
            availability: BackendAvailability::native_only(NativeSupportStatus::NativeDiscovered),
            capability: BackendCapability::Demuxer(DemuxerCapability {
                input_formats: vec!["test".to_owned()],
                stream_types: vec![StreamKind::Video],
                codec_restrictions: CapabilityConstraint::Unrestricted,
                supports_multiple_streams: Observed::detected(true, "test"),
                requires_seek: Observed::detected(false, "test"),
                supports_custom_io: Observed::detected(true, "test"),
            }),
            source: "test".to_owned(),
            environment: BackendEnvironmentRequirements::unrestricted(),
        }
    }
}
