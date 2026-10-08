use std::fs;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use framelean_analysis::{
    AnalyzedMedia, AnimationInfo, AudioStreamInfo, ExpectedSourceFacts, HdrInfo, ImageInfo,
    MediaAnalysis, MediaAnalyzeRequest, MediaAnalyzer, MediaDescriptor, MediaKind, MediaSource,
    MediaStreamDescriptor, MediaWarning, SourceFingerprint, VideoStreamInfo,
};
use framelean_core::{
    BackendId, EngineError, EngineErrorCode, ErrorKind, FileSizeBytes, ObservationStatus, Observed,
    Result,
};
use framelean_decision::{
    CapabilityResolver, CapabilitySet, DefaultCapabilityResolver, DefaultRecommendationEngine,
    DeterministicSizeEstimator, EstimatorPolicy, ExecutionChainId, InputMediaRequirements,
    ManualConfigurationSelection, ManualSelection, PresetId, PresetSelection, RecalculateSelection,
    TargetSizeSelection, TaskMode,
};
use framelean_environment::{
    CpuInfo, EnvironmentSnapshot, EnvironmentSnapshotProvider, ResourceMonitor, ResourceSample,
};
use framelean_media::capability::{
    BackendAvailability, BackendCapability, BackendCatalog, BackendCatalogProvider,
    BackendDescriptor, BackendEnvironmentRequirements, CapabilityConstraint, DecoderCapability,
    DemuxerCapability, EncoderCapability, HdrMode, MuxerCapability, MuxerCodecCombination,
    NativeSupportStatus, StreamKind,
};
use framelean_runtime::{
    AnalysisServices, AnalysisSnapshotPolicy, AnalysisSnapshotRecord,
    AnalysisSnapshotValidityStatus, AnalyzeTaskRequest, EngineRuntime, EvictionStrategy,
    ExecutionOutputRequest, ExecutionSubmissionRequest, OutputCollisionPolicy,
    RecalculateConfigurationRequest, RequestContext,
};

struct TestAnalyzer;

impl MediaAnalyzer for TestAnalyzer {
    fn analyze(&self, request: &MediaAnalyzeRequest) -> Result<AnalyzedMedia> {
        let metadata = fs::metadata(request.source.path()).unwrap();
        let source_fingerprint = SourceFingerprint::from_local_file(request.source.path())?;
        let source_id = source_fingerprint.source_id()?;
        AnalyzedMedia::new(
            MediaAnalysis {
                status: framelean_analysis::MediaAnalysisStatus::Complete,
                source_id,
                file_name: "input.bin".to_owned(),
                display_path: None,
                file_size: FileSizeBytes::new(metadata.len()),
                kind: MediaKind::Video,
                format: Observed::detected("mp4".to_owned(), "test"),
                duration: Observed::detected(
                    framelean_media::MediaDuration::new(10, 1).unwrap(),
                    "fixture",
                ),
                descriptor: MediaDescriptor::Video {
                    streams: vec![MediaStreamDescriptor::Video(Box::new(VideoStreamInfo {
                        stream_index: 0,
                        codec: "h264".to_owned(),
                        profile: Observed::detected("main".to_owned(), "test"),
                        width: 1920,
                        height: 1080,
                        frame_rate: Observed::detected(
                            framelean_media::Rational::new(30, 1).unwrap(),
                            "test",
                        ),
                        frame_count: Observed::detected(300, "test"),
                        time_base: framelean_media::Rational::new(1, 90_000).unwrap(),
                        bit_depth: Observed::detected(8, "test"),
                        pixel_format: Observed::detected("yuv420p".to_owned(), "test"),
                        hdr: HdrInfo {
                            color_range: Observed::detected("tv".to_owned(), "test"),
                            color_space: Observed::detected("bt709".to_owned(), "test"),
                            color_transfer: Observed::detected("bt709".to_owned(), "test"),
                            color_primaries: Observed::detected("bt709".to_owned(), "test"),
                        },
                        bitrate: Observed::with_status(ObservationStatus::NotProbed, "test"),
                    }))],
                },
                provider: "test".to_owned(),
                provider_version: None,
                warnings: vec![MediaWarning {
                    code: EngineErrorCode::MediaBitDepthUnavailable,
                    message: "fixture media warning".to_owned(),
                }],
            },
            source_fingerprint,
        )
    }
}

struct TestSystem;

