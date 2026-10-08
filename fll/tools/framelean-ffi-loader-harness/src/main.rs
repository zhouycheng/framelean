use std::env;
use std::error::Error;
use std::ffi::c_void;
use std::fs;
use std::mem::size_of;
use std::path::PathBuf;
use std::ptr;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use libloading::{Library, Symbol};
use serde_json::{Value, json};

const ABI_MAJOR: u16 = 1;
const ABI_MINOR: u16 = 0;
const DOCUMENT_VERSION: u16 = 1;
const OK: i32 = 0;
const ABI_MISMATCH: i32 = 6;
const NO_EVENT: i32 = 5;

type RuntimeCreate = unsafe extern "C" fn(
    out_handle: *mut *mut c_void,
    out_error: *mut *mut u8,
    out_error_len: *mut usize,
) -> i32;
type RuntimeDestroy = unsafe extern "C" fn(handle: *mut c_void) -> i32;
type Invoke = unsafe extern "C" fn(
    handle: *mut c_void,
    request: *const u8,
    request_len: usize,
    out_response: *mut *mut u8,
    out_response_len: *mut usize,
) -> i32;
type PollEvent = unsafe extern "C" fn(
    handle: *mut c_void,
    out_event: *mut *mut u8,
    out_event_len: *mut usize,
) -> i32;
type BufferFree = unsafe extern "C" fn(buffer: *mut u8, buffer_len: usize);
type GetApi = unsafe extern "C" fn(
    requested_major: u16,
    requested_minor: u16,
    out_api: *mut *const FrameLeanFllApiV1,
) -> i32;

#[repr(C)]
struct FrameLeanFllApiV1 {
    struct_size: u32,
    abi_major: u16,
    abi_minor: u16,
    runtime_create: RuntimeCreate,
    runtime_destroy: RuntimeDestroy,
    invoke: Invoke,
    poll_event: PollEvent,
    buffer_free: BufferFree,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os();
    let program = arguments.next().unwrap_or_default();
    let library_path = arguments.next().map(PathBuf::from).ok_or_else(|| {
        format!(
            "usage: {} <path-to-framelean_fll-dylib-or-dll>",
            program.to_string_lossy()
        )
    })?;
    if arguments.next().is_some() {
        return Err("expected exactly one dynamic library path".into());
    }

    // Keep the Library alive until every function pointer and returned buffer is finished.
    let library = unsafe { Library::new(&library_path)? };
    let bootstrap: Symbol<'_, GetApi> = unsafe { library.get(b"framelean_fll_get_api\0")? };

    let mut incompatible_api = api_sentinel();
    let incompatible_status = unsafe { bootstrap(ABI_MAJOR + 1, ABI_MINOR, &mut incompatible_api) };
    if incompatible_status != ABI_MISMATCH || !incompatible_api.is_null() {
        return Err(
            format!("incompatible ABI negotiation returned status {incompatible_status}").into(),
        );
    }

    let mut api_ptr = ptr::null();
    let status = unsafe { bootstrap(ABI_MAJOR, ABI_MINOR, &mut api_ptr) };
    if status != OK || api_ptr.is_null() {
        return Err(format!("ABI bootstrap failed with status {status}").into());
    }
    // SAFETY: successful bootstrap returns an immutable table owned by the loaded library.
    let api = unsafe { &*api_ptr };
    if (api.struct_size as usize) < size_of::<FrameLeanFllApiV1>() {
        return Err(format!("API table is too small: {}", api.struct_size).into());
    }
    if api.abi_major != ABI_MAJOR || api.abi_minor < ABI_MINOR {
        return Err(format!("unexpected API version {}.{}", api.abi_major, api.abi_minor).into());
    }

    let mut handle = create_runtime(api)?;

    let result = run_runtime_smoke(api, &mut handle);
    if !handle.is_null() {
        destroy_runtime(api, handle)?;
    }
    result
}

fn create_runtime(api: &FrameLeanFllApiV1) -> Result<*mut c_void, Box<dyn Error>> {
    let mut handle = ptr::null_mut();
    let mut create_error = ptr::null_mut();
    let mut create_error_len = 0;
    let status =
        unsafe { (api.runtime_create)(&mut handle, &mut create_error, &mut create_error_len) };
    if status != OK || handle.is_null() {
        let message = take_json_error(create_error, create_error_len, api.buffer_free);
        return Err(format!("runtime_create failed with status {status}: {message}").into());
    }
    Ok(handle)
}

fn destroy_runtime(api: &FrameLeanFllApiV1, handle: *mut c_void) -> Result<(), Box<dyn Error>> {
    let status = unsafe { (api.runtime_destroy)(handle) };
    if status != OK {
        return Err(format!("runtime_destroy failed with status {status}").into());
    }
    Ok(())
}

