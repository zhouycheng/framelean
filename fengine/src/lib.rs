pub mod client;
mod coordinator;
mod events;
mod gateway;
mod host;
mod publisher;
mod queries;
mod repository;
mod service_runtime;
mod storage;
use fll_contracts::MediaError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Error(pub MediaError);
impl Error {
    pub fn new(code: &str, stage: &str, message: impl ToString) -> Self {
        Self(MediaError::new(code, stage, message))
    }
    pub fn details(mut self, value: serde_json::Value) -> Self {
        self.0.details = value;
        self
    }
}
impl std::ops::Deref for Error {
    type Target = MediaError;
    fn deref(&self) -> &MediaError {
        &self.0
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
impl From<MediaError> for Error {
    fn from(e: MediaError) -> Self {
        Self(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new("io_error", "storage", e)
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::new("database_error", "database", e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new("invalid_json", "request", e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn runtime_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("FRAMELEAN_RUNTIME_DIR") {
        return p.into();
    }
    if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
            .join("FrameLean/runtime")
    } else {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join("Library/Application Support/FrameLean/runtime")
    }
}
pub fn lib_name() -> &'static str {
    if cfg!(windows) {
        "framelean_fll.dll"
    } else if cfg!(target_os = "macos") {
        "libframelean_fll.dylib"
    } else {
        "libframelean_fll.so"
    }
}
pub fn engine_entry() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or("worker".into());
    let library = args
        .next()
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?.parent().unwrap().join(lib_name()));
    match mode.as_str() {
        "worker" => service_runtime::worker(library),
        "host" => host::entry(&library, false),
        "probe" => host::entry(&library, true),
        _ => Err(Error::new(
            "invalid_arguments",
            "startup",
            "usage: framelean-engine [worker|host|probe] [FLL path]",
        )),
    }
}