impl EnvironmentSnapshotProvider for TestSystem {
    fn snapshot(&self) -> Result<EnvironmentSnapshot> {
        Ok(EnvironmentSnapshot {
            observed_at_unix_ms: 1,
            operating_system: Observed::detected("test".to_owned(), "test"),
            os_version: Observed::detected("1".to_owned(), "test"),
            device_model: Observed::with_status(ObservationStatus::NotProbed, "test"),
            cpu: CpuInfo {
                model: Observed::detected("test".to_owned(), "test"),
                architecture: "test".to_owned(),
                physical_cores: Observed::detected(1, "test"),
                logical_cores: 1,
            },
            total_memory: framelean_core::MemoryBytes::new(1),
            gpus: Observed::detected(Vec::new(), "test"),
            native_media_frameworks: Vec::new(),
        })
    }
}

impl ResourceMonitor for TestSystem {
    fn sample(&self) -> Result<ResourceSample> {
        Ok(ResourceSample {
            sampled_at_unix_ms: 1,
            cpu_usage_basis_points: Observed::detected(0, "test"),
            used_memory: Observed::detected(framelean_core::MemoryBytes::new(0), "test"),
            gpu_usage_basis_points: Observed::with_status(ObservationStatus::Unsupported, "test"),
            used_gpu_memory: Observed::with_status(ObservationStatus::Unsupported, "test"),
            temperature_millidegrees_celsius: Observed::with_status(
                ObservationStatus::Unsupported,
                "test",
            ),
            power_milliwatts: Observed::with_status(ObservationStatus::Unsupported, "test"),
        })
    }
}

struct NativeOnlyCatalog;

impl BackendCatalogProvider for NativeOnlyCatalog {
    fn backend_catalog(&self) -> Result<BackendCatalog> {
        Ok(BackendCatalog::default())
    }
}

struct ReadyCatalog;

impl BackendCatalogProvider for ReadyCatalog {
    fn backend_catalog(&self) -> Result<BackendCatalog> {
        BackendCatalog::from_backends(vec![
            backend(
                "demux",
                BackendCapability::Demuxer(DemuxerCapability {
                    input_formats: vec!["mp4".to_owned()],
                    stream_types: vec![StreamKind::Video],
                    codec_restrictions: CapabilityConstraint::Restricted(vec!["h264".to_owned()]),
                    supports_multiple_streams: Observed::detected(true, "test"),
                    requires_seek: Observed::detected(true, "test"),
                    supports_custom_io: Observed::detected(true, "test"),
                }),
            ),
            backend(
                "decoder",
                BackendCapability::Decoder(DecoderCapability {
                    stream_type: StreamKind::Video,
                    codecs: vec!["h264".to_owned()],
                    profiles: CapabilityConstraint::Restricted(vec!["main".to_owned()]),
                    pixel_or_sample_formats: CapabilityConstraint::Restricted(vec![
                        "yuv420p".to_owned(),
                    ]),
                    bit_depths: CapabilityConstraint::Restricted(vec![8]),
                }),
            ),
            backend(
                "encoder",
                BackendCapability::Encoder(EncoderCapability {
                    stream_type: StreamKind::Video,
                    codecs: vec!["h264".to_owned()],
                    profiles: CapabilityConstraint::Restricted(vec!["main".to_owned()]),
                    pixel_or_sample_formats: CapabilityConstraint::Restricted(vec![
                        "yuv420p".to_owned(),
                    ]),
                    bit_depths: CapabilityConstraint::Restricted(vec![8]),
                    hdr_modes: CapabilityConstraint::Restricted(vec![HdrMode::Sdr]),
                    rate_control_modes: CapabilityConstraint::Restricted(vec![
                        "bitrate".to_owned(),
                    ]),
                }),
            ),
            backend(
                "muxer",
                BackendCapability::Muxer(MuxerCapability {
                    output_formats: vec!["mp4".to_owned()],
                    video_codecs: vec!["h264".to_owned()],
                    audio_codecs: Vec::new(),
                    supports_subtitles: Observed::detected(false, "test"),
                    supports_data: Observed::detected(false, "test"),
                    supports_attachments: Observed::detected(false, "test"),
                    supports_multiple_streams: Observed::detected(true, "test"),
                    codec_combinations: CapabilityConstraint::Restricted(vec![
                        MuxerCodecCombination {
                            video_codec: Some("h264".to_owned()),
                            audio_codec: None,
                        },
                    ]),
                    requires_seek: Observed::detected(true, "test"),
                }),
            ),
        ])
    }
}

struct RejectingCapabilityResolver;

