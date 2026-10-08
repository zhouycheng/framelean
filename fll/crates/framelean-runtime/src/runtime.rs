use std::sync::Arc;

use framelean_analysis::MediaKind;
use framelean_core::{AnalysisId, EngineError, EngineErrorCode, ProcessorId, Result, TaskId};
use framelean_decision::{
    ConfigurationOptionGraph, CustomTargetSizeOptions, DecisionService, DeterministicSizeEstimator,
    InputMediaRequirements, fixed_presets,
};
use framelean_media::capability::{
    BackendCapability, BackendCatalog, BackendCatalogProvider, StreamKind,
};
use framelean_media::processor::{ProcessOutput, ProcessorMetadata};
use framelean_pipeline::{
    ExecutionContext, MediaPipelineNode, MediaPipelinePlan, Pipeline, PipelineBuilder,
};
use framelean_plugin::{Plugin, PluginError, PluginRegistry, ProcessorFactory};

use crate::analysis::{
    AnalysisAssembly, AnalysisSnapshotValidity, build_failure, build_success, collect_catalog,
    response_warnings, snapshot_view,
};
use crate::snapshot::{CURRENT_DECISION_MODEL_REVISION, CURRENT_ESTIMATOR_MODEL_REVISION};
use crate::{
    AnalysisServices, AnalysisSnapshotPolicy, AnalysisSnapshotRecord, AnalysisSnapshotStore,
    AnalysisSnapshotView, AnalyzeMediaResponse, AnalyzeTaskRequest, ConfigurationConflict,
    ExecutionLaneSnapshot, ExecutionResourcePool, ExecutionRuntime, ExecutionRuntimeEvent,
    ExecutionRuntimePlan, ExecutionServices, ExecutionSubmissionRequest, ExecutionSubmissionResult,
    ExecutionTaskState, FifoScheduler, PipelineSpec, RecalculateConfigurationRequest,
    RecalculateConfigurationResponse, Scheduler, Task, TaskRequest,
};

pub struct EngineRuntime {
    registry: PluginRegistry,
    scheduler: FifoScheduler,
    next_task_number: u64,
    analysis_services: Option<AnalysisServices>,
    analysis_snapshots: Option<AnalysisSnapshotStore>,
    execution_runtime: Option<ExecutionRuntime>,
}

impl EngineRuntime {
    pub fn new() -> Self {
        Self {
            registry: PluginRegistry::new(),
            scheduler: FifoScheduler::new(),
            next_task_number: 1,
            analysis_services: None,
            analysis_snapshots: None,
            execution_runtime: None,
        }
    }

    pub fn with_analysis_services(
        services: AnalysisServices,
        snapshot_policy: AnalysisSnapshotPolicy,
    ) -> Self {
        let mut runtime = Self::new();
        runtime.analysis_services = Some(services);
        runtime.analysis_snapshots = Some(AnalysisSnapshotStore::new(snapshot_policy));
        runtime
    }

    pub fn with_analysis_and_execution_services(
        analysis_services: AnalysisServices,
        snapshot_policy: AnalysisSnapshotPolicy,
        execution_services: ExecutionServices,
    ) -> Self {
        let mut runtime = Self::with_analysis_services(analysis_services, snapshot_policy);
        runtime.execution_runtime = Some(ExecutionRuntime::new(execution_services));
        runtime
    }

