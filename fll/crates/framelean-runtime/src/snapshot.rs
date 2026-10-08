use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use framelean_analysis::{MediaAnalysis, MediaAnalyzeRequest, SourceFingerprint};
use framelean_core::{AnalysisId, EngineError, EngineErrorCode, ErrorKind, Result};
use framelean_decision::{
    CapabilitySet, ConfigurationOptionGraph, CustomTargetSizeOptions, EstimatorPolicy,
    InputMediaRequirements, PresetDefinition, Recommendation, TaskMode,
    resolved_configuration_matches_candidate, validate_recommendation,
};
use framelean_environment::{EnvironmentSnapshot, ResourceSample};
use framelean_media::capability::BackendCatalog;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) const CURRENT_DECISION_MODEL_REVISION: u32 = 1;
pub(crate) const CURRENT_ESTIMATOR_MODEL_REVISION: u32 = 1;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct AnalysisRevision(u64);

impl AnalysisRevision {
    pub const fn initial() -> Self {
        Self(1)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Result<Self> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or_else(|| EngineError::new(ErrorKind::Snapshot, "analysis revision overflow"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionStrategy {
    LeastRecentlyUsed,
    Oldest,
    RejectNew,
}

#[derive(Debug, Clone)]
pub struct AnalysisSnapshotPolicy {
    pub max_entries: Option<usize>,
    pub idle_ttl: Option<Duration>,
    pub eviction_strategy: EvictionStrategy,
}

impl AnalysisSnapshotPolicy {
    pub fn new(
        max_entries: Option<usize>,
        idle_ttl: Option<Duration>,
        eviction_strategy: EvictionStrategy,
    ) -> Result<Self> {
        if max_entries == Some(0) {
            return Err(EngineError::invalid_argument(
                "snapshot max entries must be greater than zero",
            ));
        }
        if idle_ttl.is_some_and(|value| value.is_zero()) {
            return Err(EngineError::invalid_argument(
                "snapshot idle TTL must be greater than zero",
            ));
        }
        Ok(Self {
            max_entries,
            idle_ttl,
            eviction_strategy,
        })
    }
}

#[derive(Clone)]
pub(crate) struct AnalysisSnapshot {
    pub id: AnalysisId,
    pub revision: AnalysisRevision,
    pub decision_model_revision: u32,
    pub estimator_model_revision: u32,
    pub task_mode: TaskMode,
    pub media_request: MediaAnalyzeRequest,
    pub media: MediaAnalysis,
    pub requirements: InputMediaRequirements,
    pub source_fingerprint: SourceFingerprint,
    pub environment: EnvironmentSnapshot,
    pub resource_sample: Option<ResourceSample>,
    pub backend_catalog: BackendCatalog,
    pub native_backend_count: usize,
    pub plugin_backend_count: usize,
    pub capabilities: CapabilitySet,
    pub configuration_options: ConfigurationOptionGraph,
    pub recommendation: Recommendation,
    pub presets: Vec<PresetDefinition>,
    pub custom_target_size: CustomTargetSizeOptions,
    pub estimator_policy: EstimatorPolicy,
    pub created_at: Instant,
    pub last_accessed_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AnalysisSnapshotRecord {
    pub schema_version: String,
    pub id: AnalysisId,
    pub revision: AnalysisRevision,
    pub decision_model_revision: u32,
    pub estimator_model_revision: u32,
    pub task_mode: TaskMode,
    pub media_request: MediaAnalyzeRequest,
    pub media: MediaAnalysis,
    pub requirements: InputMediaRequirements,
    pub source_fingerprint: SourceFingerprint,
    pub environment: EnvironmentSnapshot,
    pub resource_sample: Option<ResourceSample>,
    pub backend_catalog: BackendCatalog,
    pub native_backend_count: usize,
    pub plugin_backend_count: usize,
    pub capabilities: CapabilitySet,
    pub configuration_options: ConfigurationOptionGraph,
    pub recommendation: Recommendation,
    pub presets: Vec<PresetDefinition>,
    pub custom_target_size: CustomTargetSizeOptions,
    pub estimator_policy: EstimatorPolicy,
}

impl AnalysisSnapshotRecord {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != "1.0" {
            return Err(EngineError::new(
                ErrorKind::Snapshot,
                "unsupported analysis snapshot record schema version",
            ));
        }
        if self.source_fingerprint.source_id()? != self.media.source_id {
            return Err(EngineError::new(
                ErrorKind::Snapshot,
                "analysis snapshot source fingerprint does not match media",
            ));
        }
        if InputMediaRequirements::from_media_analysis(&self.media) != self.requirements {
            return Err(EngineError::new(
                ErrorKind::Snapshot,
                "analysis snapshot input requirements do not match media",
            ));
        }
        if ConfigurationOptionGraph::from_capabilities(&self.capabilities)
            != self.configuration_options
        {
            return Err(EngineError::new(
                ErrorKind::Snapshot,
                "analysis snapshot configuration graph does not match candidates",
            ));
        }
        if CustomTargetSizeOptions::from_context(
            &self.capabilities,
            Some(&self.estimator_policy),
            self.requirements.source_size_bytes,
        ) != self.custom_target_size
        {
            return Err(EngineError::new(
                ErrorKind::Snapshot,
                "analysis snapshot target size options do not match estimator inputs",
            ));
        }
        if !validate_recommendation(&self.recommendation, &self.capabilities) {
            return Err(EngineError::new(
                ErrorKind::Snapshot,
                "analysis snapshot recommendation does not match a candidate",
            ));
        }
        for preset in &self.presets {
            if preset.id != preset.policy.id {
                return Err(EngineError::new(
                    ErrorKind::Snapshot,
                    "analysis snapshot preset identifier does not match its policy",
                ));
            }
            if !preset.applicable {
                continue;
            }
            let (Some(candidate), Some(configuration), Some(_)) = (
                preset.candidate.as_ref(),
                preset.configuration.as_ref(),
                preset.estimate.as_ref(),
            ) else {
                return Err(EngineError::new(
                    ErrorKind::Snapshot,
                    "applicable preset is incomplete",
                ));
            };
            if !self
                .capabilities
                .execution_chains
                .iter()
                .any(|value| value == candidate)
                || !resolved_configuration_matches_candidate(configuration, candidate)
                || configuration.selected_preset.as_ref() != Some(&preset.id)
            {
                return Err(EngineError::new(
                    ErrorKind::Snapshot,
                    "analysis snapshot preset does not match a candidate",
                ));
            }
        }
        Ok(())
    }
}

impl From<&AnalysisSnapshot> for AnalysisSnapshotRecord {
    fn from(snapshot: &AnalysisSnapshot) -> Self {
        Self {
            schema_version: "1.0".to_owned(),
            id: snapshot.id.clone(),
            revision: snapshot.revision,
            decision_model_revision: snapshot.decision_model_revision,
            estimator_model_revision: snapshot.estimator_model_revision,
            task_mode: snapshot.task_mode,
            media_request: snapshot.media_request.clone(),
            media: snapshot.media.clone(),
            requirements: snapshot.requirements.clone(),
            source_fingerprint: snapshot.source_fingerprint.clone(),
            environment: snapshot.environment.clone(),
            resource_sample: snapshot.resource_sample.clone(),
            backend_catalog: snapshot.backend_catalog.clone(),
            native_backend_count: snapshot.native_backend_count,
            plugin_backend_count: snapshot.plugin_backend_count,
            capabilities: snapshot.capabilities.clone(),
            configuration_options: snapshot.configuration_options.clone(),
            recommendation: snapshot.recommendation.clone(),
            presets: snapshot.presets.clone(),
            custom_target_size: snapshot.custom_target_size.clone(),
            estimator_policy: snapshot.estimator_policy.clone(),
        }
    }
}

impl TryFrom<AnalysisSnapshotRecord> for AnalysisSnapshot {
    type Error = EngineError;

    fn try_from(record: AnalysisSnapshotRecord) -> Result<Self> {
        record.validate()?;
        let now = Instant::now();
        Ok(Self {
            id: record.id,
            revision: record.revision,
            decision_model_revision: record.decision_model_revision,
            estimator_model_revision: record.estimator_model_revision,
            task_mode: record.task_mode,
            media_request: record.media_request,
            media: record.media,
            requirements: record.requirements,
            source_fingerprint: record.source_fingerprint,
            environment: record.environment,
            resource_sample: record.resource_sample,
            backend_catalog: record.backend_catalog,
            native_backend_count: record.native_backend_count,
            plugin_backend_count: record.plugin_backend_count,
            capabilities: record.capabilities,
            configuration_options: record.configuration_options,
            recommendation: record.recommendation,
            presets: record.presets,
            custom_target_size: record.custom_target_size,
            estimator_policy: record.estimator_policy,
            created_at: now,
            last_accessed_at: now,
        })
    }
}

pub struct AnalysisSnapshotStore {
    policy: AnalysisSnapshotPolicy,
    snapshots: HashMap<AnalysisId, AnalysisSnapshot>,
    access_order: VecDeque<AnalysisId>,
}

impl AnalysisSnapshotStore {
    pub fn new(policy: AnalysisSnapshotPolicy) -> Self {
        Self {
            policy,
            snapshots: HashMap::new(),
            access_order: VecDeque::new(),
        }
    }

    pub(crate) fn insert(&mut self, snapshot: AnalysisSnapshot) -> Result<()> {
        self.remove_expired();
        if let Some(existing) = self.snapshots.get(&snapshot.id) {
            if existing.revision != snapshot.revision {
                return Err(EngineError::with_code(
                    ErrorKind::Snapshot,
                    EngineErrorCode::AnalysisRevisionConflict,
                    "analysis snapshot id already stores a different revision",
                ));
            }
            return Err(EngineError::new(
                ErrorKind::Snapshot,
                "analysis snapshot id is already stored",
            ));
        }
        if self
            .policy
            .max_entries
            .is_some_and(|maximum| self.snapshots.len() >= maximum)
        {
            match self.policy.eviction_strategy {
                EvictionStrategy::RejectNew => {
                    return Err(EngineError::new(
                        ErrorKind::Snapshot,
                        "analysis snapshot capacity reached",
                    ));
                }
                EvictionStrategy::LeastRecentlyUsed => self.evict_access_order(),
                EvictionStrategy::Oldest => self.evict_oldest(),
            }
        }
        self.access_order.push_back(snapshot.id.clone());
        self.snapshots.insert(snapshot.id.clone(), snapshot);
        Ok(())
    }

    pub(crate) fn get_mut(&mut self, id: &AnalysisId) -> Result<&mut AnalysisSnapshot> {
        self.remove_expired();
        let snapshot = self.snapshots.get_mut(id).ok_or_else(|| {
            EngineError::with_code(
                ErrorKind::Snapshot,
                EngineErrorCode::AnalysisSnapshotExpired,
                "analysis snapshot is missing or expired",
            )
        })?;
        snapshot.last_accessed_at = Instant::now();
        self.access_order.retain(|value| value != id);
        self.access_order.push_back(id.clone());
        Ok(snapshot)
    }

    pub(crate) fn remove(&mut self, id: &AnalysisId) -> bool {
        self.access_order.retain(|value| value != id);
        self.snapshots.remove(id).is_some()
    }

    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }

    fn remove_expired(&mut self) {
        let Some(ttl) = self.policy.idle_ttl else {
            return;
        };
        let now = Instant::now();
        self.snapshots
            .retain(|_, snapshot| now.duration_since(snapshot.last_accessed_at) < ttl);
        self.access_order
            .retain(|id| self.snapshots.contains_key(id));
    }

    fn evict_access_order(&mut self) {
        if let Some(id) = self.access_order.pop_front() {
            self.snapshots.remove(&id);
        }
    }

    fn evict_oldest(&mut self) {
        if let Some(id) = self
            .snapshots
            .values()
            .min_by_key(|snapshot| snapshot.created_at)
            .map(|snapshot| snapshot.id.clone())
        {
            self.snapshots.remove(&id);
            self.access_order.retain(|value| value != &id);
        }
    }
}