impl CapabilityResolver for RejectingCapabilityResolver {
    fn resolve(
        &self,
        _requirements: &InputMediaRequirements,
        _task_mode: TaskMode,
        _environment: &EnvironmentSnapshot,
        _catalog: &BackendCatalog,
    ) -> Result<CapabilitySet> {
        Err(EngineError::new(
            ErrorKind::Capability,
            "current resolver must not be used for a frozen analysis",
        ))
    }
}

fn backend(id: &str, capability: BackendCapability) -> BackendDescriptor {
    BackendDescriptor {
        id: BackendId::new(id).unwrap(),
        provider: "test".to_owned(),
        version: Some("1".to_owned()),
        availability: BackendAvailability::execution_ready(
            NativeSupportStatus::NativeInitializable,
        ),
        environment: BackendEnvironmentRequirements::unrestricted(),
        capability,
        source: "test".to_owned(),
    }
}

fn runtime() -> EngineRuntime {
    runtime_with_provider(Arc::new(NativeOnlyCatalog))
}

fn runtime_with_provider(provider: Arc<dyn BackendCatalogProvider>) -> EngineRuntime {
    runtime_with_provider_resolver_and_policy(
        provider,
        Arc::new(DefaultCapabilityResolver),
        EstimatorPolicy::baseline(),
    )
}

fn runtime_with_provider_and_resolver(
    provider: Arc<dyn BackendCatalogProvider>,
    capability_resolver: Arc<dyn CapabilityResolver>,
) -> EngineRuntime {
    runtime_with_provider_resolver_and_policy(
        provider,
        capability_resolver,
        EstimatorPolicy::baseline(),
    )
}

fn runtime_with_provider_resolver_and_policy(
    provider: Arc<dyn BackendCatalogProvider>,
    capability_resolver: Arc<dyn CapabilityResolver>,
    estimator_policy: EstimatorPolicy,
) -> EngineRuntime {
    let system = Arc::new(TestSystem);
    EngineRuntime::with_analysis_services(
        AnalysisServices {
            analyzer: Arc::new(TestAnalyzer),
            environment: system.clone(),
            resource_monitor: system,
            native_backend_providers: vec![provider],
            capability_resolver,
            recommendation_engine: Arc::new(DefaultRecommendationEngine),
            size_estimator: Arc::new(DeterministicSizeEstimator),
            estimator_policy,
        },
        AnalysisSnapshotPolicy::new(None, None, EvictionStrategy::LeastRecentlyUsed).unwrap(),
    )
}

