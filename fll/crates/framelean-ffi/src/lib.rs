//! Stable C ABI facade for the FLL runtime.
//!
//! FEngine will load this crate as a dynamic library. Only the bootstrap symbol is exported;
//! all other operations are reached through the versioned function table and exchange bounded,
//! versioned JSON documents. Rust values, references, trait objects, and allocator-owned strings
//! never cross the ABI boundary.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::mem::{self, size_of};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;
use std::sync::{Arc, Mutex};

use framelean_analysis::MediaAnalyzeRequest;
use framelean_core::{EngineError, EngineErrorCode, ErrorKind, Result, TaskId};
use framelean_decision::TaskMode;
use framelean_environment::SystemEnvironment;
use framelean_ffmpeg::{
    FfmpegAdapter, PreviewFramesRequest, PreviewFramesResult, VideoThumbnailRequest,
    VideoThumbnailResult,
};
use framelean_runtime::{
    AnalysisServices, AnalysisSnapshotPolicy, AnalysisSnapshotRecord, AnalyzeTaskRequest,
    DeterministicSizeEstimator, EngineRuntime, EstimatorPolicy, EvictionStrategy,
    ExecutionRuntimeEvent, ExecutionServices, ExecutionSubmissionRequest,
    RecalculateConfigurationRequest, RequestContext,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The major ABI version implemented by this dynamic library.
pub const FRAMELEAN_FLL_ABI_MAJOR: u16 = 1;
/// The minor ABI version implemented by this dynamic library.
pub const FRAMELEAN_FLL_ABI_MINOR: u16 = 0;
/// The version of the JSON document envelope exchanged by the facade.
pub const FRAMELEAN_FLL_DOCUMENT_VERSION: u16 = 1;

/// Successful invocation.
pub const FRAMELEAN_FLL_OK: i32 = 0;
/// Input pointer, length, version, or JSON payload is invalid.
pub const FRAMELEAN_FLL_INVALID_ARGUMENT: i32 = 1;
/// FLL returned a domain/runtime error. The output buffer contains its structured error.
pub const FRAMELEAN_FLL_RUNTIME_ERROR: i32 = 2;
/// A panic was caught at the ABI boundary.
pub const FRAMELEAN_FLL_PANIC: i32 = 3;
/// The output buffer could not be returned to the caller.
pub const FRAMELEAN_FLL_BUFFER_ERROR: i32 = 4;
/// No execution event is currently available.
pub const FRAMELEAN_FLL_NO_EVENT: i32 = 5;
/// The requested ABI major/minor cannot be served by this library.
pub const FRAMELEAN_FLL_ABI_MISMATCH: i32 = 6;

const MAX_COMMAND_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_SNAPSHOT_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_BUFFER_BYTES: usize = MAX_SNAPSHOT_DOCUMENT_BYTES;

pub type FrameLeanFllRuntimeCreate = unsafe extern "C" fn(
    out_handle: *mut *mut c_void,
    out_error: *mut *mut u8,
    out_error_len: *mut usize,
) -> i32;
pub type FrameLeanFllRuntimeDestroy = unsafe extern "C" fn(handle: *mut c_void) -> i32;
pub type FrameLeanFllInvoke = unsafe extern "C" fn(
    handle: *mut c_void,
    request: *const u8,
    request_len: usize,
    out_response: *mut *mut u8,
    out_response_len: *mut usize,
) -> i32;
pub type FrameLeanFllPollEvent = unsafe extern "C" fn(
    handle: *mut c_void,
    out_event: *mut *mut u8,
    out_event_len: *mut usize,
) -> i32;
pub type FrameLeanFllBufferFree = unsafe extern "C" fn(buffer: *mut u8, buffer_len: usize);
pub type FrameLeanFllGetApi = unsafe extern "C" fn(
    requested_major: u16,
    requested_minor: u16,
    out_api: *mut *const FrameLeanFllApiV1,
) -> i32;

/// Versioned C ABI table returned by [`framelean_fll_get_api`].
///
/// New function pointers must be appended after the existing fields. Callers must validate
/// `struct_size` before reading fields beyond the size they understand, and must reject an
/// incompatible major version. A caller requesting an older minor version can use the prefix it
/// understands.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FrameLeanFllApiV1 {
    pub struct_size: u32,
    pub abi_major: u16,
    pub abi_minor: u16,
    pub runtime_create: FrameLeanFllRuntimeCreate,
    pub runtime_destroy: FrameLeanFllRuntimeDestroy,
    pub invoke: FrameLeanFllInvoke,
    pub poll_event: FrameLeanFllPollEvent,
    pub buffer_free: FrameLeanFllBufferFree,
}

struct RuntimeHandle {
    state: Mutex<RuntimeState>,
}

struct RuntimeState {
    runtime: EngineRuntime,
    adapter: Arc<FfmpegAdapter>,
    pending_events: VecDeque<ExecutionRuntimeEvent>,
}

#[derive(Debug, Deserialize)]
struct RequestDocument {
    document_version: u16,
    operation: String,
    #[serde(default)]
    payload: Value,
}