fn run_runtime_smoke(
    api: &FrameLeanFllApiV1,
    handle: &mut *mut c_void,
) -> Result<(), Box<dyn Error>> {
    let ping = invoke_operation(api, *handle, "ping", json!({}))?;
    if ping["service"] != "framelean-fll-runtime" {
        return Err(format!("unexpected ping response: {ping}").into());
    }

    let mut event = ptr::null_mut();
    let mut event_len = 0;
    let event_status = unsafe { (api.poll_event)(*handle, &mut event, &mut event_len) };
    if event_status != NO_EVENT || !event.is_null() || event_len != 0 {
        if !event.is_null() {
            unsafe { (api.buffer_free)(event, event_len) };
        }
        return Err(format!("initial poll_event returned status {event_status}").into());
    }

    let mut fixture = MediaFixture::create()?;
    let analysis = invoke_operation(
        api,
        *handle,
        "analyze_media",
        json!({
            "task_mode": "audio_compress",
            "media_request": {
                "source": {"kind": "local_file", "value": fixture.input.clone()},
                "request_id": "framelean-ffi-loader",
                "expected_source": null,
            },
            "context": {},
        }),
    )?;
    if analysis["media_analysis_status"] != "complete" {
        return Err(format!("dynamic analyze_media did not complete: {analysis}").into());
    }
    let analysis_id = required_string(&analysis, &["analysis_id"])?;
    let analysis_revision = analysis["analysis_revision"]
        .as_u64()
        .ok_or("analyze_media did not return an analysis revision")?;
    let candidate = analysis["capabilities"]["execution_chains"]
        .as_array()
        .and_then(|candidates| {
            candidates
                .iter()
                .find(|candidate| candidate["output_container"].as_str() == Some("m4a"))
        })
        .ok_or("analyze_media returned no supported audio execution chain")?;
    let candidate_id = required_string(candidate, &["id"])?;
    let output_extension = candidate["output_container"]
        .as_str()
        .ok_or("execution candidate has no output container")?;

    let snapshot = invoke_operation(
        api,
        *handle,
        "export_analysis_snapshot",
        json!({"analysis_id": analysis_id.clone()}),
    )?;
    if snapshot["id"] != analysis_id || snapshot["revision"] != analysis_revision {
        return Err(format!("snapshot identity changed across export: {snapshot}").into());
    }

    destroy_runtime(api, *handle)?;
    *handle = ptr::null_mut();
    *handle = create_runtime(api)?;
    invoke_operation(api, *handle, "restore_analysis_snapshot", snapshot)?;
    let restored = invoke_operation(
        api,
        *handle,
        "analysis_snapshot",
        json!({"analysis_id": analysis_id.clone()}),
    )?;
    if restored["analysis_id"] != analysis_id
        || restored["analysis_revision"] != analysis_revision
        || restored["validity"]["status"] != "valid"
    {
        return Err(format!("restored analysis snapshot is invalid: {restored}").into());
    }

    let output_path = fixture.output_path(output_extension);
    let submission = invoke_operation(
        api,
        *handle,
        "submit_execution",
        json!({
            "analysis_id": analysis_id.clone(),
            "expected_revision": analysis_revision,
            "selection": {
                "mode": "manual",
                "selection": {
                    "candidate_id": candidate_id.clone(),
                    "overrides": {},
                },
            },
            "output": {
                "requested_path": output_path.clone(),
                "collision_policy": "fail_if_exists",
            },
            "context": {},
        }),
    )?;
    let execution_id = required_string(&submission, &["execution_id"])?;
    poll_until_terminal(api, *handle, &execution_id, &output_path)?;
    Ok(())
}

fn invoke_operation(
    api: &FrameLeanFllApiV1,
    handle: *mut c_void,
    operation: &str,
    payload: Value,
) -> Result<Value, Box<dyn Error>> {
    let request = serde_json::to_vec(&json!({
        "document_version": DOCUMENT_VERSION,
        "operation": operation,
        "payload": payload,
    }))?;
    let mut response = ptr::null_mut();
    let mut response_len = 0;
    let status = unsafe {
        (api.invoke)(
            handle,
            request.as_ptr(),
            request.len(),
            &mut response,
            &mut response_len,
        )
    };
    let envelope = take_json(response, response_len, api.buffer_free)?;
    if status != OK || envelope["ok"] != true {
        return Err(format!("{operation} failed with status {status}: {envelope}").into());
    }
    Ok(envelope["result"].clone())
}

fn required_string(value: &Value, path: &[&str]) -> Result<String, Box<dyn Error>> {
    let mut current = value;
    for key in path {
        current = &current[*key];
    }
    current
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("missing string field {}", path.join(".")).into())
}

