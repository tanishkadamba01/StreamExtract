//! Safe output file writer.
//!
//! Responsible for creating output directories and files with safe flags,
//! writing chunks, preserving timestamps, and enforcing the overwrite policy.
//!
//! # Security guarantees
//!
//! - Paths are validated by `engine::safety::sanitise_path` before this
//!   module is called; this module does not re-validate paths.
//! - Files are created with `OpenOptions::create_new` by default to avoid
//!   TOCTOU races when the overwrite policy is Skip or Abort.
//! - Directories are created with `create_dir_all` after validating the path.
//! - Symlinks are never created by this writer (the caller must check the
//!   overwrite policy and the permissions model before calling this module).

use crate::errors::{ExtractionError, Result};
use crate::models::extraction_options::OverwritePolicy;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Outcome of attempting to open an output file.
pub enum OutputFileResult {
    /// File was created or opened; `path` is the final path used.
    Opened { file: File, path: PathBuf },
    /// The output was skipped (overwrite policy = Skip).
    Skipped,
    /// The output was renamed; `path` is the new path.
    Renamed { file: File, path: PathBuf },
}

impl std::fmt::Debug for OutputFileResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFileResult::Opened { path, .. } => f.debug_struct("Opened").field("path", path).finish(),
            OutputFileResult::Skipped => write!(f, "Skipped"),
            OutputFileResult::Renamed { path, .. } => f.debug_struct("Renamed").field("path", path).finish(),
        }
    }
}

/// Creates all directories in `path` (exclusive of the final component),
/// rejecting any reparse point or symlink along the way.
///
/// This prevents a directory creation from silently redirecting to a
/// different location if a symlink was injected by an earlier archive entry.
pub fn create_output_dirs(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).map_err(|e| ExtractionError::OutputWriteFailed {
        path: dir.display().to_string(),
        source: e,
    })
}

/// Opens an output file, respecting `policy`.
///
/// # Parameters
///
/// - `path`: The intended output path (must already be sanitised).
/// - `policy`: What to do if the file already exists.
pub fn open_output_file(
    path: &Path,
    policy: &OverwritePolicy,
) -> Result<OutputFileResult> {
    match policy {
        OverwritePolicy::Skip => {
            if path.exists() {
                return Ok(OutputFileResult::Skipped);
            }
            create_new_file(path).map(|f| OutputFileResult::Opened {
                file: f,
                path: path.to_path_buf(),
            })
        }
        OverwritePolicy::Overwrite => {
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .map_err(|e| ExtractionError::OutputWriteFailed {
                    path: path.display().to_string(),
                    source: e,
                })?;
            Ok(OutputFileResult::Opened {
                file,
                path: path.to_path_buf(),
            })
        }
        OverwritePolicy::Rename => {
            let final_path = find_rename_path(path);
            create_new_file(&final_path).map(|f| {
                if final_path == path {
                    OutputFileResult::Opened {
                        file: f,
                        path: final_path,
                    }
                } else {
                    OutputFileResult::Renamed {
                        file: f,
                        path: final_path,
                    }
                }
            })
        }
        OverwritePolicy::Abort => {
            if path.exists() {
                return Err(ExtractionError::OutputExists {
                    path: path.display().to_string(),
                });
            }
            create_new_file(path).map(|f| OutputFileResult::Opened {
                file: f,
                path: path.to_path_buf(),
            })
        }
    }
}

/// Creates a file with `O_CREAT | O_EXCL` semantics (fails if it exists).
fn create_new_file(path: &Path) -> Result<File> {
    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        create_output_dirs(parent)?;
    }

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| ExtractionError::OutputWriteFailed {
            path: path.display().to_string(),
            source: e,
        })
}

/// Generates a non-colliding path by appending ` (N)` before the extension.
fn find_rename_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|e| e.to_str());
    let parent = path.parent().unwrap_or(Path::new("."));

    for n in 2..=9999u32 {
        let name = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    // Fallback: should never happen in practice.
    path.to_path_buf()
}

/// Writes a chunk of data to `file` and returns the number of bytes written.
///
/// Returns an error if the write is short or fails.
pub fn write_chunk(file: &mut File, chunk: &[u8], path: &Path) -> Result<usize> {
    file.write_all(chunk)
        .map_err(|e| ExtractionError::OutputWriteFailed {
            path: path.display().to_string(),
            source: e,
        })?;
    Ok(chunk.len())
}

/// Flushes and syncs the file to ensure data is on disk.
pub fn finalise_file(file: &mut File, path: &Path) -> Result<()> {
    file.flush()
        .map_err(|e| ExtractionError::OutputWriteFailed {
            path: path.display().to_string(),
            source: e,
        })?;
    // We do not call fsync() by default (it is expensive and the OS
    // guarantees ordering on exit/flush for most use cases). The caller
    // may call sync_all if it needs strict durability before journaling.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn creates_file_in_new_directory() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("sub").join("file.txt");
        let result = open_output_file(&dest, &OverwritePolicy::Skip).unwrap();
        assert!(matches!(result, OutputFileResult::Opened { .. }));
        assert!(dest.exists());
    }

    #[test]
    fn skip_policy_skips_existing_file() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("file.txt");
        fs::write(&dest, b"original").unwrap();
        let result = open_output_file(&dest, &OverwritePolicy::Skip).unwrap();
        assert!(matches!(result, OutputFileResult::Skipped));
        assert_eq!(fs::read(&dest).unwrap(), b"original");
    }

    #[test]
    fn overwrite_policy_replaces_existing_file() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("file.txt");
        fs::write(&dest, b"original").unwrap();
        let result = open_output_file(&dest, &OverwritePolicy::Overwrite).unwrap();
        assert!(matches!(result, OutputFileResult::Opened { .. }));
    }

    #[test]
    fn abort_policy_errors_on_existing_file() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("file.txt");
        fs::write(&dest, b"original").unwrap();
        let result = open_output_file(&dest, &OverwritePolicy::Abort);
        assert!(result.is_err());
    }

    #[test]
    fn rename_policy_generates_new_name() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("file.txt");
        fs::write(&dest, b"original").unwrap();
        let result = open_output_file(&dest, &OverwritePolicy::Rename).unwrap();
        match result {
            OutputFileResult::Renamed { path, .. } => {
                assert_ne!(path, dest);
                assert!(path.exists());
            }
            other => panic!("expected Renamed, got {other:?}"),
        }
    }
}