#[derive(Debug, Deserialize)]
struct AnalyzeMediaDocument {
    task_mode: TaskMode,
    media_request: MediaAnalyzeRequest,
    #[serde(default)]
    context: RequestContext,
}

#[derive(Debug, Deserialize)]
struct AnalysisIdDocument {
    analysis_id: String,
}

#[derive(Debug, Deserialize)]
struct ExecutionIdDocument {
    execution_id: String,
}

#[derive(Debug, Deserialize)]
struct ReorderExecutionsDocument {
    expected_revision: u64,
    ordered_execution_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PreviewFramesDocument {
    input_path: String,
    output_directory: String,
    timestamps_us: Vec<u64>,
    max_width: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct VideoThumbnailDocument {
    input_path: String,
    output_path: String,
    duration_us: Option<u64>,
    max_width: u32,
}

#[derive(Debug, Serialize)]
struct ResponseDocument {
    document_version: u16,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<DispatchError>,
}

#[derive(Debug, Serialize)]
struct DispatchError {
    kind: ErrorKind,
    code: EngineErrorCode,
    message: String,
    retryable: bool,
}

#[derive(Debug, Serialize)]
struct PreviewFramesResultDocument {
    output_directory: String,
    frames: Vec<PreviewFrameDocument>,
}

#[derive(Debug, Serialize)]
struct PreviewFrameDocument {
    index: usize,
    requested_timestamp_us: u64,
    decoded_timestamp_us: u64,
    width: u32,
    height: u32,
    output_path: String,
}

#[derive(Debug, Serialize)]
struct VideoThumbnailResultDocument {
    output_path: String,
    requested_timestamp_us: u64,
    decoded_timestamp_us: u64,
    width: u32,
    height: u32,
}

#[derive(Debug, Serialize)]
struct ExecutionEventDocument {
    execution_id: String,
    resource_pool: framelean_runtime::ExecutionResourcePool,
    sequence: u64,
    state: framelean_runtime::ExecutionTaskState,
    pause_reason: Option<framelean_runtime::ExecutionPauseReason>,
    preempted_by_execution_id: Option<String>,
    resume_depth: usize,
    progress: Option<framelean_runtime::ExecutionProgress>,
    output_path: Option<String>,
    error_code: Option<EngineErrorCode>,
    message: Option<String>,
}

impl ResponseDocument {
    fn success(value: Value) -> Self {
        Self {
            document_version: FRAMELEAN_FLL_DOCUMENT_VERSION,
            ok: true,
            result: Some(value),
            error: None,
        }
    }

    fn failure(error: &EngineError) -> Self {
        Self {
            document_version: FRAMELEAN_FLL_DOCUMENT_VERSION,
            ok: false,
            result: None,
            error: Some(DispatchError {
                kind: error.kind(),
                code: error.code(),
                message: error.message().to_owned(),
                retryable: error.code().is_retryable(),
            }),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self {
            document_version: FRAMELEAN_FLL_DOCUMENT_VERSION,
            ok: false,
            result: None,
            error: Some(DispatchError {
                kind: ErrorKind::InvalidArgument,
                code: EngineErrorCode::InvalidArgument,
                message: message.into(),
                retryable: false,
            }),
        }
    }

    fn panic() -> Self {
        Self {
            document_version: FRAMELEAN_FLL_DOCUMENT_VERSION,
            ok: false,
            result: None,
            error: Some(DispatchError {
                kind: ErrorKind::Runtime,
                code: EngineErrorCode::InternalRuntimeError,
                message: "FLL ABI call panicked".to_owned(),
                retryable: false,
            }),
        }
    }
}

fn build_default_runtime() -> Result<(EngineRuntime, Arc<FfmpegAdapter>)> {
    let adapter = Arc::new(FfmpegAdapter::new()?);
    let execution_services = ExecutionServices::ffmpeg(adapter.clone());
    let system = Arc::new(SystemEnvironment::new());
    let services = AnalysisServices {
        analyzer: adapter.clone(),
        environment: system.clone(),
        resource_monitor: system,
        native_backend_providers: vec![adapter.clone()],
        capability_resolver: Arc::new(framelean_runtime::DefaultCapabilityResolver),
        recommendation_engine: Arc::new(framelean_runtime::DefaultRecommendationEngine),
        size_estimator: Arc::new(DeterministicSizeEstimator),
        estimator_policy: EstimatorPolicy::baseline(),
    };
    let policy = AnalysisSnapshotPolicy::new(None, None, EvictionStrategy::LeastRecentlyUsed)?;
    let runtime =
        EngineRuntime::with_analysis_and_execution_services(services, policy, execution_services);
    Ok((runtime, adapter))
}

fn reset_output(out_ptr: *mut *mut u8, out_len: *mut usize) -> bool {
    if out_ptr.is_null() || out_len.is_null() {
        return false;
    }
    // SAFETY: both output locations were checked for null and are caller-owned locations.
    unsafe {
        ptr::write(out_ptr, ptr::null_mut());
        ptr::write(out_len, 0);
    }
    true
}

fn write_bytes(
    bytes: Vec<u8>,
    max_bytes: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if !reset_output(out_ptr, out_len) {
        return FRAMELEAN_FLL_BUFFER_ERROR;
    }
    if bytes.is_empty() || bytes.len() > max_bytes || bytes.len() > MAX_BUFFER_BYTES {
        return FRAMELEAN_FLL_BUFFER_ERROR;
    }
    let mut bytes = bytes.into_boxed_slice();
    let pointer = bytes.as_mut_ptr();
    let length = bytes.len();
    mem::forget(bytes);
    // SAFETY: pointers were checked above and receive the allocation transferred to the caller.
    unsafe {
        ptr::write(out_ptr, pointer);
        ptr::write(out_len, length);
    }
    FRAMELEAN_FLL_OK
}

fn write_response(
    response: &ResponseDocument,
    max_bytes: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    match serde_json::to_vec(response) {
        Ok(bytes) => write_bytes(bytes, max_bytes, out_ptr, out_len),
        Err(_) => FRAMELEAN_FLL_BUFFER_ERROR,
    }
}

fn guarded_response<F>(out_ptr: *mut *mut u8, out_len: *mut usize, operation: F) -> i32
where
    F: FnOnce() -> (i32, ResponseDocument, usize) + panic::UnwindSafe,
{
    if !reset_output(out_ptr, out_len) {
        return FRAMELEAN_FLL_BUFFER_ERROR;
    }
    match panic::catch_unwind(AssertUnwindSafe(operation)) {
        Ok((status, response, max_bytes)) => {
            let write_status = write_response(&response, max_bytes, out_ptr, out_len);
            if write_status == FRAMELEAN_FLL_OK {
                status
            } else {
                write_status
            }
        }
        Err(_) => {
            let write_status = write_response(
                &ResponseDocument::panic(),
                MAX_COMMAND_DOCUMENT_BYTES,
                out_ptr,
                out_len,
            );
            if write_status == FRAMELEAN_FLL_OK {
                FRAMELEAN_FLL_PANIC
            } else {
                write_status
            }
        }
    }
}

fn parse_request(
    request: *const u8,
    request_len: usize,
) -> std::result::Result<RequestDocument, ResponseDocument> {
    if request.is_null() {
        return Err(ResponseDocument::invalid("request pointer is null"));
    }
    if request_len == 0 || request_len > MAX_SNAPSHOT_DOCUMENT_BYTES {
        return Err(ResponseDocument::invalid(
            "request length is outside the document limit",
        ));
    }
    // SAFETY: the caller promises a readable buffer of request_len bytes; the length is bounded.
    let bytes = unsafe { std::slice::from_raw_parts(request, request_len) };
    let document: RequestDocument = serde_json::from_slice(bytes)
        .map_err(|error| ResponseDocument::invalid(format!("request JSON is invalid: {error}")))?;
    if document.document_version != FRAMELEAN_FLL_DOCUMENT_VERSION {
        return Err(ResponseDocument::invalid(format!(
            "unsupported FLL document version {}; expected {}",
            document.document_version, FRAMELEAN_FLL_DOCUMENT_VERSION
        )));
    }
    if document.operation.trim().is_empty() {
        return Err(ResponseDocument::invalid(
            "request operation must not be empty",
        ));
    }
    if document.operation != "restore_analysis_snapshot" && request_len > MAX_COMMAND_DOCUMENT_BYTES
    {
        return Err(ResponseDocument::invalid(
            "request length is outside the command document limit",
        ));
    }
    Ok(document)
}

fn parse_payload<T: DeserializeOwned>(
    payload: Value,
    operation: &str,
) -> std::result::Result<T, (i32, ResponseDocument, usize)> {
    serde_json::from_value(payload).map_err(|error| {
        (
            FRAMELEAN_FLL_INVALID_ARGUMENT,
            ResponseDocument::invalid(format!("{operation} payload is invalid: {error}")),
            MAX_COMMAND_DOCUMENT_BYTES,
        )
    })
}

fn parse_id(
    value: String,
    _field: &str,
) -> std::result::Result<TaskId, (i32, ResponseDocument, usize)> {
    TaskId::new(value).map_err(|error| {
        (
            FRAMELEAN_FLL_INVALID_ARGUMENT,
            ResponseDocument::failure(&error),
            MAX_COMMAND_DOCUMENT_BYTES,
        )
    })
}

fn parse_path(
    value: String,
    field: &str,
) -> std::result::Result<PathBuf, (i32, ResponseDocument, usize)> {
    if value.trim().is_empty() {
        return Err((
            FRAMELEAN_FLL_INVALID_ARGUMENT,
            ResponseDocument::invalid(format!("{field} must not be empty")),
            MAX_COMMAND_DOCUMENT_BYTES,
        ));
    }
    Ok(PathBuf::from(value))
}

fn dispatch(state: &mut RuntimeState, request: RequestDocument) -> (i32, ResponseDocument, usize) {
    match request.operation.as_str() {
        "ping" => (
            FRAMELEAN_FLL_OK,
            ResponseDocument::success(json!({
                "service": "framelean-fll-runtime",
                "abi_major": FRAMELEAN_FLL_ABI_MAJOR,
                "abi_minor": FRAMELEAN_FLL_ABI_MINOR,
            })),
            MAX_COMMAND_DOCUMENT_BYTES,
        ),
        "analyze_media" => {
            let input =
                match parse_payload::<AnalyzeMediaDocument>(request.payload, "analyze_media") {
                    Ok(input) => input,
                    Err(result) => return result,
                };
            let result = state.runtime.analyze_media(AnalyzeTaskRequest {
                task_mode: input.task_mode,
                media_request: input.media_request,
                context: input.context,
            });
            runtime_result(result)
        }
        "analysis_snapshot" => {
            let input =
                match parse_payload::<AnalysisIdDocument>(request.payload, "analysis_snapshot") {
                    Ok(input) => input,
                    Err(result) => return result,
                };
            let analysis_id = match framelean_core::AnalysisId::new(input.analysis_id) {
                Ok(id) => id,
                Err(error) => return invalid_error(error),
            };
            runtime_result(state.runtime.analysis_snapshot(&analysis_id))
        }
        "export_analysis_snapshot" => {
            let input = match parse_payload::<AnalysisIdDocument>(
                request.payload,
                "export_analysis_snapshot",
            ) {
                Ok(input) => input,
                Err(result) => return result,
            };
            let analysis_id = match framelean_core::AnalysisId::new(input.analysis_id) {
                Ok(id) => id,
                Err(error) => return invalid_error(error),
            };
            runtime_result_with_limit(
                state.runtime.analysis_snapshot_record(&analysis_id),
                MAX_SNAPSHOT_DOCUMENT_BYTES,
            )
        }
        "restore_analysis_snapshot" => {
            let record = match parse_payload::<AnalysisSnapshotRecord>(
                request.payload,
                "restore_analysis_snapshot",
            ) {
                Ok(record) => record,
                Err(result) => return result,
            };
            runtime_result(state.runtime.restore_analysis_snapshot(record))
        }
        "discard_analysis_snapshot" => {
            let input = match parse_payload::<AnalysisIdDocument>(
                request.payload,
                "discard_analysis_snapshot",
            ) {
                Ok(input) => input,
                Err(result) => return result,
            };
            let analysis_id = match framelean_core::AnalysisId::new(input.analysis_id) {
                Ok(id) => id,
                Err(error) => return invalid_error(error),
            };
            runtime_result(state.runtime.discard_analysis_snapshot(&analysis_id))
        }
        "recalculate_configuration" => {
            let input = match parse_payload::<RecalculateConfigurationRequest>(
                request.payload,
                "recalculate_configuration",
            ) {
                Ok(input) => input,
                Err(result) => return result,
            };
            runtime_result(state.runtime.recalculate_configuration(input))
        }
        "submit_execution" => {
            let input = match parse_payload::<ExecutionSubmissionRequest>(
                request.payload,
                "submit_execution",
            ) {
                Ok(input) => input,
                Err(result) => return result,
            };
            runtime_result(state.runtime.submit_execution(input))
        }
        "execution_snapshot" => runtime_result(state.runtime.execution_snapshot()),
        "reorder_waiting_executions" => {
            let input = match parse_payload::<ReorderExecutionsDocument>(
                request.payload,
                "reorder_waiting_executions",
            ) {
                Ok(input) => input,
                Err(result) => return result,
            };
            let ordered_ids = match input
                .ordered_execution_ids
                .into_iter()
                .map(|value| parse_id(value, "ordered_execution_ids"))
                .collect::<std::result::Result<Vec<_>, _>>()
            {
                Ok(ids) => ids,
                Err(result) => return result,
            };
            runtime_result(
                state
                    .runtime
                    .reorder_waiting_executions(input.expected_revision, &ordered_ids),
            )
        }
        "preempt_and_start_execution" => {
            let input = match parse_payload::<ExecutionIdDocument>(
                request.payload,
                "preempt_and_start_execution",
            ) {
                Ok(input) => input,
                Err(result) => return result,
            };
            let execution_id = match parse_id(input.execution_id, "execution_id") {
                Ok(id) => id,
                Err(result) => return result,
            };
            runtime_result(state.runtime.preempt_and_start_execution(&execution_id))
        }
        "pause_execution" => {
            let input =
                match parse_payload::<ExecutionIdDocument>(request.payload, "pause_execution") {
                    Ok(input) => input,
                    Err(result) => return result,
                };
            let execution_id = match parse_id(input.execution_id, "execution_id") {
                Ok(id) => id,
                Err(result) => return result,
            };
            runtime_result(state.runtime.pause_execution(&execution_id))
        }
        "resume_execution" => {
            let input =
                match parse_payload::<ExecutionIdDocument>(request.payload, "resume_execution") {
                    Ok(input) => input,
                    Err(result) => return result,
                };
            let execution_id = match parse_id(input.execution_id, "execution_id") {
                Ok(id) => id,
                Err(result) => return result,
            };
            runtime_result(state.runtime.resume_execution(&execution_id))
        }
        "cancel_execution" => {
            let input =
                match parse_payload::<ExecutionIdDocument>(request.payload, "cancel_execution") {
                    Ok(input) => input,
                    Err(result) => return result,
                };
            let execution_id = match parse_id(input.execution_id, "execution_id") {
                Ok(id) => id,
                Err(result) => return result,
            };
            runtime_result(state.runtime.cancel_execution(&execution_id))
        }
        "generate_preview_frames" => {
            let input = match parse_payload::<PreviewFramesDocument>(
                request.payload,
                "generate_preview_frames",
            ) {
                Ok(input) => input,
                Err(result) => return result,
            };
            let request = match preview_request(input) {
                Ok(request) => request,
                Err(result) => return result,
            };
            let result = state.adapter.generate_preview_frames(&request);
            match result {
                Ok(result) => {
                    serialized_success(preview_result_document(result), MAX_COMMAND_DOCUMENT_BYTES)
                }
                Err(error) => (
                    FRAMELEAN_FLL_RUNTIME_ERROR,
                    ResponseDocument::failure(&error),
                    MAX_COMMAND_DOCUMENT_BYTES,
                ),
            }
        }
        "generate_video_thumbnail" => {
            let input = match parse_payload::<VideoThumbnailDocument>(
                request.payload,
                "generate_video_thumbnail",
            ) {
                Ok(input) => input,
                Err(result) => return result,
            };
            let request = match thumbnail_request(input) {
                Ok(request) => request,
                Err(result) => return result,
            };
            let result = state.adapter.generate_video_thumbnail(&request);
            match result {
                Ok(result) => serialized_success(
                    thumbnail_result_document(result),
                    MAX_COMMAND_DOCUMENT_BYTES,
                ),
                Err(error) => (
                    FRAMELEAN_FLL_RUNTIME_ERROR,
                    ResponseDocument::failure(&error),
                    MAX_COMMAND_DOCUMENT_BYTES,
                ),
            }
        }
        operation => (
            FRAMELEAN_FLL_INVALID_ARGUMENT,
            ResponseDocument::invalid(format!("unsupported FLL ABI operation: {operation}")),
            MAX_COMMAND_DOCUMENT_BYTES,
        ),
    }
}

fn invalid_error(error: EngineError) -> (i32, ResponseDocument, usize) {
    (
        FRAMELEAN_FLL_INVALID_ARGUMENT,
        ResponseDocument::failure(&error),
        MAX_COMMAND_DOCUMENT_BYTES,
    )
}

fn runtime_result<T: Serialize>(result: Result<T>) -> (i32, ResponseDocument, usize) {
    runtime_result_with_limit(result, MAX_COMMAND_DOCUMENT_BYTES)
}

fn runtime_result_with_limit<T: Serialize>(
    result: Result<T>,
    max_bytes: usize,
) -> (i32, ResponseDocument, usize) {
    match result {
        Ok(value) => serialized_success(value, max_bytes),
        Err(error) => (
            FRAMELEAN_FLL_RUNTIME_ERROR,
            ResponseDocument::failure(&error),
            max_bytes,
        ),
    }
}

fn serialized_success<T: Serialize>(value: T, max_bytes: usize) -> (i32, ResponseDocument, usize) {
    match serde_json::to_value(value) {
        Ok(value) => (
            FRAMELEAN_FLL_OK,
            ResponseDocument::success(value),
            max_bytes,
        ),
        Err(error) => (
            FRAMELEAN_FLL_RUNTIME_ERROR,
            ResponseDocument::invalid(format!("FLL response serialization failed: {error}")),
            max_bytes,
        ),
    }
}

fn preview_request(
    input: PreviewFramesDocument,
) -> std::result::Result<PreviewFramesRequest, (i32, ResponseDocument, usize)> {
    Ok(PreviewFramesRequest {
        input_path: parse_path(input.input_path, "input_path")?,
        output_directory: parse_path(input.output_directory, "output_directory")?,
        timestamps_us: input.timestamps_us,
        max_width: input.max_width,
    })
}

fn thumbnail_request(
    input: VideoThumbnailDocument,
) -> std::result::Result<VideoThumbnailRequest, (i32, ResponseDocument, usize)> {
    Ok(VideoThumbnailRequest {
        input_path: parse_path(input.input_path, "input_path")?,
        output_path: parse_path(input.output_path, "output_path")?,
        duration_us: input.duration_us,
        max_width: input.max_width,
    })
}

fn preview_result_document(result: PreviewFramesResult) -> PreviewFramesResultDocument {
    PreviewFramesResultDocument {
        output_directory: result.output_directory.to_string_lossy().into_owned(),
        frames: result
            .frames
            .into_iter()
            .map(|frame| PreviewFrameDocument {
                index: frame.index,
                requested_timestamp_us: frame.requested_timestamp_us,
                decoded_timestamp_us: frame.decoded_timestamp_us,
                width: frame.width,
                height: frame.height,
                output_path: frame.output_path.to_string_lossy().into_owned(),
            })
            .collect(),
    }
}

fn thumbnail_result_document(result: VideoThumbnailResult) -> VideoThumbnailResultDocument {
    VideoThumbnailResultDocument {
        output_path: result.output_path.to_string_lossy().into_owned(),
        requested_timestamp_us: result.requested_timestamp_us,
        decoded_timestamp_us: result.decoded_timestamp_us,
        width: result.width,
        height: result.height,
    }
}

fn event_document(event: ExecutionRuntimeEvent) -> ExecutionEventDocument {
    ExecutionEventDocument {
        execution_id: event.execution_id.to_string(),
        resource_pool: event.resource_pool,
        sequence: event.sequence,
        state: event.state,
        pause_reason: event.pause_reason,
        preempted_by_execution_id: event
            .preempted_by_execution_id
            .map(|value| value.to_string()),
        resume_depth: event.resume_depth,
        progress: event.progress,
        output_path: event
            .output_path
            .map(|value| value.to_string_lossy().into_owned()),
        error_code: event.error_code,
        message: event.message,
    }
}

fn next_event(state: &mut RuntimeState) -> Option<ExecutionRuntimeEvent> {
    if state.pending_events.is_empty() {
        state
            .pending_events
            .extend(state.runtime.drain_execution_events());
    }
    state.pending_events.pop_front()
}

unsafe extern "C" fn runtime_create(
    out_handle: *mut *mut c_void,
    out_error: *mut *mut u8,
    out_error_len: *mut usize,
) -> i32 {
    if out_handle.is_null() || !reset_output(out_error, out_error_len) {
        return FRAMELEAN_FLL_INVALID_ARGUMENT;
    }
    // SAFETY: out_handle was checked for null and is caller-owned.
    unsafe { ptr::write(out_handle, ptr::null_mut()) };
    match panic::catch_unwind(AssertUnwindSafe(build_default_runtime)) {
        Ok(Ok((runtime, adapter))) => {
            let handle = Box::new(RuntimeHandle {
                state: Mutex::new(RuntimeState {
                    runtime,
                    adapter,
                    pending_events: VecDeque::new(),
                }),
            });
            // SAFETY: out_handle was checked above and receives ownership of a heap allocation.
            unsafe { ptr::write(out_handle, Box::into_raw(handle).cast()) };
            FRAMELEAN_FLL_OK
        }
        Ok(Err(error)) => {
            let write_status = write_response(
                &ResponseDocument::failure(&error),
                MAX_COMMAND_DOCUMENT_BYTES,
                out_error,
                out_error_len,
            );
            if write_status == FRAMELEAN_FLL_OK {
                FRAMELEAN_FLL_RUNTIME_ERROR
            } else {
                write_status
            }
        }
        Err(_) => {
            let write_status = write_response(
                &ResponseDocument::panic(),
                MAX_COMMAND_DOCUMENT_BYTES,
                out_error,
                out_error_len,
            );
            if write_status == FRAMELEAN_FLL_OK {
                FRAMELEAN_FLL_PANIC
            } else {
                write_status
            }
        }
    }
}

unsafe extern "C" fn runtime_destroy(handle: *mut c_void) -> i32 {
    match panic::catch_unwind(AssertUnwindSafe(|| {
        if !handle.is_null() {
            // SAFETY: ownership is transferred back exactly once by the caller.
            unsafe { drop(Box::from_raw(handle.cast::<RuntimeHandle>())) };
        }
    })) {
        Ok(()) => FRAMELEAN_FLL_OK,
        Err(_) => FRAMELEAN_FLL_PANIC,
    }
}

unsafe extern "C" fn runtime_invoke(
    handle: *mut c_void,
    request: *const u8,
    request_len: usize,
    out_response: *mut *mut u8,
    out_response_len: *mut usize,
) -> i32 {
    guarded_response(out_response, out_response_len, || {
        if handle.is_null() {
            return (
                FRAMELEAN_FLL_INVALID_ARGUMENT,
                ResponseDocument::invalid("runtime handle is null"),
                MAX_COMMAND_DOCUMENT_BYTES,
            );
        }
        let request = match parse_request(request, request_len) {
            Ok(request) => request,
            Err(response) => {
                return (
                    FRAMELEAN_FLL_INVALID_ARGUMENT,
                    response,
                    MAX_COMMAND_DOCUMENT_BYTES,
                );
            }
        };
        // SAFETY: handle is checked for null and is only created by runtime_create.
        let handle = unsafe { &*handle.cast::<RuntimeHandle>() };
        let mut state = match handle.state.lock() {
            Ok(state) => state,
            Err(_) => {
                return (
                    FRAMELEAN_FLL_RUNTIME_ERROR,
                    ResponseDocument::invalid("FLL runtime mutex is poisoned"),
                    MAX_COMMAND_DOCUMENT_BYTES,
                );
            }
        };
        dispatch(&mut state, request)
    })
}

unsafe extern "C" fn runtime_poll_event(
    handle: *mut c_void,
    out_event: *mut *mut u8,
    out_event_len: *mut usize,
) -> i32 {
    if !reset_output(out_event, out_event_len) {
        return FRAMELEAN_FLL_BUFFER_ERROR;
    }
    match panic::catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            return (
                FRAMELEAN_FLL_INVALID_ARGUMENT,
                Some((
                    ResponseDocument::invalid("runtime handle is null"),
                    MAX_COMMAND_DOCUMENT_BYTES,
                )),
            );
        }
        // SAFETY: handle is checked for null and is only created by runtime_create.
        let handle = unsafe { &*handle.cast::<RuntimeHandle>() };
        let mut state = match handle.state.lock() {
            Ok(state) => state,
            Err(_) => {
                return (
                    FRAMELEAN_FLL_RUNTIME_ERROR,
                    Some((
                        ResponseDocument::invalid("FLL runtime mutex is poisoned"),
                        MAX_COMMAND_DOCUMENT_BYTES,
                    )),
                );
            }
        };
        match next_event(&mut state) {
            Some(event) => match serde_json::to_value(event_document(event)) {
                Ok(value) => (
                    FRAMELEAN_FLL_OK,
                    Some((ResponseDocument::success(value), MAX_COMMAND_DOCUMENT_BYTES)),
                ),
                Err(error) => (
                    FRAMELEAN_FLL_RUNTIME_ERROR,
                    Some((
                        ResponseDocument::invalid(format!(
                            "FLL execution event serialization failed: {error}"
                        )),
                        MAX_COMMAND_DOCUMENT_BYTES,
                    )),
                ),
            },
            None => (FRAMELEAN_FLL_NO_EVENT, None),
        }
    })) {
        Ok((FRAMELEAN_FLL_NO_EVENT, None)) => FRAMELEAN_FLL_NO_EVENT,
        Ok((status, Some((response, max_bytes)))) => {
            let write_status = write_response(&response, max_bytes, out_event, out_event_len);
            if write_status == FRAMELEAN_FLL_OK {
                status
            } else {
                write_status
            }
        }
        Ok((status, None)) => status,
        Err(_) => {
            let write_status = write_response(
                &ResponseDocument::panic(),
                MAX_COMMAND_DOCUMENT_BYTES,
                out_event,
                out_event_len,
            );
            if write_status == FRAMELEAN_FLL_OK {
                FRAMELEAN_FLL_PANIC
            } else {
                write_status
            }
        }
    }
}

