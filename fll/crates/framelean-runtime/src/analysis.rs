use std::sync::Arc;
use std::time::Instant;

use framelean_analysis::{
    MediaAnalysis, MediaAnalysisStatus, MediaAnalyzeRequest, MediaAnalyzer, PlatformFileId,
    SourceFingerprint, SourceId,
};
use framelean_core::{AnalysisId, EngineError, EngineErrorCode, Result};
use framelean_decision::{
    CapabilityResolver, CapabilitySet, ConfigurationOptionGraph, ConfigurationStatus,
    CustomTargetSizeOptions, EstimatorPolicy, InputMediaRequirements, PresetDefinition,
    RecalculateSelection, Recommendation, RecommendationEngine, ResolvedConfiguration,
    SizeEstimator, TaskMode,
};
use framelean_environment::{
    EnvironmentSnapshot, EnvironmentSnapshotProvider, ResourceMonitor, ResourceSample,
};
use framelean_media::capability::{BackendCatalog, BackendCatalogProvider};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::snapshot::{
    AnalysisRevision, AnalysisSnapshot, CURRENT_DECISION_MODEL_REVISION,
    CURRENT_ESTIMATOR_MODEL_REVISION,
};

pub struct AnalysisServices {
    pub analyzer: Arc<dyn MediaAnalyzer>,
    pub environment: Arc<dyn EnvironmentSnapshotProvider>,
    pub resource_monitor: Arc<dyn ResourceMonitor>,
    pub native_backend_providers: Vec<Arc<dyn BackendCatalogProvider>>,
    pub capability_resolver: Arc<dyn CapabilityResolver>,
    pub recommendation_engine: Arc<dyn RecommendationEngine>,
    pub size_estimator: Arc<dyn SizeEstimator>,
    pub estimator_policy: EstimatorPolicy,
}

