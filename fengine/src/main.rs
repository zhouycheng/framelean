mod fll;

use std::env;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

const PROTOCOL_VERSION: u16 = 1;
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const ENDPOINT_VERSION: u16 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct EndpointFile {
    schema_version: u16,
    state: String,
    pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("framelean-engine: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    match args.next().as_deref() {
        Some(arg) if arg == "--version" => {
            println!("framelean-engine {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(arg) if arg == "serve" => {
            let flag = args.next().ok_or("serve requires --state-dir <path>")?;
            if flag != "--state-dir" {
                return Err("serve requires --state-dir <path>".into());
            }
            let state_dir = PathBuf::from(args.next().ok_or("missing state directory")?);
            if args.next().is_some() {
                return Err("unexpected serve argument".into());
            }
            serve(&state_dir)
        }
        _ => Err("usage: framelean-engine --version | serve --state-dir <path>".into()),
    }
}

fn serve(state_dir: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(state_dir)?;
    let endpoint_path = state_dir.join("endpoint.json");
    let lock_path = state_dir.join("engine.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    lock.try_lock_exclusive().map_err(|error| {
        if error.kind() == io::ErrorKind::WouldBlock {
            "FrameLean Work service is already running".to_owned()
        } else {
            format!("could not lock the Work service state: {error}")
        }
    })?;

    let pid = std::process::id();
    let token = Uuid::new_v4().simple().to_string();
    write_endpoint(
        &endpoint_path,
        &EndpointFile {
            schema_version: ENDPOINT_VERSION,
            state: "starting".to_owned(),
            pid,
            port: None,
            token: None,
            error: None,
        },
    )?;

    let mut runtime = match fll::Runtime::load_default() {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = write_endpoint(
                &endpoint_path,
                &EndpointFile {
                    schema_version: ENDPOINT_VERSION,
                    state: "failed".to_owned(),
                    pid,
                    port: None,
                    token: None,
                    error: Some(error.to_string()),
                },
            );
            return Err(error.into());
        }
    };
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener,
        Err(error) => {
            let _ = write_endpoint(
                &endpoint_path,
                &EndpointFile {
                    schema_version: ENDPOINT_VERSION,
                    state: "failed".to_owned(),
                    pid,
                    port: None,
                    token: None,
                    error: Some(format!("could not bind loopback endpoint: {error}")),
                },
            );
            return Err(error.into());
        }
    };
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    write_endpoint(
        &endpoint_path,
        &EndpointFile {
            schema_version: ENDPOINT_VERSION,
            state: "ready".to_owned(),
            pid,
            port: Some(port),
            token: Some(token.clone()),
            error: None,
        },
    )?;
    eprintln!("FrameLean Work listening on 127.0.0.1:{port}");

    let mut shutdown = false;
    while !shutdown {
        match listener.accept() {
            Ok((mut stream, _)) => match handle_connection(&mut stream, &token, &mut runtime) {
                Ok(should_shutdown) => shutdown = should_shutdown,
                Err(error) => eprintln!("FrameLean Work request failed: {error}"),
            },
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error.into()),
        }
    }

    let _ = fs::remove_file(endpoint_path);
    FileExt::unlock(&lock)?;
    drop(lock);
    Ok(())
}