unsafe extern "C" fn buffer_free(buffer: *mut u8, buffer_len: usize) {
    if buffer.is_null() || buffer_len == 0 || buffer_len > MAX_BUFFER_BYTES {
        return;
    }
    // SAFETY: the caller must pass a buffer allocated by this FLL facade and its exact length.
    unsafe { drop(Vec::from_raw_parts(buffer, buffer_len, buffer_len)) };
}

static API_TABLE: FrameLeanFllApiV1 = FrameLeanFllApiV1 {
    struct_size: size_of::<FrameLeanFllApiV1>() as u32,
    abi_major: FRAMELEAN_FLL_ABI_MAJOR,
    abi_minor: FRAMELEAN_FLL_ABI_MINOR,
    runtime_create,
    runtime_destroy,
    invoke: runtime_invoke,
    poll_event: runtime_poll_event,
    buffer_free,
};

fn api_table() -> &'static FrameLeanFllApiV1 {
    &API_TABLE
}

/// Returns a pointer to the versioned FLL API table owned by the dynamic library.
///
/// A requested minor version lower than or equal to the library minor version is supported. The
/// caller must reject an incompatible major version and must inspect `struct_size` before using
/// fields that were appended after the version it understands.
///
/// # Safety
///
/// `out_api` must point to writable storage for one API-table pointer. A null pointer is rejected.
/// The output is cleared before version validation, so callers may safely reuse the storage after
/// any failed call. The returned table pointer must not be used after this dynamic library is
/// unloaded.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn framelean_fll_get_api(
    requested_major: u16,
    requested_minor: u16,
    out_api: *mut *const FrameLeanFllApiV1,
) -> i32 {
    if out_api.is_null() {
        return FRAMELEAN_FLL_INVALID_ARGUMENT;
    }
    // SAFETY: out_api was checked for null and is caller-owned writable storage.
    unsafe { ptr::write(out_api, ptr::null()) };
    match panic::catch_unwind(AssertUnwindSafe(|| {
        if requested_major != FRAMELEAN_FLL_ABI_MAJOR || requested_minor > FRAMELEAN_FLL_ABI_MINOR {
            return FRAMELEAN_FLL_ABI_MISMATCH;
        }
        // SAFETY: out_api was checked for null and is caller-owned. The pointed-to table is
        // immutable and remains valid while this dynamic library remains loaded.
        unsafe { ptr::write(out_api, api_table()) };
        FRAMELEAN_FLL_OK
    })) {
        Ok(status) => status,
        Err(_) => FRAMELEAN_FLL_PANIC,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(operation: &str, payload: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "document_version": FRAMELEAN_FLL_DOCUMENT_VERSION,
            "operation": operation,
            "payload": payload,
        }))
        .unwrap()
    }

    fn event(sequence: u64) -> ExecutionRuntimeEvent {
        ExecutionRuntimeEvent {
            execution_id: TaskId::new(format!("execution-{sequence}")).unwrap(),
            resource_pool: framelean_runtime::ExecutionResourcePool::Auxiliary,
            sequence,
            state: framelean_runtime::ExecutionTaskState::Queued,
            pause_reason: None,
            preempted_by_execution_id: None,
            resume_depth: 0,
            progress: None,
            output_path: None,
            error_code: None,
            message: None,
        }
    }

    #[test]
    fn api_table_is_versioned_and_complete() {
        let mut api = ptr::null();
        let status = unsafe { framelean_fll_get_api(1, 0, &mut api) };
        assert_eq!(status, FRAMELEAN_FLL_OK);
        assert!(!api.is_null());
        // SAFETY: successful bootstrap returns a valid immutable table owned by this library.
        let api = unsafe { &*api };
        assert_eq!(api.struct_size as usize, size_of::<FrameLeanFllApiV1>());
        assert_eq!(api.abi_major, FRAMELEAN_FLL_ABI_MAJOR);
        assert_eq!(api.abi_minor, FRAMELEAN_FLL_ABI_MINOR);
    }

    #[test]
    fn api_rejects_incompatible_versions() {
        let mut api = ptr::NonNull::<FrameLeanFllApiV1>::dangling().as_ptr() as *const _;
        assert_eq!(
            unsafe { framelean_fll_get_api(2, 0, &mut api) },
            FRAMELEAN_FLL_ABI_MISMATCH
        );
        assert!(api.is_null());

        api = ptr::NonNull::<FrameLeanFllApiV1>::dangling().as_ptr() as *const _;
        assert_eq!(
            unsafe { framelean_fll_get_api(1, 1, &mut api) },
            FRAMELEAN_FLL_ABI_MISMATCH
        );
        assert!(api.is_null());
    }

    #[test]
    fn invalid_invoke_request_returns_versioned_error() {
        let request = request("ping", json!({}));
        let mut response = ptr::null_mut();
        let mut response_len = 0;
        let status = unsafe {
            runtime_invoke(
                ptr::null_mut(),
                request.as_ptr(),
                request.len(),
                &mut response,
                &mut response_len,
            )
        };
        assert_eq!(status, FRAMELEAN_FLL_INVALID_ARGUMENT);
        let bytes = unsafe { std::slice::from_raw_parts(response, response_len) };
        let value: Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(value["document_version"], FRAMELEAN_FLL_DOCUMENT_VERSION);
        assert_eq!(value["ok"], false);
        unsafe { buffer_free(response, response_len) };
    }

    #[test]
    fn malformed_documents_fail_closed_before_handle_access() {
        let malformed = br#"{"#;
        let parsed = parse_request(malformed.as_ptr(), malformed.len());
        assert!(parsed.is_err());
    }

    #[test]
    fn unsupported_documents_fail_closed_after_parsing() {
        let unsupported = request("not-an-operation", json!({}));
        let parsed = parse_request(unsupported.as_ptr(), unsupported.len()).unwrap();
        let mut state = RuntimeState {
            runtime: EngineRuntime::new(),
            adapter: Arc::new(FfmpegAdapter),
            pending_events: VecDeque::new(),
        };
        let (status, response, _) = dispatch(&mut state, parsed);
        assert_eq!(status, FRAMELEAN_FLL_INVALID_ARGUMENT);
        assert!(!response.ok);
    }

    #[test]
    fn null_handle_invoke_returns_structured_error() {
        let request = request("ping", json!({}));
        let mut response = ptr::null_mut();
        let mut response_len = 0;
        assert_eq!(
            unsafe {
                runtime_invoke(
                    ptr::null_mut(),
                    request.as_ptr(),
                    request.len(),
                    &mut response,
                    &mut response_len,
                )
            },
            FRAMELEAN_FLL_INVALID_ARGUMENT
        );
        unsafe { buffer_free(response, response_len) };
    }

    #[test]
    fn pending_events_are_drained_without_loss_or_reordering() {
        let mut state = RuntimeState {
            runtime: EngineRuntime::new(),
            adapter: Arc::new(FfmpegAdapter),
            pending_events: VecDeque::from([event(1), event(2)]),
        };
        assert_eq!(next_event(&mut state).unwrap().sequence, 1);
        assert_eq!(next_event(&mut state).unwrap().sequence, 2);
        assert!(next_event(&mut state).is_none());
    }

    #[test]
    fn destroy_and_buffer_free_accept_null_safely() {
        assert_eq!(
            unsafe { runtime_destroy(ptr::null_mut()) },
            FRAMELEAN_FLL_OK
        );
        unsafe { buffer_free(ptr::null_mut(), 0) };
    }
}
