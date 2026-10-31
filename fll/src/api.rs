use crate::{analysis, capabilities, operations, planning};
use fll_contracts::*;
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};
pub struct Operation {
    pub(crate) cancel: AtomicBool,
    state: Mutex<OperationEvent>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}
impl Operation {
    pub(crate) fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(MediaError::new("canceled", "operation", "操作已取消"))
        } else {
            Ok(())
        }
    }
    pub(crate) fn progress(&self, phase: &str, progress: Option<f64>, time: Option<i64>) {
        let mut s = self.state.lock().unwrap();
        s.state = phase.into();
        s.progress = progress;
        s.processed_time_us = time;
    }
}
fn run(r: FllRequest, op: &Operation) -> Result<Value> {
    if r.schema_version != JSON_VERSION {
        return Err(MediaError::new(
            "protocol_mismatch",
            "request",
            "需要媒体 JSON schema 3",
        ));
    }
    if capabilities::snapshot().backend_versions["compatible"] != true {
        return Err(MediaError::new(
            "backend_version_mismatch",
            "startup",
            "libav 主版本与编译头文件不匹配",
        ));
    }
    let value = match r.operation {
        FllCommand::Capabilities {} => json!(capabilities::snapshot()),
        FllCommand::Analyze { input, cached } => {
            json!(analysis::analyze(&input, cached.as_ref(), op)?)
        }
        FllCommand::Options { media, intent } => json!(planning::options(&media, &intent)?),
        FllCommand::Plan {
            media,
            intent,
            selection,
        } => json!(planning::plan(&media, &intent, &selection)?),
        FllCommand::Execute {
            input,
            temp,
            media,
            plan,
            grant,
        } => json!(operations::execute(
            &input, &temp, &media, &plan, &grant, op
        )?),
    };
    Ok(value)
}
#[unsafe(no_mangle)]
pub extern "C" fn fll_abi_version() -> u32 {
    ABI_VERSION
}
#[unsafe(no_mangle)]
pub extern "C" fn fll_versions() -> *mut c_char {
    encoded(
        json!({"abi_version":ABI_VERSION,"json_version":JSON_VERSION,"capabilities":capabilities::snapshot()}),
    )
}
fn encoded(v: Value) -> *mut c_char {
    CString::new(v.to_string()).unwrap().into_raw()
}
/// # Safety
/// request must be a valid NUL-terminated UTF-8 string for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fll_start(request: *const c_char) -> *mut Arc<Operation> {
    if request.is_null() {
        return std::ptr::null_mut();
    }
    let parsed = unsafe { CStr::from_ptr(request) }
        .to_str()
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let id = parsed
        .as_ref()
        .and_then(|v| v["operation_id"].as_str())
        .unwrap_or("")
        .to_string();
    let op = Arc::new(Operation {
        cancel: AtomicBool::new(false),
        thread: Mutex::new(None),
        state: Mutex::new(OperationEvent {
            operation_id: id,
            state: "running".into(),
            progress: None,
            processed_time_us: None,
            result: None,
            error: None,
        }),
    });
    let work = op.clone();
    let worker = thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let r = parsed.ok_or_else(|| {
                MediaError::new("invalid_request", "request", "请求不是有效 JSON")
            })?;
            let r = serde_json::from_value::<FllRequest>(r)
                .map_err(|e| MediaError::new("invalid_request", "request", e))?;
            run(r, &work)
        }))
        .unwrap_or_else(|_| {
            Err(MediaError::new(
                "panic",
                "operation",
                "FLL operation panicked",
            ))
        });
        let mut s = work.state.lock().unwrap();
        if work.cancel.load(Ordering::Relaxed) {
            s.state = "canceled".into();
            s.error = Some(MediaError::new("canceled", "operation", "操作已取消"));
        } else {
            match result {
                Ok(v) => {
                    s.state = "succeeded".into();
                    s.progress = Some(1.0);
                    s.result = Some(v);
                }
                Err(e) => {
                    s.state = "failed".into();
                    s.error = Some(e);
                }
            }
        }
    });
    *op.thread.lock().unwrap() = Some(worker);
    Box::into_raw(Box::new(op))
}
/// # Safety
/// handle must be a live operation pointer returned by fll_start.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fll_poll(handle: *const Arc<Operation>) -> *mut c_char {
    if handle.is_null() {
        return encoded(json!({"state":"failed","error":{"code":"invalid_handle"}}));
    }
    encoded(json!(*unsafe { &*handle }.state.lock().unwrap()))
}
/// # Safety
/// handle must be live; the Host serializes cancellation with release.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fll_cancel(handle: *const Arc<Operation>) {
    if !handle.is_null() {
        unsafe { &*handle }.cancel.store(true, Ordering::Relaxed);
    }
}
/// # Safety
/// Release once after terminal poll, with no concurrent handle calls.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fll_release(handle: *mut Arc<Operation>) {
    if !handle.is_null() {
        let op = unsafe { Box::from_raw(handle) };
        if let Some(worker) = op.thread.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
}
/// # Safety
/// buffer must be an unreleased JSON buffer returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fll_free(buffer: *mut c_char) {
    if !buffer.is_null() {
        drop(unsafe { CString::from_raw(buffer) });
    }
}
