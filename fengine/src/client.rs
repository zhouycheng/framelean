use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
struct Endpoint {
    protocol_version: u32,
    url: String,
    token: String,
}
pub struct Client {
    endpoint: Endpoint,
    http: ureq::Agent,
}
impl Client {
    pub fn connect(start: bool) -> Result<Self> {
        let discover = || -> Result<Self> {
            let bytes = fs::read(crate::runtime_dir().join("endpoint.json"))
                .map_err(|e| Error::new("worker_unavailable", "connect", e))?;
            let endpoint: Endpoint = serde_json::from_slice(&bytes)?;
            if endpoint.protocol_version != fll_contracts::HTTP_VERSION {
                return Err(Error::new(
                    "protocol_mismatch",
                    "connect",
                    "Worker 协议不匹配，请先退出旧版后台",
                ));
            }
            let port = endpoint
                .url
                .strip_prefix("http://127.0.0.1:")
                .and_then(|s| s.parse::<u16>().ok())
                .filter(|p| *p > 0);
            if port.is_none() {
                return Err(Error::new(
                    "endpoint_invalid",
                    "connect",
                    "本地端点地址无效",
                ));
            }
            let http: ureq::Agent = ureq::Agent::config_builder()
                .proxy(None)
                .max_redirects(0)
                .http_status_as_error(false)
                .timeout_connect(Some(Duration::from_secs(2)))
                .timeout_global(Some(Duration::from_secs(60)))
                .build()
                .into();
            let client = Self { endpoint, http };
            client.call("health", None)?;
            Ok(client)
        };
        match discover() {
            Ok(c) => return Ok(c),
            Err(e) if e.code == "protocol_mismatch" || !start => return Err(e),
            Err(_) => {}
        }
        Self::start_worker()?;
        let until = Instant::now() + Duration::from_secs(15);
        while Instant::now() < until {
            match discover() {
                Ok(c) => return Ok(c),
                Err(e) if e.code == "protocol_mismatch" => return Err(e),
                Err(_) => thread::sleep(Duration::from_millis(100)),
            }
        }
        Err(Error::new(
            "worker_unavailable",
            "connect",
            "Worker 启动超时，请查看运行日志",
        ))
    }
    pub fn start_worker() -> Result<()> {
        let binary = std::env::var_os("FRAMELEAN_ENGINE")
            .map(PathBuf::from)
            .unwrap_or(
                std::env::current_exe()?
                    .parent()
                    .unwrap()
                    .join(if cfg!(windows) {
                        "framelean-engine.exe"
                    } else {
                        "framelean-engine"
                    }),
            );
        crate::service_runtime::private_directory(&crate::runtime_dir())?;
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(crate::runtime_dir().join("worker.log"))?;
        let mut command = Command::new(binary);
        command.arg("worker");
        if let Some(library) = std::env::var_os("FRAMELEAN_FLL") {
            command.arg(library);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x00000008 | 0x00000200);
        }
        command.spawn()?;
        Ok(())
    }
    pub fn call(&self, name: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{}/v4/{name}", self.endpoint.url);
        let auth = format!("Bearer {}", self.endpoint.token);
        let response = if let Some(mut body) = body {
            if body["request_id"].is_null() {
                body["request_id"] = json!(crate::uid());
            }
            self.http
                .post(&url)
                .header("Authorization", &auth)
                .send_json(body)
        } else {
            self.http.get(&url).header("Authorization", &auth).call()
        };
        let mut response =
            response.map_err(|e| Error::new("worker_unavailable", "transport", e))?;
        let status = response.status().as_u16();
        let value: Value = response
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_json()
            .map_err(|e| Error::new("response_invalid", "transport", e))?;
        if status >= 400 {
            return Err(
                serde_json::from_value(value["error"].clone()).unwrap_or_else(|_| {
                    Error::new("http_error", "transport", format!("HTTP {status}"))
                }),
            );
        }
        Ok(value)
    }
    pub fn wait_ready(&self) -> Result<Value> {
        let start = Instant::now();
        loop {
            let h = self.call("health", None)?;
            match h["lifecycle"].as_str() {
                Some("ready") => return Ok(h),
                Some("degraded") => {
                    return Err(
                        Error::new("component_unavailable", "startup", "后台处于降级状态")
                            .details(h),
                    );
                }
                _ => {}
            }
            if start.elapsed() > Duration::from_secs(45) {
                return Err(Error::new(
                    "recovery_pending",
                    "startup",
                    "后台尚在恢复，可用 doctor 查看状态",
                ));
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
}
