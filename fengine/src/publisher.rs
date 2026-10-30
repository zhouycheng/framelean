use crate::{
    Error, Result,
    coordinator::{Internal, Service},
    storage,
};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub fn publish(
    service: &Service,
    id: &str,
    temp: &Path,
    input: &str,
    cancel: &AtomicBool,
) -> Result<Value> {
    let (size, hash, identity) = storage::hash(temp, cancel)?;
    OpenOptions::new().write(true).open(temp)?.sync_all()?;
    let stem = Path::new(input)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    for number in 0..10000 {
        let suffix = if number == 0 {
            String::new()
        } else {
            format!("-{number}")
        };
        let final_path = temp
            .parent()
            .unwrap()
            .join(format!("{stem}-compressed{suffix}.mp4"));
        service.internal(Internal::Publishing {
            id: id.into(),
            path: final_path.to_string_lossy().into_owned(),
            size,
            hash: hash.clone(),
            identity: identity.clone(),
        })?;
        fault("publish_intent");
        if storage::identity(&File::open(temp)?)? != identity {
            return Err(Error::new(
                "output_changed",
                "publishing",
                "临时文件身份变化，发布已停止",
            ));
        }
        match fs::hard_link(temp, &final_path) {
            Ok(()) => {
                fault("publish_link");
                let published = File::open(&final_path)?;
                if storage::identity(&published)? != identity || published.metadata()?.len() != size
                {
                    return Err(Error::new(
                        "output_changed",
                        "publishing",
                        "发布文件身份变化，需要重新核对",
                    ));
                }
                storage::sync_directory(temp.parent().unwrap())?;
                return Ok(json!({"path":final_path,"size_bytes":size,"sha256":hash}));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::new("publication_failed", "publishing", e)),
        }
    }
    Err(Error::new(
        "output_conflict",
        "publishing",
        "无法生成未被占用的输出名称",
    ))
}
pub fn reconcile(row: &Value, cancel: &AtomicBool) -> Value {
    let id = row["id"].as_str().unwrap_or("");
    let result = (|| -> Result<(&str, Option<Error>)> {
        if row["final_path"].is_null() {
            return Ok((
                if row["cancel_requested_at"].is_null() {
                    "interrupted"
                } else {
                    "canceled"
                },
                Some(Error::new(
                    "interrupted",
                    "recovery",
                    "后台中断，未发布正式结果",
                )),
            ));
        }
        let path = Path::new(row["final_path"].as_str().unwrap());
        if cancel.load(Ordering::SeqCst) {
            return Ok((
                "reconciling",
                Some(Error::new(
                    "reconcile_pending",
                    "recovery",
                    "文件核对等待下次启动",
                )),
            ));
        }
        match File::open(path) {
            Ok(file) => {
                let actual = storage::identity(&file)?;
                let expected = &row["file_identity_json"];
                if !expected.is_null() && *expected != actual {
                    return Ok((
                        "failed",
                        Some(Error::new(
                            "output_conflict",
                            "recovery",
                            "正式路径属于其他文件，已保留",
                        )),
                    ));
                }
                drop(file);
                let (size, hash, _) = storage::hash(path, cancel)?;
                if row["size_bytes"].as_u64() == Some(size) && row["sha256"].as_str() == Some(&hash)
                {
                    Ok(("succeeded", None))
                } else {
                    Ok((
                        "failed",
                        Some(Error::new(
                            "output_conflict",
                            "recovery",
                            "正式文件内容不匹配，已保留",
                        )),
                    ))
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if path.parent().is_some_and(|p| fs::read_dir(p).is_ok()) {
                    Ok((
                        "interrupted",
                        Some(Error::new(
                            "output_missing",
                            "recovery",
                            "正式文件缺失，可重新执行",
                        )),
                    ))
                } else {
                    Ok((
                        "reconciling",
                        Some(Error::new(
                            "storage_unavailable",
                            "recovery",
                            "输出目录暂不可访问，等待核对",
                        )),
                    ))
                }
            }
            Err(e) => Ok((
                "reconciling",
                Some(Error::new("storage_unavailable", "recovery", e)),
            )),
        }
    })();
    let (status, error) = match result {
        Ok(v) => v,
        Err(e) => ("reconciling", Some(e)),
    };
    json!({"id":id,"status":status,"error":error})
}
pub fn fault(point: &str) {
    // Fault injection is restricted to an explicitly isolated test runtime.
    if std::env::var_os("FRAMELEAN_TEST_MODE").as_deref() != Some(std::ffi::OsStr::new("1"))
        || std::env::var_os("FRAMELEAN_DATA_DIR").is_none()
    {
        return;
    }
    if std::env::var("FRAMELEAN_TEST_FAULT").ok().as_deref() == Some(point) {
        std::process::abort();
    }
    if std::env::var("FRAMELEAN_TEST_PAUSE").ok().as_deref() == Some(point) {
        let marker = crate::runtime_dir().join(format!("{point}.ready"));
        let _ = fs::write(&marker, b"ready");
        let start = std::time::Instant::now();
        while !marker.with_extension("go").exists() && start.elapsed().as_secs() < 15 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