fn poll_until_terminal(
    api: &FrameLeanFllApiV1,
    handle: *mut c_void,
    execution_id: &str,
    output_path: &PathBuf,
) -> Result<(), Box<dyn Error>> {
    let mut last_sequence = 0;
    for _ in 0..750 {
        let Some(envelope) = poll_event(api, handle)? else {
            thread::sleep(Duration::from_millis(20));
            continue;
        };
        if envelope["ok"] != true {
            return Err(format!("poll_event returned an error: {envelope}").into());
        }
        let event = &envelope["result"];
        if event["execution_id"] != execution_id {
            return Err(format!("event belongs to another execution: {event}").into());
        }
        let sequence = event["sequence"]
            .as_u64()
            .ok_or("execution event has no sequence")?;
        if sequence <= last_sequence {
            return Err(format!("execution event sequence regressed: {event}").into());
        }
        last_sequence = sequence;
        match event["state"].as_str() {
            Some("completed") => {
                if !output_path.exists() {
                    return Err("completed execution did not publish its output".into());
                }
                return Ok(());
            }
            Some("failed") | Some("cancelled") => {
                return Err(format!("execution reached terminal failure: {event}").into());
            }
            _ => {}
        }
    }
    Err("timed out waiting for the dynamic execution terminal event".into())
}

fn poll_event(
    api: &FrameLeanFllApiV1,
    handle: *mut c_void,
) -> Result<Option<Value>, Box<dyn Error>> {
    let mut event = ptr::null_mut();
    let mut event_len = 0;
    let status = unsafe { (api.poll_event)(handle, &mut event, &mut event_len) };
    if status == NO_EVENT {
        if !event.is_null() || event_len != 0 {
            if !event.is_null() {
                unsafe { (api.buffer_free)(event, event_len) };
            }
            return Err("NO_EVENT returned a non-empty buffer".into());
        }
        return Ok(None);
    }
    let envelope = take_json(event, event_len, api.buffer_free)?;
    if status != OK {
        return Err(format!("poll_event failed with status {status}: {envelope}").into());
    }
    Ok(Some(envelope))
}

struct MediaFixture {
    input: PathBuf,
    output: Option<PathBuf>,
}

impl MediaFixture {
    fn create() -> Result<Self, Box<dyn Error>> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let input = env::temp_dir().join(format!("framelean-ffi-loader-{nonce}.wav"));
        const SAMPLE_RATE: u32 = 8_000;
        const SAMPLE_COUNT: u32 = 800;
        const CHANNELS: u16 = 1;
        const BITS_PER_SAMPLE: u16 = 16;
        let data_size = SAMPLE_COUNT * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE / 8);
        let mut bytes = Vec::with_capacity((44 + data_size) as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&CHANNELS.to_le_bytes());
        bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        let byte_rate = SAMPLE_RATE * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE / 8);
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        bytes.extend_from_slice(&(CHANNELS * (BITS_PER_SAMPLE / 8)).to_le_bytes());
        bytes.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        for sample in 0..SAMPLE_COUNT {
            let value = if sample % 32 < 16 {
                8_000_i16
            } else {
                -8_000_i16
            };
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        fs::write(&input, bytes)?;
        Ok(Self {
            input,
            output: None,
        })
    }

    fn output_path(&mut self, extension: &str) -> PathBuf {
        let output = self.input.with_file_name(format!(
            "{}-output.{}",
            self.input.file_stem().unwrap().to_string_lossy(),
            extension
        ));
        self.output = Some(output.clone());
        output
    }
}

impl Drop for MediaFixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.input);
        if let Some(output) = &self.output {
            let _ = fs::remove_file(output);
        }
    }
}

fn take_json(
    buffer: *mut u8,
    buffer_len: usize,
    buffer_free: BufferFree,
) -> Result<Value, Box<dyn Error>> {
    if buffer.is_null() || buffer_len == 0 {
        return Err("library returned an empty JSON buffer".into());
    }
    // SAFETY: the library returned a readable buffer with the stated length.
    let parsed = unsafe { serde_json::from_slice(std::slice::from_raw_parts(buffer, buffer_len)) };
    unsafe { buffer_free(buffer, buffer_len) };
    Ok(parsed?)
}

fn api_sentinel() -> *const FrameLeanFllApiV1 {
    ptr::NonNull::<FrameLeanFllApiV1>::dangling().as_ptr()
}

fn take_json_error(buffer: *mut u8, buffer_len: usize, buffer_free: BufferFree) -> String {
    if buffer.is_null() || buffer_len == 0 {
        return "no structured error returned".to_owned();
    }
    // SAFETY: the library returned a readable buffer with the stated length.
    let message = unsafe {
        serde_json::from_slice::<Value>(std::slice::from_raw_parts(buffer, buffer_len))
            .map_or_else(|error| error.to_string(), |value| value.to_string())
    };
    unsafe { buffer_free(buffer, buffer_len) };
    message
}