#[derive(Debug, Clone)]
pub struct AnalyzeTaskRequest {
    pub task_mode: TaskMode,
    pub media_request: MediaAnalyzeRequest,
    pub context: RequestContext,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RequestContext {
    pub request_id: Option<String>,
    pub client_file_id: Option<String>,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvironmentSummary {
    pub operating_system: framelean_core::Observed<String>,
    pub architecture: String,
    pub logical_cores: u32,
    pub total_memory_bytes: u64,
    pub gpu_count: Option<usize>,
    pub sampled_at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EngineBackendSummary {
    pub catalog_revision: u64,
    pub backend_count: usize,
    pub native_backend_count: usize,
    pub plugin_backend_count: usize,
    pub registered_backend_count: usize,
    pub execution_ready_backend_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorDetail {
    pub code: EngineErrorCode,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WarningDetail {
    pub code: EngineErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AnalyzeMediaResponse {
    pub schema_version: String,
    pub analysis_id: AnalysisId,
    pub analysis_revision: AnalysisRevision,
    pub task_mode: TaskMode,
    pub media_analysis_status: MediaAnalysisStatus,
    pub configuration_status: ConfigurationStatus,
    pub media: Option<MediaAnalysis>,
    pub source_fingerprint: Option<SourceFingerprintSummary>,
    pub requirements: Option<InputMediaRequirements>,
    pub environment_summary: Option<EnvironmentSummary>,
    pub engine_backend_summary: Option<EngineBackendSummary>,
    pub capabilities: Option<CapabilitySet>,
    pub configuration_options: Option<ConfigurationOptionGraph>,
    pub recommendation: Option<Recommendation>,
    pub presets: Vec<framelean_decision::PresetDefinition>,
    pub custom_target_size: Option<CustomTargetSizeOptions>,
    pub warnings: Vec<WarningDetail>,
    pub error: Option<ErrorDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceFingerprintSummary {
    pub source_id: SourceId,
    pub canonical_path: String,
    pub canonicalization_status: framelean_core::ObservationStatus,
    pub canonicalization_reason: Option<String>,
    pub file_size_bytes: u64,
    pub modified_time_unix_nanos: Option<String>,
    pub platform_file_id: Option<String>,
    pub quick_content_hash_hex: String,
}

impl SourceFingerprintSummary {
    fn from_fingerprint(fingerprint: &SourceFingerprint, source_id: &SourceId) -> Self {
        let platform_file_id = fingerprint.platform_file_id().map(|value| match value {
            PlatformFileId::Unix { device, inode } => format!("unix:{device}:{inode}"),
            PlatformFileId::Windows {
                volume_serial,
                file_index,
            } => format!("windows:{volume_serial}:{file_index}"),
        });
        let quick_content_hash_hex = fingerprint
            .quick_content_hash()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            source_id: source_id.clone(),
            canonical_path: fingerprint.canonical_path().to_string_lossy().into_owned(),
            canonicalization_status: fingerprint.canonicalization_status(),
            canonicalization_reason: fingerprint.canonicalization_reason().map(str::to_owned),
            file_size_bytes: fingerprint.size_bytes(),
            modified_time_unix_nanos: fingerprint
                .modified_time_unix_nanos()
                .map(|value| value.to_string()),
            platform_file_id,
            quick_content_hash_hex,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AnalysisSnapshotView {
    pub schema_version: String,
    pub analysis_id: AnalysisId,
    pub analysis_revision: AnalysisRevision,
    pub decision_model_revision: u32,
    pub estimator_model_revision: u32,
    pub task_mode: TaskMode,
    pub media: MediaAnalysis,
    pub source_fingerprint: SourceFingerprintSummary,
    pub requirements: InputMediaRequirements,
    pub environment_summary: EnvironmentSummary,
    pub engine_backend_summary: EngineBackendSummary,
    pub capabilities: CapabilitySet,
    pub configuration_options: ConfigurationOptionGraph,
    pub recommendation: Recommendation,
    pub presets: Vec<framelean_decision::PresetDefinition>,
    pub custom_target_size: CustomTargetSizeOptions,
    pub warnings: Vec<WarningDetail>,
    pub validity: AnalysisSnapshotValidity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisSnapshotValidityStatus {
    Valid,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AnalysisSnapshotValidity {
    pub status: AnalysisSnapshotValidityStatus,
    pub reason_code: Option<EngineErrorCode>,
    pub message: Option<String>,
}

impl AnalysisSnapshotValidity {
    pub(crate) fn valid() -> Self {
        Self {
            status: AnalysisSnapshotValidityStatus::Valid,
            reason_code: None,
            message: None,
        }
    }

    pub(crate) fn source_changed(message: impl Into<String>) -> Self {
        Self {
            status: AnalysisSnapshotValidityStatus::Invalid,
            reason_code: Some(EngineErrorCode::AnalysisSourceChanged),
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecalculateConfigurationRequest {
    pub analysis_id: AnalysisId,
    pub expected_revision: AnalysisRevision,
    pub selection: RecalculateSelection,
    pub context: RequestContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigurationConflict {
    pub code: EngineErrorCode,
    pub field: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecalculateConfigurationResponse {
    pub schema_version: String,
    pub analysis_id: AnalysisId,
    pub analysis_revision: AnalysisRevision,
    pub configuration_status: ConfigurationStatus,
    pub capabilities: CapabilitySet,
    pub configuration_options: ConfigurationOptionGraph,
    pub recommendation: Recommendation,
    pub presets: Vec<framelean_decision::PresetDefinition>,
    pub custom_target_size: CustomTargetSizeOptions,
    pub selection: RecalculateSelection,
    pub resolved_configuration: Option<ResolvedConfiguration>,
    pub conflicts: Vec<ConfigurationConflict>,
    pub warnings: Vec<WarningDetail>,
    pub error: Option<ErrorDetail>,
}

pub(crate) struct CollectedCatalog {
    pub catalog: BackendCatalog,
    pub native_backend_count: usize,
    pub plugin_backend_count: usize,
}

pub(crate) fn collect_catalog(
    native_providers: &[Arc<dyn BackendCatalogProvider>],
    plugin_catalog: BackendCatalog,
) -> Result<CollectedCatalog> {
    let plugin_backend_count = plugin_catalog.backends.len();
    let mut backends = plugin_catalog.backends;
    let mut native_backend_count = 0;
    for provider in native_providers {
        let catalog = provider.backend_catalog()?;
        native_backend_count += catalog.backends.len();
        backends.extend(catalog.backends);
    }
    Ok(CollectedCatalog {
        catalog: BackendCatalog::from_backends(backends)?,
        native_backend_count,
        plugin_backend_count,
    })
}

pub(crate) struct AnalysisAssembly {
    pub media: MediaAnalysis,
    pub requirements: InputMediaRequirements,
    pub source_fingerprint: SourceFingerprint,
    pub environment: EnvironmentSnapshot,
    pub resource_sample: Option<ResourceSample>,
    pub catalog: BackendCatalog,
    pub native_backend_count: usize,
    pub plugin_backend_count: usize,
    pub capabilities: CapabilitySet,
    pub configuration_options: ConfigurationOptionGraph,
    pub recommendation: Recommendation,
    pub presets: Vec<PresetDefinition>,
    pub custom_target_size: CustomTargetSizeOptions,
    pub estimator_policy: EstimatorPolicy,
}

pub(crate) fn build_success(
    id: AnalysisId,
    request: &AnalyzeTaskRequest,
    assembly: AnalysisAssembly,
) -> (AnalyzeMediaResponse, AnalysisSnapshot) {
    let AnalysisAssembly {
        media,
        requirements,
        source_fingerprint,
        environment,
        resource_sample,
        catalog,
        native_backend_count,
        plugin_backend_count,
        capabilities,
        configuration_options,
        recommendation,
        presets,
        custom_target_size,
        estimator_policy,
    } = assembly;
    let media_analysis_status = media.status;
    let revision = AnalysisRevision::initial();
    let configuration_status = if capabilities.available {
        ConfigurationStatus::Available
    } else {
        ConfigurationStatus::Unavailable
    };
    let environment_summary = environment_summary(&environment, resource_sample.as_ref());
    let backend_summary = backend_summary(&catalog, native_backend_count, plugin_backend_count);
    let warnings = response_warnings(&media, &capabilities);
    let source_fingerprint_summary =
        SourceFingerprintSummary::from_fingerprint(&source_fingerprint, &media.source_id);
    let response = AnalyzeMediaResponse {
        schema_version: "1.0".to_owned(),
        analysis_id: id.clone(),
        analysis_revision: revision,
        task_mode: request.task_mode,
        media_analysis_status,
        configuration_status,
        media: Some(media.clone()),
        source_fingerprint: Some(source_fingerprint_summary),
        requirements: Some(requirements.clone()),
        environment_summary: Some(environment_summary),
        engine_backend_summary: Some(backend_summary),
        capabilities: Some(capabilities.clone()),
        configuration_options: Some(configuration_options.clone()),
        recommendation: Some(recommendation.clone()),
        presets: presets.clone(),
        custom_target_size: Some(custom_target_size.clone()),
        warnings,
        error: None,
    };
    let now = Instant::now();
    let snapshot = AnalysisSnapshot {
        id,
        revision,
        decision_model_revision: CURRENT_DECISION_MODEL_REVISION,
        estimator_model_revision: CURRENT_ESTIMATOR_MODEL_REVISION,
        task_mode: request.task_mode,
        media_request: request.media_request.clone(),
        media,
        requirements,
        source_fingerprint,
        environment,
        resource_sample,
        backend_catalog: catalog,
        native_backend_count,
        plugin_backend_count,
        capabilities,
        configuration_options,
        recommendation,
        presets,
        custom_target_size,
        estimator_policy,
        created_at: now,
        last_accessed_at: now,
    };
    (response, snapshot)
}

pub(crate) fn build_failure(
    id: AnalysisId,
    task_mode: TaskMode,
    error: EngineError,
) -> AnalyzeMediaResponse {
    AnalyzeMediaResponse {
        schema_version: "1.0".to_owned(),
        analysis_id: id,
        analysis_revision: AnalysisRevision::initial(),
        task_mode,
        media_analysis_status: MediaAnalysisStatus::Failed,
        configuration_status: ConfigurationStatus::NotEvaluated,
        media: None,
        source_fingerprint: None,
        requirements: None,
        environment_summary: None,
        engine_backend_summary: None,
        capabilities: None,
        configuration_options: None,
        recommendation: None,
        presets: Vec::new(),
        custom_target_size: None,
        warnings: Vec::new(),
        error: Some(ErrorDetail {
            code: error.code(),
            message: error.message().to_owned(),
            retryable: error.code().is_retryable(),
        }),
    }
}

pub(crate) fn snapshot_view(
    snapshot: &AnalysisSnapshot,
    validity: AnalysisSnapshotValidity,
) -> AnalysisSnapshotView {
    AnalysisSnapshotView {
        schema_version: "1.0".to_owned(),
        analysis_id: snapshot.id.clone(),
        analysis_revision: snapshot.revision,
        decision_model_revision: snapshot.decision_model_revision,
        estimator_model_revision: snapshot.estimator_model_revision,
        task_mode: snapshot.task_mode,
        media: snapshot.media.clone(),
        source_fingerprint: SourceFingerprintSummary::from_fingerprint(
            &snapshot.source_fingerprint,
            &snapshot.media.source_id,
        ),
        requirements: snapshot.requirements.clone(),
        environment_summary: environment_summary(
            &snapshot.environment,
            snapshot.resource_sample.as_ref(),
        ),
        engine_backend_summary: backend_summary(
            &snapshot.backend_catalog,
            snapshot.native_backend_count,
            snapshot.plugin_backend_count,
        ),
        capabilities: snapshot.capabilities.clone(),
        configuration_options: snapshot.configuration_options.clone(),
        recommendation: snapshot.recommendation.clone(),
        presets: snapshot.presets.clone(),
        custom_target_size: snapshot.custom_target_size.clone(),
        warnings: response_warnings(&snapshot.media, &snapshot.capabilities),
        validity,
    }
}

fn environment_summary(
    environment: &EnvironmentSnapshot,
    sample: Option<&ResourceSample>,
) -> EnvironmentSummary {
    EnvironmentSummary {
        operating_system: environment.operating_system.clone(),
        architecture: environment.cpu.architecture.clone(),
        logical_cores: environment.cpu.logical_cores,
        total_memory_bytes: environment.total_memory.value(),
        gpu_count: environment.gpus.value.as_ref().map(Vec::len),
        sampled_at_unix_ms: sample.map(|value| value.sampled_at_unix_ms),
    }
}

fn backend_summary(
    catalog: &BackendCatalog,
    native_backend_count: usize,
    plugin_backend_count: usize,
) -> EngineBackendSummary {
    EngineBackendSummary {
        catalog_revision: catalog.revision,
        backend_count: catalog.backends.len(),
        native_backend_count,
        plugin_backend_count,
        registered_backend_count: catalog
            .backends
            .iter()
            .filter(|value| {
                value.availability.engine_registration
                    == framelean_media::capability::EngineRegistrationStatus::EngineRegistered
            })
            .count(),
        execution_ready_backend_count: catalog
            .backends
            .iter()
            .filter(|value| value.availability.is_execution_ready())
            .count(),
    }
}

pub(crate) fn response_warnings(
    media: &MediaAnalysis,
    capabilities: &CapabilitySet,
) -> Vec<WarningDetail> {
    let mut warnings = media
        .warnings
        .iter()
        .map(|warning| WarningDetail {
            code: warning.code,
            message: warning.message.clone(),
        })
        .collect::<Vec<_>>();
    if !capabilities.available {
        warnings.push(WarningDetail {
            code: framelean_decision::ENGINE_EXECUTION_CHAIN_NOT_READY,
            message: "FrameLean has no execution-ready media chain".to_owned(),
        });
    }
    warnings
}
