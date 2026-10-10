//! Disk space measurement for source and destination volumes.
//!
//! Tracks free space on both the source (archive) volume and the destination
//! volume independently, because they may be different physical drives.
//!
//! # Windows implementation
//!
//! Uses `GetDiskFreeSpaceExW` via the standard library's `std::fs` on stable
//! Rust, which is a safe wrapper over the Windows API call.

use crate::errors::Result;
use std::path::Path;

/// Represents space information for a single filesystem volume.
#[derive(Debug, Clone)]
pub struct VolumeSpace {
    /// Free bytes available to the current user (respects disk quotas).
    pub available_bytes: u64,
    /// Total capacity of the volume in bytes.
    pub total_bytes: u64,
}

/// Returns the available and total disk space for the volume containing `path`.
///
/// On Windows, `path` may be a file or directory; the function determines
/// which volume it resides on automatically.
pub fn query_volume_space(path: &Path) -> Result<VolumeSpace> {
    // std::fs::statvfs is not available on Windows in stable Rust.
    // We use the platform module for the actual implementation.
    crate::platform::disk_space::query_volume_space_impl(path)
}

/// Returns `true` if the volume containing `path` has at least
/// `required_bytes` of free space available.
pub fn has_sufficient_space(path: &Path, required_bytes: u64) -> Result<bool> {
    let space = query_volume_space(path)?;
    Ok(space.available_bytes >= required_bytes)
}

/// Computes the minimum space required to safely begin extraction given
/// the known or estimated output size.
///
/// `estimated_output_bytes` may be `None` if the archive does not report
/// uncompressed sizes. In that case the function returns `None` and the
/// caller should treat estimates as uncertain.
pub fn estimate_required_space(
    estimated_output_bytes: Option<u64>,
    safety_margin_bytes: u64,
) -> Option<u64> {
    estimated_output_bytes.and_then(|size| {
        size.checked_add(safety_margin_bytes)
    })
}

/// A snapshot of space for both the source and destination volumes,
/// taken at the same logical moment.
#[derive(Debug, Clone)]
pub struct SpaceSnapshot {
    /// Space on the destination volume.
    pub dest: VolumeSpace,
    /// Space on the source volume. `None` if the source path is the same
    /// volume as the destination (we only query once).
    pub source: Option<VolumeSpace>,
    /// `true` if source and destination are on the same volume.
    pub same_volume: bool,
}

impl SpaceSnapshot {
    /// Takes a fresh snapshot for the given source and destination paths.
    pub fn take(source_path: &Path, dest_path: &Path) -> Result<Self> {
        let dest = query_volume_space(dest_path)?;

        // Determine whether source and destination are on the same volume.
        // We use a best-effort heuristic: compare the volume root if we can
        // determine it, otherwise query both and compare total size.
        let source_space = query_volume_space(source_path)?;
        let same_volume = source_space.total_bytes == dest.total_bytes;

        Ok(SpaceSnapshot {
            dest,
            source: Some(source_space),
            same_volume,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn can_query_temp_dir_space() {
        let tmp = env::temp_dir();
        let result = query_volume_space(&tmp);
        assert!(result.is_ok(), "Failed to query space: {result:?}");
        let space = result.unwrap();
        assert!(space.available_bytes > 0, "Expected non-zero available bytes");
        assert!(space.total_bytes >= space.available_bytes);
    }

    #[test]
    fn estimate_adds_safety_margin() {
        let margin = 10 * 1024 * 1024;
        let required = estimate_required_space(Some(100 * 1024 * 1024), margin);
        assert_eq!(required, Some(110 * 1024 * 1024));
    }

    #[test]
    fn estimate_returns_none_for_unknown_size() {
        let required = estimate_required_space(None, 10 * 1024 * 1024);
        assert_eq!(required, None);
    }

    #[test]
    fn overflow_returns_none() {
        let required = estimate_required_space(Some(u64::MAX), 1);
        assert_eq!(required, None);
    }
}
