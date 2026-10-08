use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use framelean_core::{EngineError, EngineErrorCode, ErrorKind, Result};

use crate::{ExecutionOutputRequest, OutputCollisionPolicy};

static OUTPUT_TRANSACTION_SEQUENCE: AtomicU64 = AtomicU64::new(1);
const MAX_UNIQUE_OUTPUT_ATTEMPTS: usize = 10_000;
const MAX_WORKING_PATH_ATTEMPTS: usize = 128;

/// Owns a same-directory working output until it is atomically published.
///
/// The media pipeline writes only to [Self::working_path]. A successful
/// pipeline validates and closes that file before calling [Self::commit].
/// Dropping or rolling back an uncommitted transaction removes the working
/// artifact and never touches the requested final output.
#[derive(Debug)]
pub struct OutputTransaction {
    working_path: PathBuf,
    final_path: PathBuf,
    collision_policy: OutputCollisionPolicy,
    committed: bool,
}

impl OutputTransaction {
    pub fn begin(source_path: &Path, request: &ExecutionOutputRequest) -> Result<Self> {
        request.validate()?;
        let parent = request.requested_path.parent().ok_or_else(|| {
            EngineError::invalid_argument("execution output path has no parent directory")
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            output_error(
                format!("cannot create output directory {}", parent.display()),
                error,
            )
        })?;

        let source = canonical_comparison_path(source_path)?;
        let requested = comparison_path(&request.requested_path)?;
        if source == requested {
            return Err(EngineError::with_code(
                ErrorKind::Runtime,
                EngineErrorCode::OutputContainerNotWritable,
                "execution output path must not overwrite the source",
            ));
        }
        if request.collision_policy == OutputCollisionPolicy::FailIfExists
            && request.requested_path.exists()
        {
            return Err(EngineError::with_code(
                ErrorKind::Runtime,
                EngineErrorCode::OutputContainerNotWritable,
                "execution output path already exists",
            ));
        }

        let working_path = create_working_file(&request.requested_path)?;
        Ok(Self {
            working_path,
            final_path: request.requested_path.clone(),
            collision_policy: request.collision_policy,
            committed: false,
        })
    }

    pub fn working_path(&self) -> &Path {
        &self.working_path
    }

    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    pub fn commit(mut self) -> Result<PathBuf> {
        File::open(&self.working_path)
            .and_then(|file| file.sync_all())
            .map_err(|error| output_error("cannot sync working output", error))?;

        let published = match self.collision_policy {
            OutputCollisionPolicy::FailIfExists => {
                publish_without_overwrite(&self.working_path, &self.final_path)?;
                self.final_path.clone()
            }
            OutputCollisionPolicy::GenerateUnique => {
                publish_with_unique_name(&self.working_path, &self.final_path)?
            }
        };
        self.committed = true;
        self.final_path = published.clone();
        // The final path is authoritative once the no-overwrite link succeeds.
        // A cleanup failure must not turn a published result into a failed task.
        let _ = fs::remove_file(&self.working_path);
        sync_parent_best_effort(&published);
        Ok(published)
    }

    pub fn rollback(mut self) -> Result<()> {
        self.remove_working_file()?;
        self.committed = true;
        Ok(())
    }

    fn remove_working_file(&self) -> Result<()> {
        match fs::remove_file(&self.working_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(output_error("cannot remove working output", error)),
        }
    }
}

impl Drop for OutputTransaction {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.working_path);
        }
    }
}

fn create_working_file(requested_path: &Path) -> Result<PathBuf> {
    let parent = requested_path
        .parent()
        .expect("validated output paths always have a parent");
    let stem = requested_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("output");
    let extension = requested_path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty());

    for _ in 0..MAX_WORKING_PATH_ATTEMPTS {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos());
        let sequence = OUTPUT_TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut name = format!(
            ".framelean-{stem}-{}-{timestamp}-{sequence}.partial",
            std::process::id()
        );
        if let Some(extension) = extension {
            name.push('.');
            name.push_str(extension);
        }
        let path = parent.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(output_error("cannot create working output", error)),
        }
    }

    Err(EngineError::with_code(
        ErrorKind::Runtime,
        EngineErrorCode::OutputContainerNotWritable,
        "cannot allocate a unique working output path",
    ))
}

fn publish_with_unique_name(working_path: &Path, requested_path: &Path) -> Result<PathBuf> {
    for attempt in 0..MAX_UNIQUE_OUTPUT_ATTEMPTS {
        let candidate = if attempt == 0 {
            requested_path.to_path_buf()
        } else {
            unique_output_path(requested_path, attempt)
        };
        match fs::hard_link(working_path, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(output_error("cannot publish working output", error)),
        }
    }

    Err(EngineError::with_code(
        ErrorKind::Runtime,
        EngineErrorCode::OutputContainerNotWritable,
        "cannot find an unused output path",
    ))
}

fn publish_without_overwrite(working_path: &Path, final_path: &Path) -> Result<()> {
    fs::hard_link(working_path, final_path)
        .map_err(|error| output_error("cannot publish working output without overwriting", error))
}

fn unique_output_path(requested_path: &Path, attempt: usize) -> PathBuf {
    let parent = requested_path
        .parent()
        .expect("validated output paths always have a parent");
    let stem = requested_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("output");
    let extension = requested_path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty());
    let name = match extension {
        Some(extension) => format!("{stem} ({attempt}).{extension}"),
        None => format!("{stem} ({attempt})"),
    };
    parent.join(name)
}

