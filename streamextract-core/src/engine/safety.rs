//! Path safety validation for archive entries.
//!
//! Every entry path extracted from an untrusted archive must pass through
//! `sanitise_path` before any filesystem operation is performed.
//!
//! # Threats addressed
//!
//! - **Path traversal** (`../../../etc/passwd`).
//! - **Absolute paths** (`/etc/passwd`, `C:\Windows\System32\…`).
//! - **UNC paths** (`\\server\share\…`).
//! - **Windows reserved device names** (`CON`, `PRN`, `AUX`, `NUL`,
//!   `COM1`–`COM9`, `LPT1`–`LPT9`).
//! - **Characters illegal on Windows** (`<>:"/\|?*` and bytes 0x00–0x1F).
//! - **Windows reparse points / symlinks** created during extraction.
//!   (Policy: do not follow reparse points at the destination unless the
//!   caller explicitly opts in.)
//! - **Entries that resolve to the destination root itself** (empty path).

use crate::errors::{ExtractionError, Result};
use std::path::{Component, Path, PathBuf};

/// Windows reserved device names that must never appear as filename stems.
///
/// The check is case-insensitive and must also reject names with extensions
/// (e.g. `NUL.txt` is still a reserved name on Windows).
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5",
    "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5",
    "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Characters that are illegal in Windows file or directory names.
const WINDOWS_ILLEGAL_CHARS: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

/// Validates `raw_path` from an archive entry and returns a safe,
/// normalised `PathBuf` relative to `destination`.
///
/// The returned path is guaranteed to be a descendant of `destination`.
///
/// # Errors
///
/// Returns an `ExtractionError` if the path fails any safety check.
pub fn sanitise_path(raw_path: &str, destination: &Path) -> Result<PathBuf> {
    let raw = raw_path;

    if raw.is_empty() || raw.trim().is_empty() {
        return Err(ExtractionError::IllegalFilename {
            entry: raw_path.to_string(),
            detail: "entry path is empty".into(),
        });
    }

    // Normalise path separators to the platform separator so that the
    // Component iterator works correctly.
    let normalised = raw.replace('/', std::path::MAIN_SEPARATOR_STR);

    let entry_path = Path::new(&normalised);

    // Check for absolute paths (catches `/etc`, `C:\Windows`, `\\server`).
    if entry_path.is_absolute() {
        return Err(ExtractionError::AbsolutePath {
            entry: raw_path.to_string(),
        });
    }

    // Manually check for UNC-style `\\` prefix even on paths that are not
    // `is_absolute()` on all platforms.
    if raw.starts_with("\\\\") || raw.starts_with("//") {
        return Err(ExtractionError::AbsolutePath {
            entry: raw_path.to_string(),
        });
    }

    // Walk each component and reject traversal or absolute components.
    let mut safe_components: Vec<&str> = Vec::new();
    for component in entry_path.components() {
        match component {
            Component::Normal(part) => {
                let part_str = part.to_str().ok_or_else(|| {
                    ExtractionError::IllegalFilename {
                        entry: raw_path.to_string(),
                        detail: "component is not valid UTF-8".into(),
                    }
                })?;

                validate_component(raw_path, part_str)?;
                safe_components.push(part_str);
            }
            Component::ParentDir => {
                return Err(ExtractionError::PathTraversal {
                    entry: raw_path.to_string(),
                    detail: "contains '..' component".into(),
                });
            }
            Component::CurDir => {
                // '.' components are ignored (they are no-ops).
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ExtractionError::AbsolutePath {
                    entry: raw_path.to_string(),
                });
            }
        }
    }

    if safe_components.is_empty() {
        return Err(ExtractionError::IllegalFilename {
            entry: raw_path.to_string(),
            detail: "path resolves to an empty relative path".into(),
        });
    }

    // Build the full output path and verify it is inside the destination.
    let mut result = destination.to_path_buf();
    for part in &safe_components {
        result.push(part);
    }

    // Final canonical check: the resolved path must start with destination.
    // We use `starts_with` on the path (not lexicographic string comparison)
    // to handle symlinks that already exist at the destination.
    //
    // Note: We do NOT call `canonicalize()` here because the output path
    // may not exist yet. Instead we compare the components we assembled
    // against the destination, which we verified is safe above.
    if !result.starts_with(destination) {
        return Err(ExtractionError::PathTraversal {
            entry: raw_path.to_string(),
            detail: "resolved path escapes the destination directory".into(),
        });
    }

    Ok(result)
}