    pub fn analyze_media(&mut self, request: AnalyzeTaskRequest) -> Result<AnalyzeMediaResponse> {
        let analysis_id = AnalysisId::generate();

        let services = self.analysis_services.as_ref().ok_or_else(|| {
            EngineError::new(
                framelean_core::ErrorKind::Runtime,
                "analysis services are not configured",
            )
        })?;
        let analyzer = Arc::clone(&services.analyzer);
        let environment_provider = Arc::clone(&services.environment);
        let monitor = Arc::clone(&services.resource_monitor);
        let native_providers = services.native_backend_providers.clone();
        let capability_resolver = Arc::clone(&services.capability_resolver);
        let recommendation_engine = Arc::clone(&services.recommendation_engine);
        let size_estimator = Arc::clone(&services.size_estimator);
        let estimator_policy = services.estimator_policy.clone();

        let analyzed = match analyzer.analyze(&request.media_request) {
            Ok(analyzed) => analyzed,
            Err(error) => return Ok(build_failure(analysis_id, request.task_mode, error)),
        };
        let (media, source_fingerprint) = analyzed.into_parts();
        if let Some(expected_source) = &request.media_request.expected_source
            && let Err(error) = expected_source.validate(&source_fingerprint)
        {
            return Ok(build_failure(analysis_id, request.task_mode, error));
        }
        let requirements = InputMediaRequirements::from_media_analysis(&media);
        let environment = environment_provider.snapshot()?;
        let resource_sample = monitor.sample().ok();
        let plugin_catalog = self.registry.backend_catalog()?;
        let collected_catalog = collect_catalog(&native_providers, plugin_catalog)?;
        let catalog = collected_catalog.catalog;
        let capabilities = capability_resolver.resolve(
            &requirements,
            request.task_mode,
            &environment,
            &catalog,
        )?;
        let recommendation = recommendation_engine.recommend(
            &requirements,
            request.task_mode,
            &capabilities,
            resource_sample.as_ref(),
            (size_estimator.as_ref(), &estimator_policy),
        );
        let configuration_options = ConfigurationOptionGraph::from_capabilities(&capabilities);
        let presets = fixed_presets(
            &requirements,
            request.task_mode,
            &capabilities,
            (size_estimator.as_ref(), &estimator_policy),
        );
        let custom_target_size = CustomTargetSizeOptions::from_context(
            &capabilities,
            Some(&estimator_policy),
            media.file_size.value(),
        );
        let (response, snapshot) = build_success(
            analysis_id,
            &request,
            AnalysisAssembly {
                media,
                requirements,
                source_fingerprint,
                environment,
                resource_sample,
                catalog,
                native_backend_count: collected_catalog.native_backend_count,
                plugin_backend_count: collected_catalog.plugin_backend_count,
                capabilities,
                configuration_options,
                recommendation,
                presets,
                custom_target_size,
                estimator_policy,
            },
        );
        self.analysis_snapshots
            .as_mut()
            .expect("analysis snapshot store is configured with analysis services")
            .insert(snapshot)?;
        Ok(response)
    }

    pub fn recalculate_configuration(
        &mut self,
        request: RecalculateConfigurationRequest,
    ) -> Result<RecalculateConfigurationResponse> {
        let snapshot = self
            .analysis_snapshots
            .as_mut()
            .ok_or_else(|| {
                EngineError::new(
                    framelean_core::ErrorKind::Snapshot,
                    "analysis snapshot store is not configured",
                )
            })?
            .get_mut(&request.analysis_id)?;

        if snapshot.revision != request.expected_revision {
            return Err(EngineError::with_code(
                framelean_core::ErrorKind::Snapshot,
                EngineErrorCode::AnalysisRevisionConflict,
                "analysis revision does not match the current snapshot",
            ));
        }
        if snapshot.decision_model_revision != CURRENT_DECISION_MODEL_REVISION {
            return Err(EngineError::new(
                framelean_core::ErrorKind::Snapshot,
                format!(
                    "analysis snapshot decision model revision {} is incompatible with runtime revision {}",
                    snapshot.decision_model_revision, CURRENT_DECISION_MODEL_REVISION
                ),
            ));
        }
        if snapshot.estimator_model_revision != CURRENT_ESTIMATOR_MODEL_REVISION {
            return Err(EngineError::new(
                framelean_core::ErrorKind::Snapshot,
                format!(
                    "analysis snapshot estimator model revision {} is incompatible with runtime revision {}",
                    snapshot.estimator_model_revision, CURRENT_ESTIMATOR_MODEL_REVISION
                ),
            ));
        }
        let current_fingerprint = framelean_analysis::SourceFingerprint::from_local_file(
            snapshot.media_request.source.path(),
        )
        .map_err(|error| {
            EngineError::with_source_code(
                framelean_core::ErrorKind::Snapshot,
                EngineErrorCode::AnalysisSourceChanged,
                "analysis source can no longer be fingerprinted",
                error,
            )
        })?;
        if current_fingerprint != snapshot.source_fingerprint {
            return Err(EngineError::with_code(
                framelean_core::ErrorKind::Snapshot,
                EngineErrorCode::AnalysisSourceChanged,
                "analysis source fingerprint changed",
            ));
        }

        let frozen_estimator = DeterministicSizeEstimator;
        let resolution = DecisionService.resolve_selection_from_snapshot(
            &request.selection,
            &snapshot.requirements,
            snapshot.task_mode,
            &snapshot.capabilities,
            &snapshot.presets,
            Some((
                &frozen_estimator as &dyn framelean_decision::SizeEstimator,
                &snapshot.estimator_policy,
            )),
        );
        let (resolved_configuration, conflicts) = match resolution {
            Ok(resolved) => (Some(resolved), Vec::new()),
            Err(conflict) => (
                None,
                vec![ConfigurationConflict {
                    code: conflict.code,
                    field: conflict.field,
                    message: conflict.message,
                }],
            ),
        };
        let warnings = response_warnings(&snapshot.media, &snapshot.capabilities);

        Ok(RecalculateConfigurationResponse {
            schema_version: "1.0".to_owned(),
            analysis_id: request.analysis_id,
            analysis_revision: snapshot.revision,
            configuration_status: if snapshot.capabilities.available {
                framelean_decision::ConfigurationStatus::Available
            } else {
                framelean_decision::ConfigurationStatus::Unavailable
            },
            capabilities: snapshot.capabilities.clone(),
            configuration_options: snapshot.configuration_options.clone(),
            recommendation: snapshot.recommendation.clone(),
            presets: snapshot.presets.clone(),
            custom_target_size: snapshot.custom_target_size.clone(),
            selection: request.selection,
            resolved_configuration,
            conflicts,
            warnings,
            error: None,
        })
    }