fn canonical_comparison_path(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).map_err(|error| {
        output_error(
            format!("cannot resolve source path {}", path.display()),
            error,
        )
    })
}

fn comparison_path(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return canonical_comparison_path(path);
    }
    let parent = path.parent().ok_or_else(|| {
        EngineError::invalid_argument("execution output path has no parent directory")
    })?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        output_error(
            format!("cannot resolve output directory {}", parent.display()),
            error,
        )
    })?;
    Ok(canonical_parent.join(
        path.file_name()
            .expect("validated output paths always have a file name"),
    ))
}

fn sync_parent_best_effort(path: &Path) {
    if let Some(parent) = path.parent()
        && let Ok(directory) = File::open(parent)
    {
        let _ = directory.sync_all();
    }
}

fn output_error(message: impl Into<String>, source: io::Error) -> EngineError {
    EngineError::with_source_code(
        ErrorKind::Runtime,
        EngineErrorCode::OutputContainerNotWritable,
        message,
        source,
    )
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn fixture_directory(name: &str) -> PathBuf {
        let sequence = OUTPUT_TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "framelean-output-transaction-{name}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn commit_publishes_without_overwriting_existing_output() {
        let directory = fixture_directory("publish");
        let source = directory.join("source.mp4");
        let requested = directory.join("output.mp4");
        fs::write(&source, b"source").unwrap();
        fs::write(&requested, b"existing").unwrap();
        let transaction = OutputTransaction::begin(
            &source,
            &ExecutionOutputRequest {
                requested_path: requested.clone(),
                collision_policy: OutputCollisionPolicy::GenerateUnique,
            },
        )
        .unwrap();
        File::options()
            .append(true)
            .open(transaction.working_path())
            .unwrap()
            .write_all(b"encoded")
            .unwrap();

        let published = transaction.commit().unwrap();

        assert_eq!(fs::read(&requested).unwrap(), b"existing");
        assert_eq!(published, directory.join("output (1).mp4"));
        assert_eq!(fs::read(&published).unwrap(), b"encoded");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rollback_and_drop_remove_partial_outputs() {
        let directory = fixture_directory("rollback");
        let source = directory.join("source.mp4");
        let requested = directory.join("output.mp4");
        fs::write(&source, b"source").unwrap();
        let transaction = OutputTransaction::begin(
            &source,
            &ExecutionOutputRequest {
                requested_path: requested.clone(),
                collision_policy: OutputCollisionPolicy::FailIfExists,
            },
        )
        .unwrap();
        let first_working = transaction.working_path().to_path_buf();
        transaction.rollback().unwrap();
        assert!(!first_working.exists());

        let transaction = OutputTransaction::begin(
            &source,
            &ExecutionOutputRequest {
                requested_path: requested,
                collision_policy: OutputCollisionPolicy::FailIfExists,
            },
        )
        .unwrap();
        let second_working = transaction.working_path().to_path_buf();
        drop(transaction);
        assert!(!second_working.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn source_path_and_existing_fail_if_exists_target_are_rejected() {
        let directory = fixture_directory("reject");
        let source = directory.join("source.mp4");
        let existing = directory.join("existing.mp4");
        fs::write(&source, b"source").unwrap();
        fs::write(&existing, b"existing").unwrap();

        let source_error = OutputTransaction::begin(
            &source,
            &ExecutionOutputRequest {
                requested_path: source.clone(),
                collision_policy: OutputCollisionPolicy::GenerateUnique,
            },
        )
        .unwrap_err();
        let existing_error = OutputTransaction::begin(
            &source,
            &ExecutionOutputRequest {
                requested_path: existing,
                collision_policy: OutputCollisionPolicy::FailIfExists,
            },
        )
        .unwrap_err();

        assert_eq!(
            source_error.code(),
            EngineErrorCode::OutputContainerNotWritable
        );
        assert_eq!(
            existing_error.code(),
            EngineErrorCode::OutputContainerNotWritable
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn late_output_collisions_never_overwrite_the_competing_file() {
        let directory = fixture_directory("late-collision");
        let source = directory.join("source.mp4");
        let requested = directory.join("output.mp4");
        fs::write(&source, b"source").unwrap();

        let unique_transaction = OutputTransaction::begin(
            &source,
            &ExecutionOutputRequest {
                requested_path: requested.clone(),
                collision_policy: OutputCollisionPolicy::GenerateUnique,
            },
        )
        .unwrap();
        fs::write(unique_transaction.working_path(), b"encoded").unwrap();
        fs::write(&requested, b"competitor").unwrap();

        let published = unique_transaction.commit().unwrap();
        assert_eq!(fs::read(&requested).unwrap(), b"competitor");
        assert_eq!(published, directory.join("output (1).mp4"));
        assert_eq!(fs::read(&published).unwrap(), b"encoded");

        fs::remove_file(&requested).unwrap();
        fs::remove_file(&published).unwrap();
        let fail_transaction = OutputTransaction::begin(
            &source,
            &ExecutionOutputRequest {
                requested_path: requested.clone(),
                collision_policy: OutputCollisionPolicy::FailIfExists,
            },
        )
        .unwrap();
        fs::write(fail_transaction.working_path(), b"encoded").unwrap();
        fs::write(&requested, b"competitor").unwrap();

        let error = fail_transaction.commit().unwrap_err();
        assert_eq!(error.code(), EngineErrorCode::OutputContainerNotWritable);
        assert_eq!(fs::read(&requested).unwrap(), b"competitor");
        fs::remove_dir_all(directory).unwrap();
    }
}