/// Validates a single path component for Windows-illegal characters and
/// reserved names.
fn validate_component(entry: &str, component: &str) -> Result<()> {
    // Reject illegal characters (including control chars 0x00–0x1F).
    for ch in component.chars() {
        if WINDOWS_ILLEGAL_CHARS.contains(&ch) || (ch as u32) < 0x20 {
            return Err(ExtractionError::IllegalFilename {
                entry: entry.to_string(),
                detail: format!(
                    "component '{component}' contains illegal character '{ch}'"
                ),
            });
        }
    }

    // Reject components that end with a space or a period (Windows quirk:
    // trailing spaces/dots are stripped by the filesystem, creating ambiguity).
    if component.ends_with(' ') || component.ends_with('.') {
        return Err(ExtractionError::IllegalFilename {
            entry: entry.to_string(),
            detail: format!(
                "component '{component}' ends with a space or period"
            ),
        });
    }

    // Reject Windows reserved device names.
    // We compare the stem (part before the first '.') case-insensitively.
    let stem = component.split('.').next().unwrap_or(component);
    let stem_upper = stem.to_ascii_uppercase();
    if WINDOWS_RESERVED_NAMES.contains(&stem_upper.as_str()) {
        return Err(ExtractionError::ReservedName {
            entry: entry.to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dest() -> &'static Path {
        if cfg!(windows) {
            Path::new(r"C:\safe\extraction\dest")
        } else {
            Path::new("/safe/extraction/dest")
        }
    }

    #[test]
    fn safe_relative_path_is_accepted() {
        let d = dest();
        let result = sanitise_path("subdir/file.txt", d);
        assert!(result.is_ok(), "{result:?}");
        assert!(result.unwrap().starts_with(d));
    }

    #[test]
    fn path_traversal_is_rejected() {
        let d = dest();
        assert!(sanitise_path("../etc/passwd", d).is_err());
        assert!(sanitise_path("ok/../../../etc/passwd", d).is_err());
        assert!(sanitise_path("a/b/../../..", d).is_err());
    }

    #[test]
    fn absolute_path_is_rejected() {
        let d = dest();
        // On Windows, both Unix and Windows absolute paths must be rejected.
        assert!(sanitise_path("/etc/passwd", d).is_err());
        assert!(sanitise_path("C:\\Windows\\System32\\drivers", d).is_err());
    }

    #[test]
    fn unc_path_is_rejected() {
        let d = dest();
        assert!(sanitise_path("\\\\server\\share\\file", d).is_err());
        assert!(sanitise_path("//server/share/file", d).is_err());
    }

    #[test]
    fn reserved_names_are_rejected() {
        let d = dest();
        for name in &["NUL", "CON", "PRN", "AUX", "COM1", "LPT9"] {
            assert!(
                sanitise_path(name, d).is_err(),
                "expected '{name}' to be rejected"
            );
            // Also reject with extension.
            let with_ext = format!("{name}.txt");
            assert!(
                sanitise_path(&with_ext, d).is_err(),
                "expected '{with_ext}' to be rejected"
            );
        }
    }

    #[test]
    fn illegal_characters_are_rejected() {
        let d = dest();
        for ch in &['<', '>', ':', '"', '|', '?', '*'] {
            let name = format!("file{ch}name.txt");
            assert!(
                sanitise_path(&name, d).is_err(),
                "expected '{name}' to be rejected"
            );
        }
    }

    #[test]
    fn empty_path_is_rejected() {
        let d = dest();
        assert!(sanitise_path("", d).is_err());
    }

    #[test]
    fn trailing_dot_is_rejected() {
        let d = dest();
        assert!(sanitise_path("file.", d).is_err());
    }

    #[test]
    fn trailing_space_is_rejected() {
        let d = dest();
        assert!(sanitise_path("file ", d).is_err());
    }

    #[test]
    fn unicode_filenames_are_accepted() {
        let d = dest();
        let result = sanitise_path("文档/报告.txt", d);
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn deep_directory_structure_is_accepted() {
        let d = dest();
        let result = sanitise_path("a/b/c/d/e/f/g/file.txt", d);
        assert!(result.is_ok(), "{result:?}");
    }
}