    /// Atomically validates the durable analysis selection and creates a real
    /// media execution task. The selected backend remains responsible for
    /// rejecting execution chains it cannot actually run.
    pub fn submit_execution(
        &mut self,
        request: ExecutionSubmissionRequest,
    ) -> Result<ExecutionSubmissionResult> {
        request.output.validate()?;
        let analysis_id = request.analysis_id.clone();
        let output = request.output;
        let recalculated = self.recalculate_configuration(RecalculateConfigurationRequest {
            analysis_id: request.analysis_id,
            expected_revision: request.expected_revision,
            selection: request.selection,
            context: request.context,
        })?;
        let Some(resolved_configuration) = recalculated.resolved_configuration else {
            let conflict = recalculated.conflicts.first();
            return Err(EngineError::with_code(
                framelean_core::ErrorKind::Runtime,
                conflict.map_or(EngineErrorCode::EngineExecutionChainNotReady, |value| {
                    value.code
                }),
                conflict.map_or("media execution configuration is unavailable", |value| {
                    value.message.as_str()
                }),
            ));
        };
        let (source_path, backend_catalog, resource_pool) = {
            let snapshot = self
                .analysis_snapshots
                .as_mut()
                .ok_or_else(|| {
                    EngineError::new(
                        framelean_core::ErrorKind::Snapshot,
                        "analysis snapshot store is not configured",
                    )
                })?
                .get_mut(&analysis_id)?;
            (
                snapshot.media_request.source.path().to_path_buf(),
                snapshot.backend_catalog.clone(),
                match snapshot.media.kind {
                    MediaKind::Video => ExecutionResourcePool::Video,
                    MediaKind::Audio
                    | MediaKind::Image
                    | MediaKind::AnimatedImage
                    | MediaKind::Other => ExecutionResourcePool::Auxiliary,
                },
            )
        };
        let pipeline = build_media_pipeline_plan(&resolved_configuration, &backend_catalog)?;
        let execution_id = self
            .execution_runtime
            .as_mut()
            .ok_or_else(|| {
                EngineError::with_code(
                    framelean_core::ErrorKind::Pipeline,
                    EngineErrorCode::EngineExecutionChainNotReady,
                    "media execution services are not configured",
                )
            })?
            .submit(ExecutionRuntimePlan {
                source_path,
                output,
                pipeline,
                configuration: resolved_configuration,
                resource_pool,
            })?;
        let execution_lane = self.execution_snapshot()?;
        let state = if execution_lane
            .active_executions
            .iter()
            .any(|entry| entry.execution_id == execution_id)
        {
            ExecutionTaskState::Running
        } else {
            ExecutionTaskState::Queued
        };
        let queue_position = execution_lane
            .normal_waiting
            .iter()
            .position(|entry| entry.execution_id == execution_id)
            .map_or(0, |position| position + 1);
        Ok(ExecutionSubmissionResult {
            execution_id,
            state,
            queue_position,
            queue_revision: execution_lane.queue_revision,
        })
    }

