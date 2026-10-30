use crate::{Error, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(unix)]
pub fn identity(file: &File) -> Result<Value> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    Ok(json!({"volume":m.dev(),"file":m.ino()}))
}
#[cfg(windows)]
pub fn identity(file: &File) -> Result<Value> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(
        json!({"volume":info.dwVolumeSerialNumber,"high":info.nFileIndexHigh,"low":info.nFileIndexLow}),
    )
}
pub fn hash(path: &Path, cancel: &AtomicBool) -> Result<(u64, String, Value)> {
    let mut f = File::open(path)?;
    let id = identity(&f)?;
    let before = f.metadata()?;
    let mut h = Sha256::new();
    let mut b = [0u8; 65536];
    let mut size = 0;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::new("canceled", "validating", "文件核验已取消"));
        }
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
        size += n as u64;
    }
    let after = f.metadata()?;
    if size != before.len()
        || before.len() != after.len()
        || before.modified()? != after.modified()?
        || identity(&File::open(path)?)? != id
    {
        return Err(Error::new(
            "output_changed",
            "validating",
            "文件核验期间发生变化",
        ));
    }
    Ok((
        size,
        h.finalize().iter().map(|b| format!("{b:02x}")).collect(),
        id,
    ))
}
pub fn cleanup(id: &str, path: &Path, expected: &Value) -> Result<()> {
    if path.file_name().and_then(|s| s.to_str()) != Some(&format!(".framelean-{id}.partial.mp4")) {
        return Err(Error::new(
            "cleanup_identity",
            "cleanup",
            "临时文件归属无法确认，已保留",
        ));
    }
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if expected.is_null() || &identity(&file)? != expected {
        return Err(Error::new(
            "cleanup_identity",
            "cleanup",
            "临时文件身份变化，已保留",
        ));
    }
    drop(file);
    fs::remove_file(path)?;
    Ok(())
}
pub fn directory(path: &Path) -> Result<String> {
    let path = fs::canonicalize(path)?;
    if !path.is_dir() {
        return Err(Error::new(
            "output_directory",
            "preflight",
            "输出位置不是目录",
        ));
    }
    let a = path.join(format!(".framelean-check-{}", crate::uid()));
    let b = a.with_extension("link");
    let test = (|| -> Result<()> {
        let file = OpenOptions::new().write(true).create_new(true).open(&a)?;
        file.sync_all()?;
        fs::hard_link(&a, &b)?;
        Ok(())
    })();
    let _ = fs::remove_file(&b);
    let _ = fs::remove_file(&a);
    test.map_err(|e| Error::new("publication_unavailable", "preflight", e))?;
    if fs2::available_space(&path)? < 16 * 1024 * 1024 {
        return Err(Error::new(
            "disk_space",
            "preflight",
            "输出位置可用空间不足 16 MiB",
        ));
    }
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::new("path_encoding", "preflight", "路径需要 Unicode"))
}
pub fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_hash_and_cleanup_only_owned_file() {
        let dir = std::env::temp_dir().join(crate::uid());
        fs::create_dir(&dir).unwrap();
        let path = dir.join(".framelean-test.partial.mp4");
        fs::write(&path, b"original").unwrap();
        let id = identity(&File::open(&path).unwrap()).unwrap();
        assert_eq!(
            hash(&path, &AtomicBool::new(true)).unwrap_err().code,
            "canceled"
        );
        assert!(cleanup("test", &path, &json!({"volume":0,"file":0})).is_err());
        assert!(path.exists());
        cleanup("test", &path, &id).unwrap();
        cleanup("test", &path, &Value::Null).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
