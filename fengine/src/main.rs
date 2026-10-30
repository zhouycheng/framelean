use fs2::FileExt;
use libloading::Library;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, UNIX_EPOCH},
};
use tiny_http::{Header, Response, Server, StatusCode};
use uuid::Uuid;
const PROTOCOL: u32 = 1;
fn uid() -> String {
    Uuid::new_v4().to_string()
}
fn lib_name() -> &'static str {
    if cfg!(windows) {
        "framelean_fll.dll"
    } else if cfg!(target_os = "macos") {
        "libframelean_fll.dylib"
    } else {
        "libframelean_fll.so"
    }
}
fn runtime_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("FRAMELEAN_RUNTIME_DIR") {
        return p.into();
    }
    if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA missing"))
            .join("FrameLean/runtime")
    } else {
        PathBuf::from(std::env::var_os("HOME").expect("HOME missing"))
            .join("Library/Application Support/FrameLean/runtime")
    }
}
fn sibling() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(lib_name())
}
fn err(code: &str, message: impl ToString) -> Value {
    json!({"code":code,"message":message.to_string()})
}
fn fingerprint(path: &str) -> Result<Value, String> {
    let m = fs::metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() {
        return Err("输入不是普通文件".into());
    }
    Ok(
        json!({"size":m.len(),"modified_ns":m.modified().map_err(|e|e.to_string())?.duration_since(UNIX_EPOCH).map_err(|e|e.to_string())?.as_nanos().to_string()}),
    )
}
unsafe fn json_buffer(lib: &Library, ptr: *mut c_char) -> Result<Value, String> {
    if ptr.is_null() {
        return Err("FLL returned null JSON".into());
    }
    let data = unsafe { CStr::from_ptr(ptr) }.to_bytes().to_vec();
    let free = unsafe { lib.get::<unsafe extern "C" fn(*mut c_char)>(b"fll_free\0") }
        .map_err(|e| e.to_string())?;
    unsafe { free(ptr) };
    serde_json::from_slice(&data).map_err(|e| e.to_string())
}
fn host(path: &Path, probe: bool) -> Result<(), String> {
    unsafe {
        let lib = Library::new(path).map_err(|e| format!("FLL 加载失败：{e}"))?;
        let version = lib
            .get::<unsafe extern "C" fn() -> u32>(b"fll_abi_version\0")
            .map_err(|e| e.to_string())?;
        if version() != PROTOCOL {
            return Err("FLL ABI 不兼容，需要版本 1".into());
        }
        let versions = lib
            .get::<unsafe extern "C" fn() -> *mut c_char>(b"fll_versions\0")
            .map_err(|e| e.to_string())?;
        if probe {
            println!("{}", json_buffer(&lib, versions())?);
            return Ok(());
        }
        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        let req = CString::new(line.trim()).map_err(|e| e.to_string())?;
        let start = lib
            .get::<unsafe extern "C" fn(*const c_char) -> *mut c_void>(b"fll_start\0")
            .map_err(|e| e.to_string())?;
        let poll = lib
            .get::<unsafe extern "C" fn(*const c_void) -> *mut c_char>(b"fll_poll\0")
            .map_err(|e| e.to_string())?;
        let cancel = *lib
            .get::<unsafe extern "C" fn(*const c_void)>(b"fll_cancel\0")
            .map_err(|e| e.to_string())?;
        let release = lib
            .get::<unsafe extern "C" fn(*mut c_void)>(b"fll_release\0")
            .map_err(|e| e.to_string())?;
        let handle = start(req.as_ptr());
        if handle.is_null() {
            return Err("FLL 请求无效".into());
        }
        let live_handle = Arc::new(Mutex::new(handle as usize));
        let control_handle = live_handle.clone();
        // Control-channel loss means the owning Worker is gone. Never leave an orphan encoder.
        thread::spawn(move || {
            let mut reader = BufReader::new(io::stdin());
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => std::process::exit(3),
                    Ok(_) => {
                        if line.trim() == "cancel" {
                            let pointer = control_handle.lock().unwrap();
                            if *pointer != 0 {
                                cancel(*pointer as *const c_void);
                            }
                        }
                    }
                }
            }
        });
        loop {
            let state = json_buffer(&lib, poll(handle))?;
            println!("{state}");
            io::stdout().flush().map_err(|e| e.to_string())?;
            if terminal(&state) {
                let mut pointer = live_handle.lock().unwrap();
                *pointer = 0;
                release(handle);
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
}
fn terminal(s: &Value) -> bool {
    matches!(
        s["state"].as_str(),
        Some("succeeded" | "failed" | "canceled")
    )
}
struct Active {
    child: Arc<Mutex<Child>>,
    cancel: Arc<AtomicBool>,
}
struct State {
    seq: u64,
    task: Value,
    analysis: HashMap<String, Value>,
    active: Option<Active>,
}
struct App {
    instance_id: String,
    state: Mutex<State>,
    token: String,
    library: PathBuf,
    dir: PathBuf,
    health: Value,
    shutdown: AtomicBool,
}
impl App {
    fn snapshot(&self) -> Value {
        let s = self.state.lock().unwrap();
        json!({"protocol_version":PROTOCOL,"instance_id":self.instance_id,"seq":s.seq,"task":s.task,"health":self.health})
    }
    fn update(&self, id: &str, event: Value) {
        let mut s = self.state.lock().unwrap();
        if s.task["task_id"] != id {
            return;
        }
        if let Some(m) = event.as_object() {
            for (k, v) in m {
                s.task[k] = v.clone();
            }
        }
        s.seq += 1;
    }
    fn cancel(&self, id: &str) -> Result<(), String> {
        let s = self.state.lock().unwrap();
        if s.task["task_id"] != id {
            return Err("任务不存在".into());
        }
        if let Some(a) = &s.active {
            a.cancel.store(true, Ordering::SeqCst);
            if let Some(stdin) = a.child.lock().unwrap().stdin.as_mut() {
                let _ = writeln!(stdin, "cancel");
            }
        }
        Ok(())
    }
    fn submit(self: &Arc<Self>, kind: &str, r: &Value) -> Result<Value, String> {
        if self.health["available"] != true {
            return Err("媒体组件不可用，请查看 health 诊断".into());
        }
        let request_id = r["request_id"]
            .as_str()
            .filter(|x| !x.is_empty() && x.len() < 128)
            .ok_or("需要 request_id（1–127 字符）")?;
        let mut s = self.state.lock().unwrap();
        if self.shutdown.load(Ordering::SeqCst) {
            return Err("Worker 正在退出".into());
        }
        if s.task["request_id"] == request_id {
            return Ok(s.task.clone());
        }
        if s.active.is_some() {
            return Err("已有任务执行中".into());
        }
        let (input, fp, analysis) = if kind == "analyze" {
            let p = r["input"].as_str().ok_or("缺少 input")?;
            let path = fs::canonicalize(p).map_err(|e| e.to_string())?;
            let input = path.to_str().ok_or("文件路径必须为 Unicode")?.to_string();
            let fp = fingerprint(&input)?;
            (input, fp, Value::Null)
        } else {
            let id = r["analysis_id"].as_str().ok_or("缺少 analysis_id")?;
            let a = s
                .analysis
                .get(id)
                .ok_or("分析记录不存在，请重新分析")?
                .clone();
            let p = a["input"].as_str().unwrap().to_string();
            let fp = fingerprint(&p)?;
            if fp != a["fingerprint"] {
                return Err("源文件已变化，请重新分析".into());
            }
            (p, fp, a)
        };
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("host.log"))
            .map_err(|e| e.to_string())?;
        let id = uid();
        let temp = if kind == "execute" {
            let output = r["output_dir"].as_str().ok_or("缺少 output_dir")?;
            let dir = fs::canonicalize(output).map_err(|e| e.to_string())?;
            if !dir.is_dir() {
                return Err("输出位置不是目录".into());
            }
            let p = dir.join(format!(".framelean-{id}.partial.mp4"));
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&p)
                .map_err(|e| e.to_string())?;
            Some(p)
        } else {
            None
        };
        let spawned = Command::new(std::env::current_exe().unwrap())
            .arg("host")
            .arg(&self.library)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                if let Some(p) = &temp {
                    let _ = fs::remove_file(p);
                }
                return Err(e.to_string());
            }
        };
        let job = json!({"kind":kind,"input":input,"temp":temp.as_ref().map(|p|p.to_string_lossy().into_owned())});
        if let Err(e) = writeln!(child.stdin.as_mut().unwrap(), "{job}") {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(p) = &temp {
                let _ = fs::remove_file(p);
            }
            return Err(e.to_string());
        }
        let stdout = child.stdout.take().unwrap();
        let child = Arc::new(Mutex::new(child));
        let canceled = Arc::new(AtomicBool::new(false));
        s.task = json!({"task_id":id,"request_id":request_id,"kind":kind,"state":"running","input":input,"fingerprint":fp,"progress":0.0,"host_pid":child.lock().unwrap().id(),"analysis":analysis,"analysis_id":r["analysis_id"]});
        s.seq += 1;
        s.active = Some(Active {
            child: child.clone(),
            cancel: canceled.clone(),
        });
        let mut response = s.task.clone();
        response["seq"] = json!(s.seq);
        response["instance_id"] = json!(self.instance_id);
        drop(s);
        let app = self.clone();
        let kind = kind.to_string();
        thread::spawn(move || {
            let app_events = app.clone();
            let event_id = id.clone();
            let reader = thread::spawn(move || {
                let mut last = Value::Null;
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        if !terminal(&v) {
                            app_events.update(&event_id, v.clone());
                        }
                        last = v;
                    }
                }
                last
            });
            let mut cancel_at = None;
            let status = loop {
                if canceled.load(Ordering::SeqCst) {
                    let since = cancel_at.get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() >= Duration::from_secs(3) {
                        let _ = child.lock().unwrap().kill();
                    }
                }
                match child.lock().unwrap().try_wait() {
                    Ok(Some(s)) => break Some(s),
                    Ok(None) => {}
                    Err(_) => break None,
                }
                thread::sleep(Duration::from_millis(50));
            };
            let mut result = reader.join().unwrap_or(Value::Null);
            let mut state = app.state.lock().unwrap();
            if canceled.load(Ordering::SeqCst) {
                result = json!({"state":"canceled","error":err("canceled","任务已取消")});
            } else if !status.is_some_and(|s| s.success()) || !terminal(&result) {
                result = json!({"state":"failed","error":err("host_exit","处理宿主异常退出，请查看 host.log")});
            }
            if result["state"] == "succeeded" {
                if fingerprint(&input).ok().as_ref() != Some(&fp) {
                    result = json!({"state":"failed","error":err("source_changed","处理期间源文件已变化")});
                } else if let Some(p) = &temp {
                    match publish(p, &input) {
                        Ok(dest) => {
                            result["output"] = json!(dest.to_string_lossy());
                            result["output_bytes"] =
                                json!(fs::metadata(dest).map(|m| m.len()).unwrap_or(0));
                        }
                        Err(e) => {
                            result = json!({"state":"failed","error":err("publish_failed",e)})
                        }
                    }
                } else if kind == "analyze" {
                    result["analysis_id"] = json!(id);
                    state.analysis.clear();
                    state.analysis.insert(
                        id.clone(),
                        json!({"input":input,"fingerprint":fp,"media":result["result"]}),
                    );
                }
            }
            if let Some(p) = temp {
                let _ = fs::remove_file(p);
            }
            // Commit terminal state and release admission in one lock, so retries cannot race finalization.
            if let Some(o) = result.as_object() {
                for (k, v) in o {
                    state.task[k] = v.clone();
                }
            }
            state.active = None;
            state.seq += 1;
        });
        Ok(response)
    }
}
fn publish(temp: &Path, input: &str) -> Result<PathBuf, String> {
    OpenOptions::new()
        .read(true)
        .open(temp)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    let name = Path::new(input)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    for n in 0..10_000 {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!("-{n}")
        };
        let dest = temp
            .parent()
            .unwrap()
            .join(format!("{name}-compressed{suffix}.mp4"));
        match fs::hard_link(temp, &dest) {
            Ok(_) => return Ok(dest),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("无法安全发布（输出目录需支持硬链接）：{e}")),
        }
    }
    Err("输出重名次数过多".into())
}
fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).unwrap()
}
fn reply(req: tiny_http::Request, code: u16, v: Value) {
    let _ = req.respond(
        Response::from_string(v.to_string())
            .with_status_code(StatusCode(code))
            .with_header(header("Content-Type", "application/json; charset=utf-8"))
            .with_header(header("Cache-Control", "no-store")),
    );
}
fn stream_events(app: Arc<App>, req: tiny_http::Request) -> io::Result<()> {
    let mut writer = req.into_writer();
    writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")?;
    writer.flush()?;
    let mut last = None;
    while !app.shutdown.load(Ordering::SeqCst) {
        let snapshot = app.snapshot();
        let seq = snapshot["seq"].as_u64().unwrap();
        let data = if last != Some(seq) {
            last = Some(seq);
            format!("id: {seq}\ndata: {snapshot}\n\n")
        } else {
            ": keepalive\n\n".into()
        };
        write!(writer, "{:x}\r\n{}\r\n", data.len(), data)?;
        writer.flush()?;
        thread::sleep(Duration::from_millis(200));
    }
    writer.write_all(b"0\r\n\r\n")?;
    writer.flush()
}
fn handle(app: Arc<App>, mut req: tiny_http::Request) {
    if req.headers().iter().any(|h| h.field.equiv("Origin")) {
        reply(
            req,
            403,
            json!({"error":err("origin_denied","仅支持原生本地客户端")}),
        );
        return;
    }
    let expected = format!("Bearer {}", app.token);
    if !req
        .headers()
        .iter()
        .any(|h| h.field.equiv("Authorization") && h.value.as_str() == expected)
    {
        reply(
            req,
            401,
            json!({"error":err("unauthorized","本地凭证无效")}),
        );
        return;
    }
    let path = req.url().to_string();
    let method = req.method().as_str();
    if method == "GET" {
        match path.as_str() {
            "/v1/health" => reply(
                req,
                200,
                json!({"protocol_version":PROTOCOL,"pid":std::process::id(),"components":app.health}),
            ),
            "/v1/snapshot" => reply(req, 200, app.snapshot()),
            "/v1/events" => {
                let _ = stream_events(app, req);
            }
            _ => reply(req, 404, json!({"error":err("not_found","未知接口")})),
        }
        return;
    }
    if method != "POST" {
        reply(req, 405, json!({"error":err("method","不支持的方法")}));
        return;
    }
    let mut bytes = Vec::new();
    if req.as_reader().take(65537).read_to_end(&mut bytes).is_err() || bytes.len() > 65536 {
        reply(
            req,
            413,
            json!({"error":err("request_size","请求过大或无法读取")}),
        );
        return;
    }
    let Ok(r) = serde_json::from_slice::<Value>(&bytes) else {
        reply(
            req,
            400,
            json!({"error":err("invalid_json","请求不是 JSON")}),
        );
        return;
    };
    let result = match path.as_str() {
        "/v1/analyze" => app.submit("analyze", &r),
        "/v1/execute" => app.submit("execute", &r),
        "/v1/cancel" => app
            .cancel(r["task_id"].as_str().unwrap_or(""))
            .map(|_| json!({"accepted":true})),
        "/v1/shutdown" => {
            app.shutdown.store(true, Ordering::SeqCst);
            let id = app.snapshot()["task"]["task_id"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let _ = app.cancel(&id);
            Ok(json!({"accepted":true}))
        }
        _ => Err("未知接口".into()),
    };
    match result {
        Ok(mut v) => {
            v["request_id"] = r["request_id"].clone();
            reply(req, 200, v)
        }
        Err(e) => reply(
            req,
            400,
            json!({"request_id":r["request_id"],"error":err("request_rejected",e)}),
        ),
    }
}
fn worker(library: PathBuf) -> Result<(), String> {
    let dir = runtime_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("worker.lock"))
        .map_err(|e| e.to_string())?;
    if lock.try_lock_exclusive().is_err() {
        return Ok(());
    }
    // Probe in a disposable Host, preserving the Worker's native-library isolation.
    let probe = Command::new(std::env::current_exe().unwrap())
        .arg("probe")
        .arg(&library)
        .output()
        .map_err(|e| e.to_string())?;
    let health = if probe.status.success() {
        let versions: Value = serde_json::from_slice(&probe.stdout).map_err(|e| e.to_string())?;
        json!({"available":versions["encoder"]==true,"library":library,"versions":versions})
    } else {
        json!({"available":false,"error":String::from_utf8_lossy(&probe.stderr)})
    };
    let server = Server::http("127.0.0.1:0").map_err(|e| e.to_string())?;
    let app = Arc::new(App {
        instance_id: uid(),
        state: Mutex::new(State {
            seq: 0,
            task: Value::Null,
            analysis: HashMap::new(),
            active: None,
        }),
        token: uid(),
        library,
        dir: dir.clone(),
        health,
        shutdown: AtomicBool::new(false),
    });
    let endpoint = json!({"protocol_version":PROTOCOL,"url":format!("http://{}",server.server_addr()),"token":app.token,"pid":std::process::id()});
    let endpoint_path = dir.join("endpoint.json");
    fs::write(&endpoint_path, endpoint.to_string()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&endpoint_path, fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    while !app.shutdown.load(Ordering::SeqCst) {
        if let Some(req) = server
            .recv_timeout(Duration::from_millis(100))
            .map_err(|e| e.to_string())?
        {
            let app = app.clone();
            thread::spawn(move || handle(app, req));
        }
    }
    while app.state.lock().unwrap().active.is_some() {
        thread::sleep(Duration::from_millis(100));
    }
    let _ = fs::remove_file(endpoint_path);
    drop(lock);
    Ok(())
}
fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or("worker".into());
    let path = args.next().map(PathBuf::from).unwrap_or_else(sibling);
    let result = match mode.as_str() {
        "host" => host(&path, false),
        "probe" => host(&path, true),
        "worker" => worker(path),
        _ => Err("usage: framelean-engine [worker|host|probe] [FLL path]".into()),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publish_never_overwrites() {
        let d = std::env::temp_dir().join(uid());
        fs::create_dir(&d).unwrap();
        let p = d.join("temp");
        fs::write(&p, b"new").unwrap();
        fs::write(d.join("source-compressed.mp4"), b"old").unwrap();
        let out = publish(&p, "source.mp4").unwrap();
        assert_eq!(fs::read(d.join("source-compressed.mp4")).unwrap(), b"old");
        assert_eq!(fs::read(out).unwrap(), b"new");
        fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn fingerprint_rejects_directory() {
        assert!(fingerprint(std::env::temp_dir().to_str().unwrap()).is_err());
    }
}