    pub fn execution_snapshot(&self) -> Result<ExecutionLaneSnapshot> {
        self.execution_runtime
            .as_ref()
            .map(ExecutionRuntime::snapshot)
            .ok_or_else(|| {
                EngineError::with_code(
                    framelean_core::ErrorKind::Pipeline,
                    EngineErrorCode::EngineExecutionChainNotReady,
                    "media execution services are not configured",
                )
            })
    }

    pub fn reorder_waiting_executions(
        &mut self,
        expected_revision: u64,
        ordered_execution_ids: &[TaskId],
    ) -> Result<u64> {
        self.execution_runtime
            .as_mut()
            .ok_or_else(|| {
                EngineError::with_code(
                    framelean_core::ErrorKind::Pipeline,
                    EngineErrorCode::EngineExecutionChainNotReady,
                    "media execution services are not configured",
                )
            })?
            .reorder_waiting(expected_revision, ordered_execution_ids)
    }

    pub fn preempt_and_start_execution(&mut self, execution_id: &TaskId) -> Result<()> {
        self.execution_runtime
            .as_mut()
            .ok_or_else(|| {
                EngineError::with_code(
                    framelean_core::ErrorKind::Pipeline,
                    EngineErrorCode::EngineExecutionChainNotReady,
                    "media execution services are not configured",
                )
            })?
            .preempt_and_start(execution_id)
    }

    pub fn pause_execution(&mut self, execution_id: &TaskId) -> Result<()> {
        self.execution_runtime
            .as_mut()
            .ok_or_else(|| {
                EngineError::with_code(
                    framelean_core::ErrorKind::Pipeline,
                    EngineErrorCode::EngineExecutionChainNotReady,
                    "media execution services are not configured",
                )
            })?
            .pause_for_user(execution_id)
    }

    pub fn resume_execution(&mut self, execution_id: &TaskId) -> Result<()> {
        self.execution_runtime
            .as_mut()
            .ok_or_else(|| {
                EngineError::with_code(
                    framelean_core::ErrorKind::Pipeline,
                    EngineErrorCode::EngineExecutionChainNotReady,
                    "media execution services are not configured",
                )
            })?
            .resume_user_paused(execution_id)
    }

    pub fn cancel_execution(&mut self, execution_id: &TaskId) -> Result<()> {
        self.execution_runtime
            .as_mut()
            .ok_or_else(|| {
                EngineError::with_code(
                    framelean_core::ErrorKind::Pipeline,
                    EngineErrorCode::EngineExecutionChainNotReady,
                    "media execution services are not configured",
                )
            })?
            .cancel(execution_id)
    }

    pub fn drain_execution_events(&mut self) -> Vec<ExecutionRuntimeEvent> {
        self.execution_runtime
            .as_mut()
            .map(ExecutionRuntime::drain_events)
            .unwrap_or_default()
    }

    pub fn analysis_snapshot_count(&self) -> usize {
        self.analysis_snapshots
            .as_ref()
            .map_or(0, AnalysisSnapshotStore::len)
    }

    pub fn analysis_snapshot(&mut self, analysis_id: &AnalysisId) -> Result<AnalysisSnapshotView> {
        let snapshot = self
            .analysis_snapshots
            .as_mut()
            .ok_or_else(|| {
                EngineError::new(
                    framelean_core::ErrorKind::Snapshot,
                    "analysis snapshot store is not configured",
                )
            })?
            .get_mut(analysis_id)?;
        let validity = match framelean_analysis::SourceFingerprint::from_local_file(
            snapshot.media_request.source.path(),
        ) {
            Ok(current) if current == snapshot.source_fingerprint => {
                AnalysisSnapshotValidity::valid()
            }
            Ok(_) => {
                AnalysisSnapshotValidity::source_changed("analysis source fingerprint changed")
            }
            Err(error) => AnalysisSnapshotValidity::source_changed(format!(
                "analysis source can no longer be fingerprinted: {}",
                error.message()
            )),
        };
        Ok(snapshot_view(snapshot, validity))
    }

