//! Disk space queries for Windows and other platforms.
//!
//! The Windows implementation uses `GetDiskFreeSpaceExW` via the `windows`
//! crate (Microsoft's official Rust bindings).
//!
//! For non-Windows platforms (used in CI/testing) we fall back to `statvfs`.

use crate::engine::space_estimator::VolumeSpace;
use crate::errors::{ExtractionError, Result};
use std::path::Path;

/// Platform-specific implementation of volume space query.
pub fn query_volume_space_impl(path: &Path) -> Result<VolumeSpace> {
    // Determine a path that actually exists so we can query its volume.
    let query_path = find_existing_ancestor(path).unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir())
    });
    query_available_space(&query_path)
}

/// Finds the nearest existing ancestor of `path` (including `path` itself).
fn find_existing_ancestor(path: &Path) -> Option<std::path::PathBuf> {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn query_available_space(path: &Path) -> Result<VolumeSpace> {
    #[cfg(windows)]
    return query_windows(path);

    #[cfg(not(windows))]
    return query_unix(path);
}

#[cfg(windows)]
fn query_windows(path: &Path) -> Result<VolumeSpace> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let try_query = |wide_ptr: *const u16| -> std::result::Result<VolumeSpace, windows::core::Error> {
        let mut free_bytes_available: u64 = 0;
        let mut total_number_of_bytes: u64 = 0;
        let mut total_number_of_free_bytes: u64 = 0;

        unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(wide_ptr),
                Some(&mut free_bytes_available),
                Some(&mut total_number_of_bytes),
                Some(&mut total_number_of_free_bytes),
            )?;
        }

        Ok(VolumeSpace {
            available_bytes: free_bytes_available,
            total_bytes: total_number_of_bytes,
        })
    };

    // 1. Try querying the exact path.
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0u16))
        .collect();

    if let Ok(space) = try_query(wide.as_ptr()) {
        return Ok(space);
    }

    // 2. If querying the directory failed (e.g. ERROR_ACCESS_DENIED because the caller
    // lacks FILE_LIST_DIRECTORY permissions on that specific folder), fall back to
    // querying the volume root (e.g. "C:\").
    if let Some(volume_root) = get_volume_root(path) {
        let root_wide: Vec<u16> = volume_root
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0u16))
            .collect();
        if let Ok(space) = try_query(root_wide.as_ptr()) {
            return Ok(space);
        }
    }

    // 3. Fall back to querying the default directory (pass NULL to GetDiskFreeSpaceExW).
    if let Ok(space) = try_query(std::ptr::null()) {
        return Ok(space);
    }

    // If all attempts failed, repeat the original call to capture the actual Windows error.
    let mut free_bytes_available: u64 = 0;
    let mut total_number_of_bytes: u64 = 0;
    let mut total_number_of_free_bytes: u64 = 0;

    unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide.as_ptr()),
            Some(&mut free_bytes_available),
            Some(&mut total_number_of_bytes),
            Some(&mut total_number_of_free_bytes),
        )
        .map_err(|e| ExtractionError::WindowsApiError {
            code: e.code().0 as u32,
            detail: format!(
                "GetDiskFreeSpaceExW failed for '{}'",
                path.display()
            ),
        })?;
    }

    Ok(VolumeSpace {
        available_bytes: free_bytes_available,
        total_bytes: total_number_of_bytes,
    })
}

#[cfg(windows)]
fn get_volume_root(path: &Path) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let mut components = path.components();
    match components.next()? {
        Component::Prefix(prefix) => {
            let mut root = std::path::PathBuf::from(prefix.as_os_str());
            root.push(std::path::MAIN_SEPARATOR.to_string());
            Some(root)
        }
        _ => None,
    }
}

#[cfg(not(windows))]
fn query_unix(path: &Path) -> Result<VolumeSpace> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        ExtractionError::Internal {
            detail: "path contains null byte".into(),
        }
    })?;

    // SAFETY: `c_path` is a valid C string; `stat` is properly initialized
    // to zero before the call.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if ret != 0 {
        return Err(ExtractionError::Io(std::io::Error::last_os_error()));
    }

    let block = stat.f_frsize as u64;
    Ok(VolumeSpace {
        available_bytes: stat.f_bavail.saturating_mul(block),
        total_bytes: stat.f_blocks.saturating_mul(block),
    })
}
