use fll_contracts::{MediaError, Result, SourceStamp};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata},
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::UNIX_EPOCH,
};

pub(crate) fn stamp(path: &Path) -> Result<SourceStamp> {
    let read = || -> std::io::Result<SourceStamp> {
        let mut file = File::open(path)?;
        let before = file.metadata()?;
        if !before.is_file() {
            return Err(std::io::Error::other("输入不是普通文件"));
        }
        let identity = identity(&file, &before)?;
        let modified = before
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(std::io::Error::other)?
            .as_nanos()
            .to_string();
        let mut hash = Sha256::new();
        let mut bytes = vec![0; before.len().min(2 * 1024 * 1024) as usize];
        if before.len() <= 2 * 1024 * 1024 {
            file.read_exact(&mut bytes)?;
        } else {
            file.read_exact(&mut bytes[..1024 * 1024])?;
            file.seek(SeekFrom::End(-(1024 * 1024)))?;
            file.read_exact(&mut bytes[1024 * 1024..])?;
        }
        hash.update(bytes);
        let after = file.metadata()?;
        if before.len() != after.len() || before.modified()? != after.modified()? {
            return Err(std::io::Error::other("读取期间源文件发生变化"));
        }
        Ok(SourceStamp {
            version: 1,
            identity,
            size: before.len(),
            modified_ns: modified,
            sample_sha256: hash.finalize().iter().map(|b| format!("{b:02x}")).collect(),
        })
    };
    read().map_err(|e| MediaError::new("source_unreadable", "source", e))
}

#[cfg(unix)]
fn identity(_: &File, m: &Metadata) -> std::io::Result<String> {
    use std::os::unix::fs::MetadataExt;
    Ok(format!("unix:{}:{}", m.dev(), m.ino()))
}
#[cfg(windows)]
fn identity(file: &File, _: &Metadata) -> std::io::Result<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(format!(
        "windows:{}:{}:{}",
        info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
    ))
}