    pub fn analysis_snapshot_record(
        &mut self,
        analysis_id: &AnalysisId,
    ) -> Result<AnalysisSnapshotRecord> {
        let snapshot = self
            .analysis_snapshots
            .as_mut()
            .ok_or_else(|| {
                EngineError::new(
                    framelean_core::ErrorKind::Snapshot,
                    "analysis snapshot store is not configured",
                )
            })?
            .get_mut(analysis_id)?;
        Ok(AnalysisSnapshotRecord::from(&*snapshot))
    }

    pub fn discard_analysis_snapshot(&mut self, analysis_id: &AnalysisId) -> Result<bool> {
        Ok(self
            .analysis_snapshots
            .as_mut()
            .ok_or_else(|| {
                EngineError::new(
                    framelean_core::ErrorKind::Snapshot,
                    "analysis snapshot store is not configured",
                )
            })?
            .remove(analysis_id))
    }

    pub fn restore_analysis_snapshot(&mut self, record: AnalysisSnapshotRecord) -> Result<()> {
        let snapshot = record.try_into()?;
        self.analysis_snapshots
            .as_mut()
            .ok_or_else(|| {
                EngineError::new(
                    framelean_core::ErrorKind::Snapshot,
                    "analysis snapshot store is not configured",
                )
            })?
            .insert(snapshot)?;
        Ok(())
    }

    pub fn register_plugin(&mut self, plugin: &dyn Plugin) -> Result<()> {
        plugin
            .register(&mut self.registry)
            .map_err(EngineError::from)
    }

    pub fn submit(&mut self, request: TaskRequest) -> Result<TaskId> {
        let task_id = TaskId::new(format!("task-{}", self.next_task_number))?;
        self.next_task_number += 1;
        self.scheduler
            .enqueue(Task::from_request(task_id.clone(), request))?;
        Ok(task_id)
    }

    pub fn queued_task_count(&self) -> usize {
        self.scheduler.len()
    }

    pub fn run_next(&mut self) -> Result<Option<Task>> {
        let Some(mut task) = self.scheduler.dequeue() else {
            return Ok(None);
        };
        task.start()?;

        let result = self.execute_task(&mut task);
        match result {
            Ok(output) => task.complete(output)?,
            Err(error) => task.fail(error)?,
        }
        Ok(Some(task))
    }

    fn execute_task(&mut self, task: &mut Task) -> Result<ProcessOutput> {
        let processor_ids = task.pipeline().processors().to_vec();
        let mut pipeline = self.build_pipeline(&PipelineSpec::new(processor_ids))?;
        let input = task.take_input()?;
        let mut context = ExecutionContext::new(task.id().clone());
        pipeline
            .execute(input, &mut context)
            .map_err(EngineError::from)
    }

    fn build_pipeline(&self, specification: &PipelineSpec) -> Result<Pipeline> {
        let mut builder = PipelineBuilder::new();
        for requested_id in specification.processors() {
            let factory = self
                .registry
                .factory(requested_id)
                .map_err(EngineError::from)?;
            self.validate_factory(requested_id, factory)?;

            let processor = factory.create().map_err(EngineError::from)?;
            self.validate_processor(requested_id, factory.metadata(), processor.metadata())?;
            builder
                .add_processor(processor)
                .map_err(EngineError::from)?;
        }
        builder.build().map_err(EngineError::from)
    }

    fn validate_factory(
        &self,
        requested_id: &ProcessorId,
        factory: &dyn ProcessorFactory,
    ) -> Result<()> {
        let actual_id = factory.metadata().id();
        if actual_id != requested_id {
            return Err(PluginError::FactoryMetadataMismatch {
                requested: requested_id.clone(),
                actual: actual_id.clone(),
            }
            .into());
        }
        Ok(())
    }

    fn validate_processor(
        &self,
        requested_id: &ProcessorId,
        factory_metadata: &ProcessorMetadata,
        processor_metadata: &ProcessorMetadata,
    ) -> Result<()> {
        if processor_metadata.id() != requested_id {
            return Err(PluginError::ProcessorMetadataMismatch {
                requested: requested_id.clone(),
                actual: processor_metadata.id().clone(),
            }
            .into());
        }
        if processor_metadata.stage() != factory_metadata.stage() {
            return Err(PluginError::ProcessorStageMismatch {
                expected: factory_metadata.stage(),
                actual: processor_metadata.stage(),
            }
            .into());
        }
        Ok(())
    }
}

