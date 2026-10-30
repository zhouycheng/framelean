use crate::{Error, Result};
use fll_contracts::{ABI_VERSION, FllRequest, JSON_VERSION, OperationEvent};
use libloading::Library;
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
const MAX_FRAME: usize = 1024 * 1024;
struct ManagedChild(Child);
impl std::ops::Deref for ManagedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}
impl std::ops::DerefMut for ManagedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
unsafe fn buffer(lib: &Library, p: *mut c_char) -> Result<Value> {
    if p.is_null() {
        return Err(Error::new(
            "fll_protocol",
            "host",
            "FLL returned null buffer",
        ));
    }
    let free = unsafe { lib.get::<unsafe extern "C" fn(*mut c_char)>(b"fll_free\0") }
        .map_err(component)?;
    let bytes = unsafe { CStr::from_ptr(p) }.to_bytes();
    let data = if bytes.len() <= MAX_FRAME {
        Some(bytes.to_vec())
    } else {
        None
    };
    unsafe { free(p) };
    serde_json::from_slice(
        &data.ok_or_else(|| Error::new("fll_protocol", "host", "FLL result exceeds 1 MiB"))?,
    )
    .map_err(Into::into)
}
fn component(e: impl ToString) -> Error {
    Error::new("component_unavailable", "host", e)
}
fn line(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut data = Vec::new();
    let n = reader
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut data)?;
    if n == 0 {
        return Ok(None);
    }
    if n > MAX_FRAME || data.last() != Some(&b'\n') {
        return Err(io::Error::other("Host frame exceeds limit or is truncated"));
    }
    String::from_utf8(data).map(Some).map_err(io::Error::other)
}
pub fn entry(path: &Path, probe: bool) -> Result<()> {
    unsafe {
        let lib = Library::new(path).map_err(component)?;
        let version = lib
            .get::<unsafe extern "C" fn() -> u32>(b"fll_abi_version\0")
            .map_err(component)?;
        if version() != ABI_VERSION {
            return Err(Error::new("abi_mismatch", "host", "FLL ABI 不兼容"));
        }
        let versions = lib
            .get::<unsafe extern "C" fn() -> *mut c_char>(b"fll_versions\0")
            .map_err(component)?;
        let info = buffer(&lib, versions())?;
        if info["json_version"] != JSON_VERSION {
            return Err(Error::new(
                "protocol_mismatch",
                "host",
                "FLL JSON 契约不兼容，需要 schema 2",
            ));
        }
        if info["capabilities"]["backend_versions"]["compatible"] == false {
            return Err(Error::new(
                "backend_version_mismatch",
                "host",
                "libav 主版本与 FLL 不匹配",
            ));
        }
        if probe {
            println!("{info}");
            return Ok(());
        }
        println!("{}", json!({"type":"ready","versions":info}));
        io::stdout().flush()?;
        let mut input = BufReader::new(io::stdin());
        let raw = line(&mut input)?.ok_or_else(|| component("control channel closed"))?;
        let request = CString::new(raw.trim()).map_err(component)?;
        let start = lib
            .get::<unsafe extern "C" fn(*const c_char) -> *mut c_void>(b"fll_start\0")
            .map_err(component)?;
        let poll = lib
            .get::<unsafe extern "C" fn(*const c_void) -> *mut c_char>(b"fll_poll\0")
            .map_err(component)?;
        let cancel = *lib
            .get::<unsafe extern "C" fn(*const c_void)>(b"fll_cancel\0")
            .map_err(component)?;
        let release = lib
            .get::<unsafe extern "C" fn(*mut c_void)>(b"fll_release\0")
            .map_err(component)?;
        let handle = start(request.as_ptr());
        if handle.is_null() {
            return Err(component("FLL start returned null"));
        }
        let live = Arc::new(Mutex::new(handle as usize));
        let control = live.clone();
        thread::spawn(move || {
            loop {
                match line(&mut input) {
                    Ok(Some(v)) if v.trim() == "cancel" => {
                        let p = control.lock().unwrap();
                        if *p != 0 {
                            cancel(*p as *const c_void);
                        }
                    }
                    Ok(Some(_)) => {}
                    _ => std::process::exit(3),
                }
            }
        });
        loop {
            // A malformed live operation must terminate the Host before unloading its library.
            let read = (|| -> Result<(Value, OperationEvent)> {
                let value = buffer(&lib, poll(handle))?;
                let event = serde_json::from_value(value.clone())?;
                Ok((value, event))
            })();
            let (value, event) = read.unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(4);
            });
            println!("{value}");
            if io::stdout().flush().is_err() {
                std::process::exit(3);
            }
            if event.terminal() {
                let mut p = live.lock().unwrap();
                *p = 0;
                release(handle);
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
}

pub fn run(
    binary: &Path,
    library: &Path,
    log_dir: &Path,
    request: &FllRequest,
    cancel: &AtomicBool,
    timeout: Option<Duration>,
    mut progress: impl FnMut(u32, &OperationEvent),
) -> Result<Value> {
    let rotation = LOG_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    fs::create_dir_all(log_dir)?;
    let log_path = log_dir.join("host.log");
    if fs::metadata(&log_path).is_ok_and(|m| m.len() > 8 * 1024 * 1024) {
        let _ = fs::remove_file(log_dir.join("host.log.2"));
        let _ = fs::rename(log_dir.join("host.log.1"), log_dir.join("host.log.2"));
        let _ = fs::rename(&log_path, log_dir.join("host.log.1"));
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    drop(rotation);
    let mut child = ManagedChild(
        Command::new(binary)
            .arg("host")
            .arg(library)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn()?,
    );
    let mut stdin = child.stdin.take().unwrap();
    let raw = serde_json::to_vec(request)?;
    if raw.len() + 1 > MAX_FRAME {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::new("request_size", "host", "媒体请求超过 1 MiB"));
    }
    if let Err(e) = stdin.write_all(&raw).and_then(|_| stdin.write_all(b"\n")) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e.into());
    }
    let stdout = child.stdout.take().unwrap();
    let observed = Arc::new(Mutex::new((None::<Value>, None::<String>, false)));
    let sink = observed.clone();
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            match line(&mut reader) {
                Ok(Some(s)) => match serde_json::from_str::<Value>(&s) {
                    Ok(v) => {
                        let mut state = sink.lock().unwrap();
                        if v["type"] == "ready" {
                            state.2 = true;
                        } else {
                            state.0 = Some(v);
                        }
                    }
                    Err(e) => {
                        sink.lock().unwrap().1 = Some(e.to_string());
                        break;
                    }
                },
                Ok(None) => break,
                Err(e) => {
                    sink.lock().unwrap().1 = Some(e.to_string());
                    break;
                }
            }
        }
    });
    let start = Instant::now();
    let mut ready_at = None;
    let mut cancel_at = None;
    let mut failure = None;
    loop {
        let (last, error, ready) = observed.lock().unwrap().clone();
        if ready && ready_at.is_none() {
            ready_at = Some(Instant::now());
        }
        if let Some(e) = error {
            failure = Some(Error::new("host_protocol", "host", e));
        }
        if let Some(v) = last {
            if v["type"] == "host_error" {
                failure = Some(Error(serde_json::from_value(v["error"].clone())?));
            } else {
                match serde_json::from_value::<OperationEvent>(v) {
                    Ok(event) if event.operation_id == request.operation_id => {
                        progress(child.id(), &event)
                    }
                    _ => {
                        failure = Some(Error::new(
                            "host_protocol",
                            "host",
                            "Host event identity or schema mismatch",
                        ))
                    }
                }
            }
        }
        // A newly copied signed bundle can spend time in the OS loader before ready.
        if (!ready && start.elapsed() > Duration::from_secs(30))
            || ready_at.is_some_and(|at| timeout.is_some_and(|t| at.elapsed() > t))
        {
            failure = Some(Error::new("host_timeout", "host", "Host 超过等待期限"));
        }
        if cancel.load(Ordering::SeqCst) {
            if cancel_at.is_none() {
                let _ = writeln!(stdin, "cancel");
                cancel_at = Some(Instant::now());
            }
            if cancel_at.unwrap().elapsed() >= Duration::from_secs(3) {
                let _ = child.kill();
            }
        }
        if failure.is_some() {
            let _ = child.kill();
        }
        if let Some(status) = child.try_wait()? {
            let _ = reader.join();
            let last = observed.lock().unwrap().0.clone();
            if cancel.load(Ordering::SeqCst) {
                return Err(Error::new("canceled", "host", "任务已取消"));
            }
            if let Some(e) = failure {
                return Err(e);
            }
            let last = last.ok_or_else(|| {
                Error::new("host_crashed", "host", format!("Host exited: {status}"))
            })?;
            if last["type"] == "host_error" {
                return Err(Error(serde_json::from_value(last["error"].clone())?));
            }
            let event: OperationEvent = serde_json::from_value(last)?;
            if !status.success() || !event.terminal() || event.operation_id != request.operation_id
            {
                return Err(Error::new(
                    "host_crashed",
                    "host",
                    format!("Host exited without valid terminal result: {status}"),
                ));
            }
            if let Some(e) = event.error {
                return Err(e.into());
            }
            return event
                .result
                .ok_or_else(|| Error::new("host_protocol", "host", "终态缺少结果"));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
static LOG_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