#[test]
fn same_runtime_recalculates_but_native_only_chain_stays_unavailable() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-analysis-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let mut runtime = runtime();
    let initial = runtime
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    assert_eq!(runtime.analysis_snapshot_count(), 1);
    assert!(!initial.capabilities.as_ref().unwrap().available);
    assert!(
        initial
            .warnings
            .iter()
            .any(|warning| { warning.code == EngineErrorCode::MediaBitDepthUnavailable })
    );
    let backend_summary = initial.engine_backend_summary.as_ref().unwrap();
    assert_eq!(backend_summary.plugin_backend_count, 0);
    assert_eq!(
        backend_summary.backend_count,
        backend_summary.native_backend_count
    );

    let recalculated = runtime
        .recalculate_configuration(RecalculateConfigurationRequest {
            analysis_id: initial.analysis_id,
            expected_revision: initial.analysis_revision,
            selection: RecalculateSelection::Preset(PresetSelection {
                preset_id: PresetId::new("balanced").unwrap(),
                candidate_id: ExecutionChainId::new("unavailable").unwrap(),
                overrides: ManualSelection::empty(),
            }),
            context: RequestContext::default(),
        })
        .unwrap();
    assert!(!recalculated.capabilities.available);
    assert_eq!(recalculated.analysis_revision, initial.analysis_revision);
    assert!(recalculated.resolved_configuration.is_none());
    assert!(
        recalculated
            .conflicts
            .iter()
            .any(|value| value.code == framelean_decision::ENGINE_EXECUTION_CHAIN_NOT_READY)
    );
    assert!(
        recalculated
            .warnings
            .iter()
            .any(|warning| { warning.code == EngineErrorCode::MediaBitDepthUnavailable })
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn same_size_content_change_invalidates_snapshot() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-source-change-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"first00").unwrap();
    let mut runtime = runtime();
    let initial = runtime
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();

    fs::write(&path, b"second0").unwrap();
    let snapshot = runtime.analysis_snapshot(&initial.analysis_id).unwrap();
    assert_eq!(
        snapshot.validity.status,
        AnalysisSnapshotValidityStatus::Invalid
    );
    assert_eq!(
        snapshot.validity.reason_code,
        Some(EngineErrorCode::AnalysisSourceChanged)
    );
    let error = runtime
        .recalculate_configuration(RecalculateConfigurationRequest {
            analysis_id: initial.analysis_id,
            expected_revision: initial.analysis_revision,
            selection: RecalculateSelection::Preset(PresetSelection {
                preset_id: PresetId::new("balanced").unwrap(),
                candidate_id: ExecutionChainId::new("unavailable").unwrap(),
                overrides: ManualSelection::empty(),
            }),
            context: RequestContext::default(),
        })
        .unwrap_err();
    assert_eq!(
        error.code(),
        framelean_core::EngineErrorCode::AnalysisSourceChanged
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn configuration_resolution_does_not_mutate_analysis_snapshot() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-resolved-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let mut runtime = runtime_with_provider(Arc::new(ReadyCatalog));
    let initial = runtime
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    assert!(initial.capabilities.as_ref().unwrap().available);
    assert!(initial.requirements.is_some());
    assert!(initial.source_fingerprint.is_some());
    assert!(initial.configuration_options.is_some());
    assert!(initial.recommendation.as_ref().unwrap().estimate.is_some());
    assert!(
        initial
            .presets
            .iter()
            .filter(|preset| preset.applicable)
            .all(|preset| {
                preset.estimate.is_some()
                    && preset.candidate.as_ref().map(|value| &value.id)
                        == preset
                            .configuration
                            .as_ref()
                            .map(|value| &value.execution_chain_id)
            })
    );
    let candidate_id = initial.capabilities.as_ref().unwrap().execution_chains[0]
        .id
        .clone();

    let resolved = runtime
        .recalculate_configuration(RecalculateConfigurationRequest {
            analysis_id: initial.analysis_id.clone(),
            expected_revision: initial.analysis_revision,
            selection: RecalculateSelection::Manual(ManualConfigurationSelection {
                candidate_id: candidate_id.clone(),
                overrides: ManualSelection {
                    container: Some("mp4".to_owned()),
                    video_codec: Some("h264".to_owned()),
                    audio_codec: None,
                    audio_streams: None,
                    output_pixel_format: Some("yuv420p".to_owned()),
                    preserves_hdr: Some(false),
                },
            }),
            context: RequestContext::default(),
        })
        .unwrap();
    assert!(resolved.resolved_configuration.is_some());
    assert_eq!(resolved.analysis_revision, initial.analysis_revision);

    let snapshot = runtime.analysis_snapshot(&initial.analysis_id).unwrap();
    assert_eq!(snapshot.analysis_revision, initial.analysis_revision);
    assert_eq!(snapshot.requirements, initial.requirements.clone().unwrap());
    assert_eq!(
        snapshot.source_fingerprint,
        initial.source_fingerprint.clone().unwrap()
    );
    assert_eq!(
        snapshot.configuration_options,
        initial.configuration_options.clone().unwrap()
    );
    assert_eq!(
        snapshot.recommendation,
        initial.recommendation.clone().unwrap()
    );
    assert_eq!(snapshot.presets, initial.presets);
    assert_eq!(
        snapshot.validity.status,
        AnalysisSnapshotValidityStatus::Valid
    );

    let rejected = runtime
        .recalculate_configuration(RecalculateConfigurationRequest {
            analysis_id: initial.analysis_id,
            expected_revision: initial.analysis_revision,
            selection: RecalculateSelection::Manual(ManualConfigurationSelection {
                candidate_id,
                overrides: ManualSelection {
                    container: Some("mp4".to_owned()),
                    video_codec: Some("hevc".to_owned()),
                    audio_codec: None,
                    audio_streams: None,
                    output_pixel_format: None,
                    preserves_hdr: None,
                },
            }),
            context: RequestContext::default(),
        })
        .unwrap();
    assert!(rejected.resolved_configuration.is_none());
    assert_eq!(rejected.analysis_revision, initial.analysis_revision);
    assert_eq!(
        rejected.conflicts[0].code,
        framelean_core::EngineErrorCode::MediaCapabilityIncompatible
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn execution_submission_rejects_an_unimplemented_media_pipeline_without_output_side_effects() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-execution-source-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let output_path = path.with_extension("output.mp4");
    fs::write(&path, b"fixture").unwrap();
    let mut runtime = runtime_with_provider(Arc::new(ReadyCatalog));
    let initial = runtime
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    let candidate_id = initial.capabilities.as_ref().unwrap().execution_chains[0]
        .id
        .clone();

    let error = runtime
        .submit_execution(ExecutionSubmissionRequest {
            analysis_id: initial.analysis_id,
            expected_revision: initial.analysis_revision,
            selection: RecalculateSelection::Manual(ManualConfigurationSelection {
                candidate_id,
                overrides: ManualSelection::empty(),
            }),
            output: ExecutionOutputRequest {
                requested_path: output_path.clone(),
                collision_policy: OutputCollisionPolicy::FailIfExists,
            },
            context: RequestContext::default(),
        })
        .unwrap_err();

    assert_eq!(error.code(), EngineErrorCode::EngineExecutionChainNotReady);
    assert!(!output_path.exists());
    assert_eq!(runtime.queued_task_count(), 0);
    fs::remove_file(path).unwrap();
}

#[test]
fn snapshot_record_round_trip_restores_configuration_state() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-persisted-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let mut original = runtime_with_provider(Arc::new(ReadyCatalog));
    let initial = original
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    let record = original
        .analysis_snapshot_record(&initial.analysis_id)
        .unwrap();
    let record_json = serde_json::to_value(&record).unwrap();
    assert_eq!(record_json["decision_model_revision"], 1);
    assert_eq!(record_json["estimator_model_revision"], 1);
    let encoded = serde_json::to_vec(&record).unwrap();
    let decoded: AnalysisSnapshotRecord = serde_json::from_slice(&encoded).unwrap();

    let mut restored = runtime_with_provider(Arc::new(ReadyCatalog));
    restored.restore_analysis_snapshot(decoded).unwrap();
    let view = restored.analysis_snapshot(&initial.analysis_id).unwrap();
    let view_json = serde_json::to_value(&view).unwrap();
    assert_eq!(view_json["decision_model_revision"], 1);
    assert_eq!(view_json["estimator_model_revision"], 1);
    assert_eq!(view.analysis_revision, initial.analysis_revision);
    assert_eq!(view.presets, initial.presets);
    assert_eq!(view.validity.status, AnalysisSnapshotValidityStatus::Valid);

    let candidate_id = view.capabilities.execution_chains[0].id.clone();
    let resolved = restored
        .recalculate_configuration(RecalculateConfigurationRequest {
            analysis_id: initial.analysis_id.clone(),
            expected_revision: initial.analysis_revision,
            selection: RecalculateSelection::Preset(PresetSelection {
                preset_id: PresetId::new("balanced").unwrap(),
                candidate_id,
                overrides: ManualSelection::empty(),
            }),
            context: RequestContext::default(),
        })
        .unwrap();
    assert!(resolved.resolved_configuration.is_some());

    let next = restored
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    assert_ne!(next.analysis_id, initial.analysis_id);
    fs::remove_file(path).unwrap();
}