fn build_media_pipeline_plan(
    configuration: &framelean_decision::ResolvedConfiguration,
    catalog: &BackendCatalog,
) -> Result<MediaPipelinePlan> {
    let mut nodes = vec![
        MediaPipelineNode::Source,
        MediaPipelineNode::Demuxer {
            backend_id: configuration.demuxer_backend.clone(),
        },
    ];
    nodes.extend(
        configuration
            .video_decoders
            .iter()
            .map(|selection| MediaPipelineNode::Decoder {
                backend_id: selection.backend_id.clone(),
                stream_index: selection.stream_index,
                stream_kind: StreamKind::Video,
            }),
    );
    nodes.extend(
        configuration
            .audio_streams
            .iter()
            .map(|selection| MediaPipelineNode::Decoder {
                backend_id: selection.decoder_backend.clone(),
                stream_index: selection.input_stream_index,
                stream_kind: StreamKind::Audio,
            }),
    );
    for selection in &configuration.processors {
        let backend = catalog
            .backends
            .iter()
            .find(|backend| backend.id == selection.backend_id)
            .ok_or_else(|| {
                EngineError::with_code(
                    framelean_core::ErrorKind::Pipeline,
                    EngineErrorCode::EngineExecutionChainNotReady,
                    format!(
                        "selected processor backend {} is absent from the frozen catalog",
                        selection.backend_id
                    ),
                )
            })?;
        let BackendCapability::Processor(capability) = &backend.capability else {
            return Err(EngineError::with_code(
                framelean_core::ErrorKind::Pipeline,
                EngineErrorCode::EngineExecutionChainNotReady,
                format!(
                    "selected backend {} is not a processor",
                    selection.backend_id
                ),
            ));
        };
        nodes.push(match capability.stream_type {
            StreamKind::Video => MediaPipelineNode::VideoProcessor {
                backend_id: selection.backend_id.clone(),
                operation: selection.operation.clone(),
            },
            StreamKind::Audio => MediaPipelineNode::AudioProcessor {
                backend_id: selection.backend_id.clone(),
                operation: selection.operation.clone(),
            },
            StreamKind::Subtitle | StreamKind::Data | StreamKind::Attachment => {
                return Err(EngineError::with_code(
                    framelean_core::ErrorKind::Pipeline,
                    EngineErrorCode::EngineExecutionChainNotReady,
                    "selected processor stream kind is not implemented",
                ));
            }
        });
    }
    if let Some(backend_id) = &configuration.video_encoder_backend {
        nodes.push(MediaPipelineNode::Encoder {
            backend_id: backend_id.clone(),
            stream_kind: StreamKind::Video,
        });
    }
    for stream in &configuration.audio_streams {
        nodes.push(MediaPipelineNode::Encoder {
            backend_id: stream.encoder_backend.clone(),
            stream_kind: StreamKind::Audio,
        });
    }
    nodes.extend([
        MediaPipelineNode::Muxer {
            backend_id: configuration.muxer_backend.clone(),
        },
        MediaPipelineNode::Sink,
    ]);
    MediaPipelinePlan::new(nodes).map_err(EngineError::from)
}

