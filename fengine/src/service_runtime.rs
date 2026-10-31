use crate::{
    Error, Result,
    coordinator::{Config, Service},
    gateway,
};
use fs2::FileExt;
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
pub fn worker(library: PathBuf) -> Result<()> {
    let runtime = crate::runtime_dir();
    private_directory(&runtime)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(runtime.join("worker.lock"))?;
    if lock.try_lock_exclusive().is_err() {
        return Ok(());
    }
    let data = std::env::var_os("FRAMELEAN_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime.parent().unwrap().join("data-v2"));
    private_directory(&data)?;
    let owner = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data.join("owner.lock"))?;
    owner
        .try_lock_exclusive()
        .map_err(|_| Error::new("database_owned", "startup", "数据库已由另一 Worker 使用"))?;
    let server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|e| Error::new("listen_failed", "startup", e))?;
    let (service, coordinator) = Service::start(Config {
        engine: std::env::current_exe()?,
        library,
        runtime: runtime.clone(),
        database: data.join("framelean-v2.sqlite3"),
    });
    let endpoint = runtime.join("endpoint.json");
    let temporary = runtime.join(format!("endpoint-{}.tmp", crate::uid()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    write!(
        file,
        "{}",
        json!({"protocol_version":fll_contracts::HTTP_VERSION,"url":format!("http://{}",server.server_addr()),"token":service.token,"pid":std::process::id(),"instance_id":service.events.instance})
    )?;
    file.sync_all()?;
    drop(file);
    #[cfg(windows)]
    if endpoint.exists() {
        fs::remove_file(&endpoint)?;
    }
    fs::rename(temporary, &endpoint)?;
    let active = Arc::new(AtomicUsize::new(0));
    let streams = Arc::new(AtomicUsize::new(0));
    while !service.stopped.load(Ordering::SeqCst) {
        if let Some(req) = server.recv_timeout(Duration::from_millis(100))? {
            let sse = req.url().starts_with("/v4/events");
            if active.load(Ordering::SeqCst) >= 16 || (sse && streams.load(Ordering::SeqCst) >= 8) {
                gateway::reply(
                    req,
                    503,
                    json!({"error":Error::new("connection_limit","gateway","连接数量已达到上限")}),
                );
                continue;
            }
            active.fetch_add(1, Ordering::SeqCst);
            if sse {
                streams.fetch_add(1, Ordering::SeqCst);
            }
            let service = service.clone();
            let active = active.clone();
            let streams = streams.clone();
            thread::spawn(move || {
                gateway::handle(service, req);
                active.fetch_sub(1, Ordering::SeqCst);
                if sse {
                    streams.fetch_sub(1, Ordering::SeqCst);
                }
            });
        }
    }
    let _ = coordinator.join();
    let _ = fs::remove_file(endpoint);
    drop(owner);
    drop(lock);
    Ok(())
}
pub fn private_directory(path: &std::path::Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