#[test]
fn restored_snapshot_recalculates_from_frozen_decision_state() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-frozen-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let mut original = runtime_with_provider(Arc::new(ReadyCatalog));
    let initial = original
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    let record = original
        .analysis_snapshot_record(&initial.analysis_id)
        .unwrap();
    let mut restored = runtime_with_provider_and_resolver(
        Arc::new(NativeOnlyCatalog),
        Arc::new(RejectingCapabilityResolver),
    );
    restored.restore_analysis_snapshot(record).unwrap();
    let candidate_id = initial.capabilities.as_ref().unwrap().execution_chains[0]
        .id
        .clone();

    let recalculated = restored
        .recalculate_configuration(RecalculateConfigurationRequest {
            analysis_id: initial.analysis_id,
            expected_revision: initial.analysis_revision,
            selection: RecalculateSelection::Manual(ManualConfigurationSelection {
                candidate_id,
                overrides: ManualSelection::empty(),
            }),
            context: RequestContext::default(),
        })
        .unwrap();

    assert_eq!(
        recalculated.capabilities,
        initial.capabilities.clone().unwrap()
    );
    assert_eq!(
        recalculated.configuration_options,
        initial.configuration_options.clone().unwrap()
    );
    assert_eq!(
        recalculated.recommendation,
        initial.recommendation.clone().unwrap()
    );
    assert_eq!(recalculated.presets, initial.presets);
    assert!(recalculated.resolved_configuration.is_some());
    fs::remove_file(path).unwrap();
}