impl Default for EngineRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use framelean_core::{ErrorKind, ProcessorId};
    use framelean_media::processor::{
        ProcessInput, ProcessingStage, Processor, ProcessorContext, ProcessorError, ProcessorResult,
    };
    use framelean_media::{AudioFrame, MediaBuffer, StreamId, VideoFrame};
    use framelean_plugin::{PluginMetadata, PluginResult};

    use super::*;
    use crate::TaskState;

    enum Behavior {
        Passthrough,
        Fail,
    }

    struct TestProcessor {
        metadata: ProcessorMetadata,
        calls: Arc<AtomicUsize>,
        behavior: Behavior,
    }

    impl Processor for TestProcessor {
        fn metadata(&self) -> &ProcessorMetadata {
            &self.metadata
        }

        fn process(
            &mut self,
            input: ProcessInput,
            _context: &mut ProcessorContext,
        ) -> ProcessorResult<ProcessOutput> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.behavior {
                Behavior::Passthrough => Ok(match input {
                    ProcessInput::Packet(value) => ProcessOutput::Packet(value),
                    ProcessInput::Video(value) => ProcessOutput::Video(value),
                    ProcessInput::Audio(value) => ProcessOutput::Audio(value),
                }),
                Behavior::Fail => Err(ProcessorError::failed("runtime fixture failed")),
            }
        }
    }

    struct TestFactory {
        metadata: ProcessorMetadata,
        created_metadata: ProcessorMetadata,
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl ProcessorFactory for TestFactory {
        fn metadata(&self) -> &ProcessorMetadata {
            &self.metadata
        }

        fn create(&self) -> PluginResult<Box<dyn Processor>> {
            Ok(Box::new(TestProcessor {
                metadata: self.created_metadata.clone(),
                calls: Arc::clone(&self.calls),
                behavior: if self.fail {
                    Behavior::Fail
                } else {
                    Behavior::Passthrough
                },
            }))
        }
    }

    struct TestPlugin {
        metadata: PluginMetadata,
        factories: Vec<TestFactory>,
    }

    impl Plugin for TestPlugin {
        fn metadata(&self) -> &PluginMetadata {
            &self.metadata
        }

        fn register(&self, registry: &mut PluginRegistry) -> PluginResult<()> {
            for factory in &self.factories {
                registry.register_factory(Box::new(TestFactory {
                    metadata: factory.metadata.clone(),
                    created_metadata: factory.created_metadata.clone(),
                    calls: Arc::clone(&factory.calls),
                    fail: factory.fail,
                }))?;
            }
            Ok(())
        }
    }

    fn metadata(id: &str, stage: ProcessingStage) -> ProcessorMetadata {
        ProcessorMetadata::new(ProcessorId::new(id).unwrap(), id, stage).unwrap()
    }

    fn plugin(factory: TestFactory) -> TestPlugin {
        TestPlugin {
            metadata: PluginMetadata::new("test.plugin", "Test Plugin").unwrap(),
            factories: vec![factory],
        }
    }

    fn factory(
        id: &str,
        stage: ProcessingStage,
        calls: Arc<AtomicUsize>,
        fail: bool,
    ) -> TestFactory {
        let metadata = metadata(id, stage);
        TestFactory {
            created_metadata: metadata.clone(),
            metadata,
            calls,
            fail,
        }
    }

    fn video_request(processors: Vec<ProcessorId>) -> TaskRequest {
        TaskRequest::new(
            PipelineSpec::new(processors),
            ProcessInput::Video(VideoFrame::new(
                StreamId::new(0),
                None,
                MediaBuffer::new(vec![1, 2, 3]),
            )),
        )
    }

    #[test]
    fn runtime_composes_duplicate_factory_entries_into_completed_task() {
        let calls = Arc::new(AtomicUsize::new(0));
        let processor_id = ProcessorId::new("test.passthrough").unwrap();
        let plugin = plugin(factory(
            processor_id.as_str(),
            ProcessingStage::Video,
            Arc::clone(&calls),
            false,
        ));
        let mut runtime = EngineRuntime::new();
        runtime.register_plugin(&plugin).unwrap();
        runtime
            .submit(video_request(vec![processor_id.clone(), processor_id]))
            .unwrap();

        let task = runtime.run_next().unwrap().unwrap();

        assert_eq!(task.state(), TaskState::Completed);
        assert!(matches!(task.output(), Some(ProcessOutput::Video(_))));
        assert!(task.failure().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn processor_failure_is_saved_on_failed_task() {
        let processor_id = ProcessorId::new("test.failure").unwrap();
        let plugin = plugin(factory(
            processor_id.as_str(),
            ProcessingStage::Video,
            Arc::new(AtomicUsize::new(0)),
            true,
        ));
        let mut runtime = EngineRuntime::new();
        runtime.register_plugin(&plugin).unwrap();
        runtime.submit(video_request(vec![processor_id])).unwrap();

        let task = runtime.run_next().unwrap().unwrap();

        assert_eq!(task.state(), TaskState::Failed);
        assert!(task.output().is_none());
        assert_eq!(task.failure().unwrap().kind(), ErrorKind::Pipeline);
    }

    #[test]
    fn input_stage_mismatch_fails_before_processor_runs() {
        let calls = Arc::new(AtomicUsize::new(0));
        let processor_id = ProcessorId::new("test.video").unwrap();
        let plugin = plugin(factory(
            processor_id.as_str(),
            ProcessingStage::Video,
            Arc::clone(&calls),
            false,
        ));
        let mut runtime = EngineRuntime::new();
        runtime.register_plugin(&plugin).unwrap();
        runtime
            .submit(TaskRequest::new(
                PipelineSpec::new(vec![processor_id]),
                ProcessInput::Audio(AudioFrame::new(
                    StreamId::new(0),
                    None,
                    MediaBuffer::new(vec![1]),
                )),
            ))
            .unwrap();

        let task = runtime.run_next().unwrap().unwrap();

        assert_eq!(task.state(), TaskState::Failed);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(task.failure().unwrap().kind(), ErrorKind::Pipeline);
    }

    #[test]
    fn processor_metadata_contract_is_checked_defensively() {
        let requested = ProcessorId::new("test.requested").unwrap();
        let mut mismatched = factory(
            requested.as_str(),
            ProcessingStage::Video,
            Arc::new(AtomicUsize::new(0)),
            false,
        );
        mismatched.created_metadata = metadata("test.actual", ProcessingStage::Video);
        let plugin = plugin(mismatched);
        let mut runtime = EngineRuntime::new();
        runtime.register_plugin(&plugin).unwrap();
        runtime.submit(video_request(vec![requested])).unwrap();

        let task = runtime.run_next().unwrap().unwrap();

        assert_eq!(task.state(), TaskState::Failed);
        assert_eq!(task.failure().unwrap().kind(), ErrorKind::Plugin);
    }

    #[test]
    fn factory_metadata_contract_is_checked_defensively() {
        let requested = ProcessorId::new("test.requested-factory").unwrap();
        let factory = factory(
            "test.actual-factory",
            ProcessingStage::Video,
            Arc::new(AtomicUsize::new(0)),
            false,
        );
        let runtime = EngineRuntime::new();

        let error = runtime.validate_factory(&requested, &factory).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Plugin);
        assert!(error.message().contains("test.actual-factory"));
        assert!(error.message().contains("test.requested-factory"));
    }

    #[test]
    fn processor_stage_contract_is_checked_defensively() {
        let requested = ProcessorId::new("test.stage").unwrap();
        let mut mismatched = factory(
            requested.as_str(),
            ProcessingStage::Video,
            Arc::new(AtomicUsize::new(0)),
            false,
        );
        mismatched.created_metadata = metadata(requested.as_str(), ProcessingStage::Audio);
        let plugin = plugin(mismatched);
        let mut runtime = EngineRuntime::new();
        runtime.register_plugin(&plugin).unwrap();
        runtime.submit(video_request(vec![requested])).unwrap();

        let task = runtime.run_next().unwrap().unwrap();

        assert_eq!(task.state(), TaskState::Failed);
        assert_eq!(task.failure().unwrap().kind(), ErrorKind::Plugin);
    }

    #[test]
    fn unknown_processor_id_fails_task() {
        let mut runtime = EngineRuntime::new();
        runtime
            .submit(video_request(vec![
                ProcessorId::new("test.unknown").unwrap(),
            ]))
            .unwrap();

        let task = runtime.run_next().unwrap().unwrap();

        assert_eq!(task.state(), TaskState::Failed);
        assert_eq!(task.failure().unwrap().kind(), ErrorKind::Plugin);
    }

    #[test]
    fn fifo_scheduler_preserves_submission_order() {
        let calls = Arc::new(AtomicUsize::new(0));
        let processor_id = ProcessorId::new("test.fifo").unwrap();
        let plugin = plugin(factory(
            processor_id.as_str(),
            ProcessingStage::Video,
            calls,
            false,
        ));
        let mut runtime = EngineRuntime::new();
        runtime.register_plugin(&plugin).unwrap();
        let first = runtime
            .submit(video_request(vec![processor_id.clone()]))
            .unwrap();
        let second = runtime.submit(video_request(vec![processor_id])).unwrap();

        assert_eq!(runtime.run_next().unwrap().unwrap().id(), &first);
        assert_eq!(runtime.run_next().unwrap().unwrap().id(), &second);
    }

    #[test]
    fn empty_pipeline_spec_fails_task() {
        let mut runtime = EngineRuntime::new();
        runtime.submit(video_request(Vec::new())).unwrap();

        let task = runtime.run_next().unwrap().unwrap();

        assert_eq!(task.state(), TaskState::Failed);
        assert_eq!(task.failure().unwrap().kind(), ErrorKind::Pipeline);
    }
}
