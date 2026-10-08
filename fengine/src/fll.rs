use std::env;
use std::error::Error;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::PathBuf;
use std::ptr;

use libloading::{Library, Symbol};
use serde_json::{Value, json};

const ABI_MAJOR: u16 = 1;
const ABI_MINOR: u16 = 0;
const ABI_OK: i32 = 0;
const MAX_ABI_BUFFER: usize = 64 * 1024 * 1024;

pub(super) struct InvokeError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

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
type BufferFree = unsafe extern "C" fn(buffer: *mut u8, buffer_len: usize);
type GetApi = unsafe extern "C" fn(
    requested_major: u16,
    requested_minor: u16,
    out_api: *mut *const ApiV1,
) -> i32;

#[repr(C)]
#[derive(Clone, Copy)]
struct ApiV1 {
    struct_size: u32,
    abi_major: u16,
    abi_minor: u16,
    runtime_create: RuntimeCreate,
    runtime_destroy: RuntimeDestroy,
    invoke: Invoke,
    poll_event: unsafe extern "C" fn(*mut c_void, *mut *mut u8, *mut usize) -> i32,
    buffer_free: BufferFree,
}

pub(super) struct Runtime {
    _library: Library,
    api: ApiV1,
    handle: *mut c_void,
}

impl Runtime {
    pub fn load_default() -> Result<Self, Box<dyn Error>> {
        let path = library_path()?;
        // Keep the library loaded for the full lifetime of every function pointer and runtime handle.
        let library = unsafe { Library::new(&path) }.map_err(|error| {
            format!(
                "could not load FLL dynamic library {}: {error}",
                path.display()
            )
        })?;
        let bootstrap: Symbol<'_, GetApi> = unsafe { library.get(b"framelean_fll_get_api\0") }
            .map_err(|error| {
                format!(
                    "FLL library {} does not export framelean_fll_get_api: {error}",
                    path.display()
                )
            })?;
        let bootstrap = *bootstrap;
        let mut api_pointer = ptr::null();
        let status = unsafe { bootstrap(ABI_MAJOR, ABI_MINOR, &mut api_pointer) };
        if status != ABI_OK || api_pointer.is_null() {
            return Err(format!(
                "FLL ABI v{ABI_MAJOR}.{ABI_MINOR} negotiation failed with status {status}"
            )
            .into());
        }
        let struct_size = unsafe { std::ptr::addr_of!((*api_pointer).struct_size).read() } as usize;
        if struct_size < size_of::<ApiV1>() {
            return Err(format!(
                "FLL ABI table is too small: expected at least {} bytes, got {struct_size}",
                size_of::<ApiV1>()
            )
            .into());
        }
        let api = unsafe { *api_pointer };
        if api.abi_major != ABI_MAJOR || api.abi_minor < ABI_MINOR {
            return Err(format!(
                "FLL ABI mismatch: expected v{ABI_MAJOR}.{ABI_MINOR}, got v{}.{}",
                api.abi_major, api.abi_minor
            )
            .into());
        }

        let mut handle = ptr::null_mut();
        let mut error_pointer = ptr::null_mut();
        let mut error_len = 0;
        let status =
            unsafe { (api.runtime_create)(&mut handle, &mut error_pointer, &mut error_len) };
        if status != ABI_OK || handle.is_null() {
            let error = take_buffer(error_pointer, error_len, api.buffer_free)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|value| value["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "runtime_create returned no error detail".to_owned());
            return Err(
                format!("FLL runtime initialization failed (status {status}): {error}").into(),
            );
        }

        Ok(Self {
            _library: library,
            api,
            handle,
        })
    }

    pub fn invoke(&mut self, operation: &str, payload: Value) -> Result<Value, InvokeError> {
        let request = serde_json::to_vec(&json!({
            "document_version": 1,
            "operation": operation,
            "payload": payload,
        }))
        .map_err(|error| InvokeError {
            code: "serialization_failed".to_owned(),
            message: error.to_string(),
            retryable: false,
        })?;
        if request.len() > 16 * 1024 * 1024 {
            return Err(InvokeError {
                code: "request_too_large".to_owned(),
                message: "FLL command exceeds the 16 MiB ABI limit".to_owned(),
                retryable: false,
            });
        }

        let mut response_pointer = ptr::null_mut();
        let mut response_len = 0;
        let status = unsafe {
            (self.api.invoke)(
                self.handle,
                request.as_ptr(),
                request.len(),
                &mut response_pointer,
                &mut response_len,
            )
        };
        let bytes = take_buffer(response_pointer, response_len, self.api.buffer_free).map_err(
            |message| InvokeError {
                code: "abi_buffer_error".to_owned(),
                message,
                retryable: false,
            },
        )?;
        let envelope: Value = serde_json::from_slice(&bytes).map_err(|error| InvokeError {
            code: "invalid_engine_response".to_owned(),
            message: format!("FLL returned invalid JSON: {error}"),
            retryable: false,
        })?;
        if status != ABI_OK || envelope["ok"] != true {
            return Err(InvokeError {
                code: envelope["error"]["code"]
                    .as_str()
                    .unwrap_or("runtime_failure")
                    .to_owned(),
                message: envelope["error"]["message"]
                    .as_str()
                    .unwrap_or("FLL operation failed")
                    .to_owned(),
                retryable: envelope["error"]["retryable"].as_bool().unwrap_or(false),
            });
        }
        envelope.get("result").cloned().ok_or_else(|| InvokeError {
            code: "invalid_engine_response".to_owned(),
            message: "FLL success response has no result".to_owned(),
            retryable: false,
        })
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.api.runtime_destroy)(self.handle) };
            self.handle = ptr::null_mut();
        }
    }
}

fn library_path() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(path) = env::var_os("FRAMELEAN_FLL_LIBRARY") {
        return Ok(PathBuf::from(path));
    }
    let name = if cfg!(target_os = "macos") {
        "libframelean_fll.dylib"
    } else if cfg!(target_os = "windows") {
        "framelean_fll.dll"
    } else {
        return Err("FLL dynamic loading is supported only on macOS and Windows".into());
    };
    let executable_dir = env::current_exe()?
        .parent()
        .ok_or("FrameLean Work executable has no parent directory")?
        .to_owned();
    let path = executable_dir.join(name);
    if !path.is_file() {
        return Err(format!(
            "FLL dynamic library is missing beside Work: {}",
            path.display()
        )
        .into());
    }
    Ok(path)
}

fn take_buffer(pointer: *mut u8, length: usize, free: BufferFree) -> Result<Vec<u8>, String> {
    if pointer.is_null() {
        return if length == 0 {
            Ok(Vec::new())
        } else {
            Err(format!("FLL returned a null buffer with length {length}"))
        };
    }
    if length == 0 || length > MAX_ABI_BUFFER {
        return Err(format!("FLL returned an invalid buffer length {length}"));
    }
    let bytes = unsafe { std::slice::from_raw_parts(pointer, length) }.to_vec();
    unsafe { free(pointer, length) };
    Ok(bytes)
}
