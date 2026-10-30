//! FLL ABI v1. Handles belong to the library; callers release JSON with fll_free.
mod media;
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
    pub cancel: AtomicBool,
    pub state: Mutex<Value>,
}
impl Operation {
    pub fn update(&self, value: Value) {
        *self.state.lock().unwrap() = value;
    }
    pub fn check(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Relaxed) {
            Err("canceled".into())
        } else {
            Ok(())
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn fll_abi_version() -> u32 {
    1
}
#[unsafe(no_mangle)]
pub extern "C" fn fll_versions() -> *mut c_char {
    encoded(media::versions())
}
fn encoded(v: Value) -> *mut c_char {
    CString::new(v.to_string()).unwrap().into_raw()
}
/// # Safety
/// `request` must be a valid NUL-terminated UTF-8 JSON string for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fll_start(request: *const c_char) -> *mut Arc<Operation> {
    if request.is_null() {
        return std::ptr::null_mut();
    }
    let parsed = unsafe { CStr::from_ptr(request) }
        .to_str()
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let Some(request) = parsed else {
        return std::ptr::null_mut();
    };
    let op = Arc::new(Operation {
        cancel: AtomicBool::new(false),
        state: Mutex::new(json!({"state":"running","progress":0.0})),
    });
    let work = op.clone();
    thread::spawn(move || {
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| media::run(&request, &work)));
        match result {
            Ok(Ok(result))=>work.update(json!({"state":"succeeded","progress":1.0,"result":result})),
            Ok(Err(e))=>work.update(json!({"state":if e=="canceled"{"canceled"}else{"failed"},"error":{"code":if e=="canceled"{"canceled"}else{"media_error"},"message":e}})),
            Err(_)=>work.update(json!({"state":"failed","error":{"code":"panic","message":"FLL operation panicked"}})),
        }
    });
    Box::into_raw(Box::new(op))
}
/// # Safety
/// `handle` must be a live handle returned by fll_start.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fll_poll(handle: *const Arc<Operation>) -> *mut c_char {
    if handle.is_null() {
        return encoded(json!({"state":"failed","error":{"code":"invalid_handle"}}));
    }
    encoded(unsafe { &*handle }.state.lock().unwrap().clone())
}
/// # Safety
/// `handle` must be a live handle returned by fll_start.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fll_cancel(handle: *const Arc<Operation>) {
    if !handle.is_null() {
        unsafe { &*handle }.cancel.store(true, Ordering::Relaxed);
    }
}
/// # Safety
/// Release each handle once, after polling a terminal state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fll_release(handle: *mut Arc<Operation>) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}
/// # Safety
/// `buffer` must be a live JSON pointer returned by this library; release once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fll_free(buffer: *mut c_char) {
    if !buffer.is_null() {
        drop(unsafe { CString::from_raw(buffer) });
    }
}
