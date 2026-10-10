//! Windows-specific filesystem operations.
//!
//! This module is the only place that calls Windows APIs for sparse file
//! operations and range deallocation. All unsafe code is isolated here with
//! explicit safety documentation.
//!
//! # Current status
//!
//! Phase 1 does not implement destructive storage reclamation. This module
//! currently contains only the API stubs and safety analysis needed for Phase 6
//! (read-only ZIP range analysis) and Phase 7 (proof-of-concept reclamation).
//!
//! # API choices for storage reclamation (future phases)
//!
//! | API | Purpose | Availability |
//! |-----|---------|-------------|
//! | `FSCTL_SET_SPARSE` | Marks a file as sparse | NTFS only |
//! | `FSCTL_SET_ZERO_DATA` | Deallocates a range | NTFS sparse files only |
//! | `FSCTL_QUERY_ALLOCATED_RANGES` | Lists allocated ranges | NTFS only |
//! | `GetDiskFreeSpaceExW` | Volume free space | All Windows FS |
//!
//! # Safety contract
//!
//! Any function that calls a Windows API must:
//! 1. Check the return value and translate errors to `ExtractionError`.
//! 2. Document what happens if the call partially succeeds.
//! 3. Never assume that a zeroing call has physically freed disk space;
//!    always query `GetDiskFreeSpaceExW` before and after to verify.

use crate::errors::{ExtractionError, Result};

/// Returns `true` if the filesystem at `path` is NTFS.
///
/// This is a prerequisite for sparse file operations. Non-NTFS filesystems
/// (FAT32, exFAT, ReFS) do not support `FSCTL_SET_SPARSE`.
#[cfg(windows)]
pub fn is_ntfs(path: &std::path::Path) -> Result<bool> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

    // We need the volume root (e.g. "C:\") to call GetVolumeInformationW.
    // Extract it from the path's prefix.
    let root = get_volume_root(path)?;
    let root_wide: Vec<u16> = root
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0u16))
        .collect();

    let mut fs_name_buf = vec![0u16; 256];
    let mut flags: u32 = 0;

    unsafe {
        GetVolumeInformationW(
            windows::core::PCWSTR(root_wide.as_ptr()),
            None,
            None,
            None,
            Some(&mut flags),
            Some(&mut fs_name_buf),
        )
        .map_err(|e| ExtractionError::WindowsApiError {
            code: e.code().0 as u32,
            detail: format!("GetVolumeInformationW failed for '{}'", root.display()),
        })?;
    }

    // Trim null terminator and compare.
    let fs_name = String::from_utf16_lossy(
        &fs_name_buf[..fs_name_buf.iter().position(|&c| c == 0).unwrap_or(fs_name_buf.len())],
    );

    Ok(fs_name.eq_ignore_ascii_case("NTFS"))
}

/// Extract the volume root from a path (e.g., `C:\` from `C:\foo\bar`).
#[cfg(windows)]
fn get_volume_root(path: &std::path::Path) -> Result<std::path::PathBuf> {
    use std::path::Component;
    // Walk components to find the prefix + root.
    let mut root = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(p) => root.push(p.as_os_str()),
            Component::RootDir => {
                root.push(std::path::MAIN_SEPARATOR_STR);
                break;
            }
            _ => break,
        }
    }
    if root.as_os_str().is_empty() {
        // Relative path – use current directory.
        root = std::env::current_dir().map_err(ExtractionError::Io)?;
        // Recurse to get the volume root of cwd.
        return get_volume_root(&root);
    }
    Ok(root)
}

/// Stub: Mark a file as sparse (NTFS only).
///
/// This function is not called in Phase 1. It will be implemented in Phase 7
/// after the read-only range analysis (Phase 6) produces a verified safety
/// proof for the target archive layout.
///
/// # Safety requirements (future implementation)
///
/// - The file handle must be opened with `GENERIC_WRITE` and
///   `FILE_FLAG_BACKUP_SEMANTICS` if needed.
/// - The file must reside on an NTFS volume.
/// - `DeviceIoControl(FSCTL_SET_SPARSE)` must succeed before
///   `FSCTL_SET_ZERO_DATA` is called.
#[cfg(windows)]
pub fn mark_file_sparse(_path: &std::path::Path) -> Result<()> {
    Err(ExtractionError::ReclamationNotSupported {
        reason: "sparse file marking not yet implemented (Phase 7)".into(),
    })
}

/// Stub: Deallocate a byte range within a sparse file (NTFS only).
///
/// This function is not called in Phase 1. See `mark_file_sparse` for the
/// full list of requirements.
///
/// # Safety requirements (future implementation)
///
/// - `mark_file_sparse` must have been called first.
/// - The range must have been verified as safe to discard by the format
///   adapter and the recovery journal.
/// - The operation must be journaled before this call is made.
/// - The return value must be checked; partial success must be handled.
#[cfg(windows)]
pub fn deallocate_range(
    _path: &std::path::Path,
    _offset: u64,
    _length: u64,
) -> Result<()> {
    Err(ExtractionError::ReclamationNotSupported {
        reason: "range deallocation not yet implemented (Phase 7)".into(),
    })
}

// On non-Windows, provide no-op stubs so the crate compiles for CI.
#[cfg(not(windows))]
pub fn is_ntfs(_path: &std::path::Path) -> Result<bool> {
    Ok(false) // Not applicable on non-Windows.
}

#[cfg(not(windows))]
pub fn mark_file_sparse(_path: &std::path::Path) -> Result<()> {
    Err(ExtractionError::ReclamationNotSupported {
        reason: "sparse file operations are only supported on Windows NTFS".into(),
    })
}

#[cfg(not(windows))]
pub fn deallocate_range(
    _path: &std::path::Path,
    _offset: u64,
    _length: u64,
) -> Result<()> {
    Err(ExtractionError::ReclamationNotSupported {
        reason: "range deallocation is only supported on Windows NTFS".into(),
    })
}