#[test]
fn restored_snapshot_uses_its_frozen_preset_definition() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-frozen-preset-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let mut original = runtime_with_provider(Arc::new(ReadyCatalog));
    let initial = original
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    let mut record = original
        .analysis_snapshot_record(&initial.analysis_id)
        .unwrap();
    let preset = record
        .presets
        .iter_mut()
        .find(|preset| preset.id.as_str() == "balanced")
        .unwrap();
    for ceiling in &mut preset.policy.video_bitrate_ceilings {
        ceiling.h264_bps = 123_456;
    }
    let configuration = preset.configuration.as_mut().unwrap();
    configuration.target_video_bitrate = Some(framelean_core::BitRateBps::new(123_456));
    let estimate = preset.estimate.as_mut().unwrap();
    estimate.expected_bytes = 154_320;
    estimate.recommended_video_bitrate = Some(framelean_core::BitRateBps::new(123_456));
    let candidate_id = preset.candidate.as_ref().unwrap().id.clone();

    let mut restored = runtime_with_provider(Arc::new(NativeOnlyCatalog));
    restored.restore_analysis_snapshot(record).unwrap();
    let recalculated = restored
        .recalculate_configuration(RecalculateConfigurationRequest {
            analysis_id: initial.analysis_id,
            expected_revision: initial.analysis_revision,
            selection: RecalculateSelection::Preset(PresetSelection {
                preset_id: PresetId::new("balanced").unwrap(),
                candidate_id,
                overrides: ManualSelection::empty(),
            }),
            context: RequestContext::default(),
        })
        .unwrap();

    assert_eq!(
        recalculated
            .resolved_configuration
            .unwrap()
            .target_video_bitrate,
        Some(framelean_core::BitRateBps::new(123_456))
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn restored_snapshot_uses_its_frozen_estimator_policy() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-frozen-estimator-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, vec![0_u8; 100_000]).unwrap();
    let calibrated_policy = |overhead| EstimatorPolicy {
        calibrated_container_overhead_bytes: Some(overhead),
        calibration_sample_count: 10,
        ..EstimatorPolicy::baseline()
    };
    let mut original = runtime_with_provider_resolver_and_policy(
        Arc::new(ReadyCatalog),
        Arc::new(DefaultCapabilityResolver),
        calibrated_policy(1_000),
    );
    let initial = original
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    let record = original
        .analysis_snapshot_record(&initial.analysis_id)
        .unwrap();
    let candidate_id = initial.capabilities.as_ref().unwrap().execution_chains[0]
        .id
        .clone();
    let mut restored = runtime_with_provider_resolver_and_policy(
        Arc::new(NativeOnlyCatalog),
        Arc::new(DefaultCapabilityResolver),
        calibrated_policy(9_000),
    );
    restored.restore_analysis_snapshot(record).unwrap();

    let recalculated = restored
        .recalculate_configuration(RecalculateConfigurationRequest {
            analysis_id: initial.analysis_id,
            expected_revision: initial.analysis_revision,
            selection: RecalculateSelection::CustomTargetSize(TargetSizeSelection {
                candidate_id,
                target_bytes: 100_000,
                allow_resolution_change: false,
                allow_frame_rate_change: false,
            }),
            context: RequestContext::default(),
        })
        .unwrap();

    assert_eq!(
        recalculated
            .resolved_configuration
            .unwrap()
            .target_size
            .unwrap()
            .total_bitrate
            .value(),
        79_200
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn restoring_the_same_analysis_id_twice_is_rejected_without_overwriting() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-duplicate-restore-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let mut original = runtime_with_provider(Arc::new(ReadyCatalog));
    let initial = original
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    let record = original
        .analysis_snapshot_record(&initial.analysis_id)
        .unwrap();

    let mut restored = runtime_with_provider(Arc::new(ReadyCatalog));
    restored.restore_analysis_snapshot(record.clone()).unwrap();
    let error = restored.restore_analysis_snapshot(record).unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Snapshot);
    assert!(error.message().contains("already"));
    assert_eq!(restored.analysis_snapshot_count(), 1);
    fs::remove_file(path).unwrap();
}

