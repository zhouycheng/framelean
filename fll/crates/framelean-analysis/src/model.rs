use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use framelean_core::{BitRateBps, EngineErrorCode, FileSizeBytes, Observed, Result};
use framelean_media::Rational;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SourceId(String);

impl SourceId {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(framelean_core::EngineError::invalid_identifier("source"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

const QUICK_HASH_CHUNK_BYTES: u64 = 1024 * 1024;
const FULL_HASH_LIMIT_BYTES: u64 = QUICK_HASH_CHUNK_BYTES * 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "platform", rename_all = "snake_case")]
pub enum PlatformFileId {
    Unix { device: u64, inode: u64 },
    Windows { volume_serial: u64, file_index: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceFingerprint {
    canonical_path: PathBuf,
    canonicalization_status: framelean_core::ObservationStatus,
    canonicalization_reason: Option<String>,
    size_bytes: u64,
    modified_time_unix_nanos: Option<u128>,
    platform_file_id: Option<PlatformFileId>,
    quick_content_hash: [u8; 32],
}

impl SourceFingerprint {
    pub fn from_local_file(path: &Path) -> Result<Self> {
        let metadata = fs::metadata(path).map_err(|error| {
            let code = match error.kind() {
                std::io::ErrorKind::NotFound => framelean_core::EngineErrorCode::MediaFileNotFound,
                std::io::ErrorKind::PermissionDenied => {
                    framelean_core::EngineErrorCode::MediaPermissionDenied
                }
                _ => framelean_core::EngineErrorCode::MediaInfoReadFailed,
            };
            framelean_core::EngineError::with_source_code(
                framelean_core::ErrorKind::Analysis,
                code,
                "cannot read media source metadata",
                error,
            )
        })?;
        if !metadata.is_file() {
            return Err(framelean_core::EngineError::with_code(
                framelean_core::ErrorKind::Analysis,
                framelean_core::EngineErrorCode::MediaInvalidFormat,
                "media source is not a regular file",
            ));
        }
        let (canonical_path, canonicalization_status, canonicalization_reason) =
            match fs::canonicalize(path) {
                Ok(value) => (value, framelean_core::ObservationStatus::Detected, None),
                Err(error) => (
                    path.to_path_buf(),
                    framelean_core::ObservationStatus::Failed,
                    Some(error.to_string()),
                ),
            };
        let modified_time_unix_nanos = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos());
        let quick_content_hash = quick_content_hash(path, metadata.len())?;

        Ok(Self {
            canonical_path,
            canonicalization_status,
            canonicalization_reason,
            size_bytes: metadata.len(),
            modified_time_unix_nanos,
            platform_file_id: platform_file_id(&metadata),
            quick_content_hash,
        })
    }

    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub fn canonicalization_status(&self) -> framelean_core::ObservationStatus {
        self.canonicalization_status
    }

    pub fn canonicalization_reason(&self) -> Option<&str> {
        self.canonicalization_reason.as_deref()
    }

    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub fn modified_time_unix_nanos(&self) -> Option<u128> {
        self.modified_time_unix_nanos
    }

    pub fn platform_file_id(&self) -> Option<&PlatformFileId> {
        self.platform_file_id.as_ref()
    }

    pub fn quick_content_hash(&self) -> &[u8; 32] {
        &self.quick_content_hash
    }

    pub fn source_id(&self) -> Result<SourceId> {
        let mut hash = blake3::Hasher::new();
        hash.update(b"framelean-source-fingerprint-v1");
        update_path_hash(&mut hash, &self.canonical_path);
        hash.update(&self.size_bytes.to_le_bytes());
        hash.update(
            &self
                .modified_time_unix_nanos
                .unwrap_or_default()
                .to_le_bytes(),
        );
        match &self.platform_file_id {
            Some(PlatformFileId::Unix { device, inode }) => {
                hash.update(b"unix");
                hash.update(&device.to_le_bytes());
                hash.update(&inode.to_le_bytes());
            }
            Some(PlatformFileId::Windows {
                volume_serial,
                file_index,
            }) => {
                hash.update(b"windows");
                hash.update(&volume_serial.to_le_bytes());
                hash.update(&file_index.to_le_bytes());
            }
            None => {
                hash.update(b"none");
            }
        }
        hash.update(&self.quick_content_hash);
        SourceId::new(format!("source-{}", hash.finalize().to_hex()))
    }
}

fn quick_content_hash(path: &Path, size: u64) -> Result<[u8; 32]> {
    let mut file = File::open(path).map_err(|error| {
        framelean_core::EngineError::with_source_code(
            framelean_core::ErrorKind::Analysis,
            framelean_core::EngineErrorCode::MediaInfoReadFailed,
            "cannot open media source for fingerprinting",
            error,
        )
    })?;
    let mut hash = blake3::Hasher::new();
    hash.update(&size.to_le_bytes());
    if size <= FULL_HASH_LIMIT_BYTES {
        let mut limited = file.take(FULL_HASH_LIMIT_BYTES);
        let mut bytes = Vec::with_capacity(size as usize);
        limited
            .read_to_end(&mut bytes)
            .map_err(fingerprint_io_error)?;
        hash.update(&bytes);
    } else {
        let mut head = vec![0; QUICK_HASH_CHUNK_BYTES as usize];
        file.read_exact(&mut head).map_err(fingerprint_io_error)?;
        hash.update(&head);
        file.seek(SeekFrom::End(-(QUICK_HASH_CHUNK_BYTES as i64)))
            .map_err(fingerprint_io_error)?;
        let mut tail = vec![0; QUICK_HASH_CHUNK_BYTES as usize];
        file.read_exact(&mut tail).map_err(fingerprint_io_error)?;
        hash.update(&tail);
    }
    Ok(*hash.finalize().as_bytes())
}

fn fingerprint_io_error(error: std::io::Error) -> framelean_core::EngineError {
    framelean_core::EngineError::with_source_code(
        framelean_core::ErrorKind::Analysis,
        framelean_core::EngineErrorCode::MediaInfoReadFailed,
        "failed to read media source fingerprint",
        error,
    )
}

#[cfg(unix)]
fn platform_file_id(metadata: &fs::Metadata) -> Option<PlatformFileId> {
    use std::os::unix::fs::MetadataExt;
    Some(PlatformFileId::Unix {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn platform_file_id(metadata: &fs::Metadata) -> Option<PlatformFileId> {
    use std::os::windows::fs::MetadataExt;
    Some(PlatformFileId::Windows {
        volume_serial: metadata.volume_serial_number().unwrap_or_default() as u64,
        file_index: metadata.file_index().unwrap_or_default(),
    })
}

#[cfg(not(any(unix, windows)))]
fn platform_file_id(_metadata: &fs::Metadata) -> Option<PlatformFileId> {
    None
}

#[cfg(unix)]
fn update_path_hash(hash: &mut blake3::Hasher, path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

#[cfg(windows)]
fn update_path_hash(hash: &mut blake3::Hasher, path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    let units: Vec<_> = path.as_os_str().encode_wide().collect();
    hash.update(&(units.len() as u64).to_le_bytes());
    for unit in units {
        hash.update(&unit.to_le_bytes());
    }
}

#[cfg(not(any(unix, windows)))]
fn update_path_hash(hash: &mut blake3::Hasher, path: &Path) {
    let value = path.to_string_lossy();
    hash.update(&(value.len() as u64).to_le_bytes());
    hash.update(value.as_bytes());
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MediaSource {
    LocalFile(PathBuf),
}

impl MediaSource {
    pub fn local_file(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(framelean_core::EngineError::invalid_argument(
                "local media path cannot be empty",
            ));
        }
        Ok(Self::LocalFile(path))
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::LocalFile(path) => path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MediaAnalyzeRequest {
    pub source: MediaSource,
    pub request_id: Option<String>,
    #[serde(default)]
    pub expected_source: Option<ExpectedSourceFacts>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExpectedSourceFacts {
    pub file_size_bytes: u64,
    pub modified_time_unix_nanos: Option<String>,
}

impl ExpectedSourceFacts {
    pub fn validate(&self, fingerprint: &SourceFingerprint) -> Result<()> {
        if fingerprint.size_bytes() != self.file_size_bytes {
            return Err(framelean_core::EngineError::with_code(
                framelean_core::ErrorKind::Analysis,
                framelean_core::EngineErrorCode::AnalysisSourceChanged,
                "media source size no longer matches the submitted source facts",
            ));
        }
        if let Some(expected) = &self.modified_time_unix_nanos {
            let expected = expected.parse::<u128>().map_err(|_| {
                framelean_core::EngineError::invalid_argument(
                    "expected source modified time must be an unsigned integer",
                )
            })?;
            if fingerprint.modified_time_unix_nanos() != Some(expected) {
                return Err(framelean_core::EngineError::with_code(
                    framelean_core::ErrorKind::Analysis,
                    framelean_core::EngineErrorCode::AnalysisSourceChanged,
                    "media source modified time no longer matches the submitted source facts",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaAnalysisStatus {
    Complete,
    Partial,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
    AnimatedImage,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MediaWarning {
    pub code: EngineErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyzedMedia {
    media: MediaAnalysis,
    source_fingerprint: SourceFingerprint,
}

impl AnalyzedMedia {
    pub fn new(media: MediaAnalysis, source_fingerprint: SourceFingerprint) -> Result<Self> {
        if media.source_id != source_fingerprint.source_id()? {
            return Err(framelean_core::EngineError::with_code(
                framelean_core::ErrorKind::Analysis,
                EngineErrorCode::AnalysisSourceChanged,
                "media source id does not match analysis fingerprint",
            ));
        }
        Ok(Self {
            media,
            source_fingerprint,
        })
    }

    pub fn into_parts(self) -> (MediaAnalysis, SourceFingerprint) {
        (self.media, self.source_fingerprint)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MediaAnalysis {
    pub status: MediaAnalysisStatus,
    pub source_id: SourceId,
    pub file_name: String,
    pub display_path: Option<String>,
    pub file_size: FileSizeBytes,
    pub kind: MediaKind,
    pub format: Observed<String>,
    pub duration: Observed<framelean_media::MediaDuration>,
    pub descriptor: MediaDescriptor,
    pub provider: String,
    pub provider_version: Option<String>,
    pub warnings: Vec<MediaWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MediaDescriptor {
    Video {
        streams: Vec<MediaStreamDescriptor>,
    },
    Audio {
        streams: Vec<MediaStreamDescriptor>,
    },
    Image {
        image: Box<ImageInfo>,
    },
    AnimatedImage {
        image: Box<ImageInfo>,
        animation: Box<AnimationInfo>,
    },
    Other {
        streams: Vec<MediaStreamDescriptor>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "info", rename_all = "snake_case")]
pub enum MediaStreamDescriptor {
    Video(Box<VideoStreamInfo>),
    Audio(Box<AudioStreamInfo>),
    Subtitle(SubtitleStreamInfo),
    Data(DataStreamInfo),
    Attachment(DataStreamInfo),
    Unknown(UnknownStreamInfo),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HdrInfo {
    pub color_range: Observed<String>,
    pub color_space: Observed<String>,
    pub color_transfer: Observed<String>,
    pub color_primaries: Observed<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VideoStreamInfo {
    pub stream_index: u32,
    pub codec: String,
    pub profile: Observed<String>,
    pub width: u32,
    pub height: u32,
    pub frame_rate: Observed<Rational>,
    pub frame_count: Observed<u64>,
    pub time_base: Rational,
    pub bit_depth: Observed<u8>,
    pub pixel_format: Observed<String>,
    pub hdr: HdrInfo,
    pub bitrate: Observed<BitRateBps>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AudioStreamInfo {
    pub stream_index: u32,
    pub codec: String,
    pub profile: Observed<String>,
    pub sample_rate_hz: Observed<u32>,
    pub channel_count: Observed<u32>,
    pub channel_layout: Observed<String>,
    pub sample_format: Observed<String>,
    pub bitrate: Observed<BitRateBps>,
    pub duration: Observed<framelean_media::MediaDuration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SubtitleStreamInfo {
    pub stream_index: u32,
    pub codec: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DataStreamInfo {
    pub stream_index: u32,
    pub codec: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnknownStreamInfo {
    pub stream_index: u32,
    pub codec: String,
    pub media_type_code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImageInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub pixel_format: Observed<String>,
    pub bit_depth: Observed<u8>,
    pub alpha: Observed<bool>,
    pub color_space: Observed<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AnimationInfo {
    pub frame_rate: Observed<Rational>,
    pub frame_count: Observed<u64>,
    pub duration: Observed<framelean_media::MediaDuration>,
}

pub trait MediaAnalyzer: Send + Sync {
    fn analyze(&self, request: &MediaAnalyzeRequest) -> Result<AnalyzedMedia>;
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn local_source_preserves_os_path() {
        let source = MediaSource::local_file(PathBuf::from("媒体 文件.mp4")).unwrap();
        assert_eq!(source.path(), Path::new("媒体 文件.mp4"));
    }

    #[test]
    fn source_fingerprint_changes_when_same_size_content_changes() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("framelean-fingerprint-{suffix}.bin"));
        fs::write(&path, b"aaaa").unwrap();
        let first = SourceFingerprint::from_local_file(&path).unwrap();

        fs::write(&path, b"bbbb").unwrap();
        let second = SourceFingerprint::from_local_file(&path).unwrap();
        fs::remove_file(path).unwrap();

        assert_eq!(first.size_bytes(), second.size_bytes());
        assert_ne!(first.quick_content_hash(), second.quick_content_hash());
        assert_ne!(first.source_id().unwrap(), second.source_id().unwrap());
    }

    #[test]
    fn source_fingerprint_changes_when_file_size_changes() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("framelean-size-fingerprint-{suffix}.bin"));
        fs::write(&path, b"a").unwrap();
        let first = SourceFingerprint::from_local_file(&path).unwrap();
        fs::write(&path, b"larger").unwrap();
        let second = SourceFingerprint::from_local_file(&path).unwrap();
        fs::remove_file(path).unwrap();

        assert_ne!(first.size_bytes(), second.size_bytes());
        assert_ne!(first.source_id().unwrap(), second.source_id().unwrap());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn source_fingerprint_preserves_non_utf8_unix_paths() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let mut name = b"framelean-non-utf8-".to_vec();
        name.push(0xff);
        let path = std::env::temp_dir().join(OsString::from_vec(name));
        fs::write(&path, b"fixture").unwrap();
        let fingerprint = SourceFingerprint::from_local_file(&path).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(fingerprint.size_bytes(), 7);
        assert!(!fingerprint.source_id().unwrap().as_str().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn local_source_preserves_non_utf8_path_bytes() {
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let source = MediaSource::local_file(OsString::from_vec(vec![b'a', 0xff])).unwrap();
        assert_eq!(source.path().as_os_str().as_bytes(), &[b'a', 0xff]);
    }
}