fn handle_connection(
    stream: &mut TcpStream,
    expected_token: &str,
    runtime: &mut fll::Runtime,
) -> Result<bool, Box<dyn Error>> {
    // macOS accepted sockets inherit the listener's nonblocking mode.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;

    let hello: Value = match read_json_frame(stream) {
        Ok(value) => value,
        Err(error) => {
            write_error_response(stream, "", "invalid_request", error.to_string())?;
            return Ok(false);
        }
    };
    let hello_id = request_id(&hello);
    if hello["protocol_version"] != PROTOCOL_VERSION
        || hello["command"] != "hello"
        || hello_id.is_empty()
    {
        write_error_response(
            stream,
            &hello_id,
            "invalid_request",
            "invalid hello envelope",
        )?;
        return Ok(false);
    }
    if hello["payload"]["token"].as_str() != Some(expected_token) {
        write_error_response(
            stream,
            &hello_id,
            "unauthorized",
            "Work authentication failed",
        )?;
        return Ok(false);
    }
    write_frame(
        stream,
        &json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": hello_id,
            "output": "response",
            "ok": true,
            "result": {"work_protocol_version": PROTOCOL_VERSION},
        }),
    )?;

    let request: Value = match read_json_frame(stream) {
        Ok(request) => request,
        Err(error) => {
            write_error_response(stream, "", "invalid_request", error.to_string())?;
            return Ok(false);
        }
    };
    let id = request_id(&request);
    if request["protocol_version"] != PROTOCOL_VERSION || id.is_empty() {
        write_error_response(
            stream,
            &id,
            "invalid_request",
            "invalid protocol version or request_id",
        )?;
        return Ok(false);
    }

    let command = request["command"].as_str().unwrap_or_default();
    let payload = request.get("payload").cloned().unwrap_or(Value::Null);
    let result = match command {
        "ping" => runtime
            .invoke("ping", json!({}))
            .map(|result| (result, false)),
        "analyze_media" => runtime
            .invoke("analyze_media", payload)
            .map(|result| (result, false)),
        "shutdown" => Ok((json!({"stopping": true}), true)),
        _ => Err(fll::InvokeError {
            code: "unsupported_command".to_owned(),
            message: format!("unsupported Work command: {command}"),
            retryable: false,
        }),
    };

    match result {
        Ok((value, should_shutdown)) => {
            write_frame(
                stream,
                &json!({
                    "protocol_version": PROTOCOL_VERSION,
                    "request_id": id,
                    "output": "response",
                    "ok": true,
                    "result": value,
                }),
            )?;
            Ok(should_shutdown)
        }
        Err(error) => {
            write_frame(
                stream,
                &json!({
                    "protocol_version": PROTOCOL_VERSION,
                    "request_id": id,
                    "output": "error",
                    "ok": false,
                    "error": {
                        "code": if error.code == "unsupported_command" { "unsupported_command" } else { "engine_failure" },
                        "engine_code": error.code,
                        "message": error.message,
                        "retryable": error.retryable,
                    },
                }),
            )?;
            Ok(false)
        }
    }
}

fn request_id(request: &Value) -> String {
    request
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .unwrap_or_default()
        .to_owned()
}

fn write_error_response(
    stream: &mut TcpStream,
    request_id: &str,
    code: &str,
    message: impl Into<String>,
) -> io::Result<()> {
    write_frame(
        stream,
        &json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": request_id,
            "output": "error",
            "ok": false,
            "error": {"code": code, "message": message.into(), "retryable": false},
        }),
    )
}

fn read_json_frame(stream: &mut TcpStream) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&read_frame(stream)?)?)
}

fn read_frame(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame length {length} is outside 1..={MAX_FRAME_BYTES}"),
        ));
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn write_frame(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("outgoing frame is {} bytes", bytes.len()),
        ));
    }
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

fn write_endpoint(path: &Path, endpoint: &EndpointFile) -> io::Result<()> {
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(endpoint)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn framed_json_round_trips_with_big_endian_length() {
        let value = json!({"protocol_version": 1, "command": "ping"});
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &value).unwrap();
        assert_eq!(
            u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize,
            bytes.len() - 4
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&read_frame(&mut Cursor::new(bytes)).unwrap()).unwrap(),
            value
        );
    }

    #[test]
    fn empty_and_oversized_frames_are_rejected() {
        assert!(read_frame(&mut Cursor::new(0u32.to_be_bytes())).is_err());
        let header = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes();
        assert!(read_frame(&mut Cursor::new(header)).is_err());
    }
}