#[test]
fn restoring_a_different_revision_for_the_same_analysis_id_is_rejected() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-revision-restore-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let mut original = runtime_with_provider(Arc::new(ReadyCatalog));
    let initial = original
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    let record = original
        .analysis_snapshot_record(&initial.analysis_id)
        .unwrap();
    let mut conflicting = record.clone();
    conflicting.revision = conflicting.revision.next().unwrap();

    let mut restored = runtime_with_provider(Arc::new(ReadyCatalog));
    restored.restore_analysis_snapshot(record).unwrap();
    let error = restored.restore_analysis_snapshot(conflicting).unwrap_err();

    assert_eq!(error.code(), EngineErrorCode::AnalysisRevisionConflict);
    assert_eq!(restored.analysis_snapshot_count(), 1);
    let view = restored.analysis_snapshot(&initial.analysis_id).unwrap();
    assert_eq!(view.analysis_revision, initial.analysis_revision);
    fs::remove_file(path).unwrap();
}

#[test]
fn incompatible_snapshot_decision_model_revision_rejects_configuration_recalculation() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-decision-revision-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let mut original = runtime_with_provider(Arc::new(ReadyCatalog));
    let initial = original
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    let record = original
        .analysis_snapshot_record(&initial.analysis_id)
        .unwrap();
    let candidate_id = initial.capabilities.as_ref().unwrap().execution_chains[0]
        .id
        .clone();
    let mut record_json = serde_json::to_value(record).unwrap();
    record_json["decision_model_revision"] = serde_json::json!(2);
    let incompatible: AnalysisSnapshotRecord = serde_json::from_value(record_json).unwrap();

    let mut restored = runtime_with_provider(Arc::new(ReadyCatalog));
    restored.restore_analysis_snapshot(incompatible).unwrap();
    let error = restored
        .recalculate_configuration(RecalculateConfigurationRequest {
            analysis_id: initial.analysis_id,
            expected_revision: initial.analysis_revision,
            selection: RecalculateSelection::Manual(ManualConfigurationSelection {
                candidate_id,
                overrides: ManualSelection::empty(),
            }),
            context: RequestContext::default(),
        })
        .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Snapshot);
    assert!(error.message().contains("decision model revision"));
    fs::remove_file(path).unwrap();
}

#[test]
fn incompatible_snapshot_estimator_model_revision_rejects_configuration_recalculation() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-estimator-revision-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let mut original = runtime_with_provider(Arc::new(ReadyCatalog));
    let initial = original
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    let record = original
        .analysis_snapshot_record(&initial.analysis_id)
        .unwrap();
    let candidate_id = initial.capabilities.as_ref().unwrap().execution_chains[0]
        .id
        .clone();
    let mut record_json = serde_json::to_value(record).unwrap();
    record_json["estimator_model_revision"] = serde_json::json!(2);
    let incompatible: AnalysisSnapshotRecord = serde_json::from_value(record_json).unwrap();

    let mut restored = runtime_with_provider(Arc::new(ReadyCatalog));
    restored.restore_analysis_snapshot(incompatible).unwrap();
    let error = restored
        .recalculate_configuration(RecalculateConfigurationRequest {
            analysis_id: initial.analysis_id,
            expected_revision: initial.analysis_revision,
            selection: RecalculateSelection::Manual(ManualConfigurationSelection {
                candidate_id,
                overrides: ManualSelection::empty(),
            }),
            context: RequestContext::default(),
        })
        .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Snapshot);
    assert!(error.message().contains("estimator model revision"));
    fs::remove_file(path).unwrap();
}

#[test]
fn independent_runtimes_generate_distinct_analysis_ids() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-distinct-ids-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let request = || AnalyzeTaskRequest {
        task_mode: TaskMode::VideoCompress,
        media_request: MediaAnalyzeRequest {
            source: MediaSource::local_file(&path).unwrap(),
            request_id: None,
            expected_source: None,
        },
        context: RequestContext::default(),
    };
    let first = runtime_with_provider(Arc::new(ReadyCatalog))
        .analyze_media(request())
        .unwrap();
    let second = runtime_with_provider(Arc::new(ReadyCatalog))
        .analyze_media(request())
        .unwrap();

    assert_ne!(first.analysis_id, second.analysis_id);
    fs::remove_file(path).unwrap();
}

#[test]
fn expected_source_facts_reject_a_file_replaced_before_analysis() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-expected-source-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let mut runtime = runtime_with_provider(Arc::new(ReadyCatalog));

    let response = runtime
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: Some(ExpectedSourceFacts {
                    file_size_bytes: 1,
                    modified_time_unix_nanos: None,
                }),
            },
            context: RequestContext::default(),
        })
        .unwrap();

    assert_eq!(
        response.media_analysis_status,
        framelean_analysis::MediaAnalysisStatus::Failed
    );
    assert_eq!(
        response.error.unwrap().code,
        EngineErrorCode::AnalysisSourceChanged
    );
    assert_eq!(runtime.analysis_snapshot_count(), 0);
    fs::remove_file(path).unwrap();
}

#[test]
fn analysis_snapshot_can_be_discarded_when_external_persistence_fails() {
    let path = std::env::temp_dir().join(format!(
        "framelean-runtime-discard-snapshot-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, b"fixture").unwrap();
    let mut runtime = runtime_with_provider(Arc::new(ReadyCatalog));
    let response = runtime
        .analyze_media(AnalyzeTaskRequest {
            task_mode: TaskMode::VideoCompress,
            media_request: MediaAnalyzeRequest {
                source: MediaSource::local_file(&path).unwrap(),
                request_id: None,
                expected_source: None,
            },
            context: RequestContext::default(),
        })
        .unwrap();
    assert_eq!(runtime.analysis_snapshot_count(), 1);

    assert!(
        runtime
            .discard_analysis_snapshot(&response.analysis_id)
            .unwrap()
    );

    assert_eq!(runtime.analysis_snapshot_count(), 0);
    assert!(runtime.analysis_snapshot(&response.analysis_id).is_err());
    fs::remove_file(path).unwrap();
}

#[test]
fn aggregate_media_variants_serialize_for_client_responses() {
    let image = ImageInfo {
        codec: "png".to_owned(),
        width: 1,
        height: 1,
        pixel_format: Observed::detected("rgba".to_owned(), "test"),
        bit_depth: Observed::detected(8, "test"),
        alpha: Observed::detected(true, "test"),
        color_space: Observed::detected("srgb".to_owned(), "test"),
    };
    let audio = || {
        MediaStreamDescriptor::Audio(Box::new(AudioStreamInfo {
            stream_index: 1,
            codec: "aac".to_owned(),
            profile: Observed::detected("lc".to_owned(), "test"),
            sample_rate_hz: Observed::detected(48_000, "test"),
            channel_count: Observed::detected(2, "test"),
            channel_layout: Observed::detected("stereo".to_owned(), "test"),
            sample_format: Observed::detected("fltp".to_owned(), "test"),
            bitrate: Observed::detected(framelean_core::BitRateBps::new(128_000), "test"),
            duration: Observed::detected(
                framelean_media::MediaDuration::new(10, 1).unwrap(),
                "test",
            ),
        }))
    };
    let video = || {
        MediaStreamDescriptor::Video(Box::new(VideoStreamInfo {
            stream_index: 0,
            codec: "h264".to_owned(),
            profile: Observed::detected("main".to_owned(), "test"),
            width: 1920,
            height: 1080,
            frame_rate: Observed::detected(framelean_media::Rational::new(30, 1).unwrap(), "test"),
            frame_count: Observed::detected(300, "test"),
            time_base: framelean_media::Rational::new(1, 90_000).unwrap(),
            bit_depth: Observed::detected(8, "test"),
            pixel_format: Observed::detected("yuv420p".to_owned(), "test"),
            hdr: HdrInfo {
                color_range: Observed::detected("tv".to_owned(), "test"),
                color_space: Observed::detected("bt709".to_owned(), "test"),
                color_transfer: Observed::detected("bt709".to_owned(), "test"),
                color_primaries: Observed::detected("bt709".to_owned(), "test"),
            },
            bitrate: Observed::detected(framelean_core::BitRateBps::new(5_000_000), "test"),
        }))
    };

    let descriptors = vec![
        MediaDescriptor::Image {
            image: Box::new(image.clone()),
        },
        MediaDescriptor::AnimatedImage {
            image: Box::new(image),
            animation: Box::new(AnimationInfo {
                frame_rate: Observed::detected(
                    framelean_media::Rational::new(10, 1).unwrap(),
                    "test",
                ),
                frame_count: Observed::detected(10, "test"),
                duration: Observed::detected(
                    framelean_media::MediaDuration::new(1, 1).unwrap(),
                    "test",
                ),
            }),
        },
        MediaDescriptor::Audio {
            streams: vec![audio()],
        },
        MediaDescriptor::Video {
            streams: vec![video()],
        },
        MediaDescriptor::Video {
            streams: vec![video(), audio(), audio()],
        },
    ];
    for descriptor in descriptors {
        let json = serde_json::to_value(&descriptor).unwrap();
        assert!(json.get("type").is_some());
    }
}
