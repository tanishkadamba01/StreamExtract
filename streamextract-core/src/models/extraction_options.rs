//! Options that control how an extraction session is configured.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What to do when an output file already exists at the destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OverwritePolicy {
    /// Skip the entry; leave the existing file untouched.
    #[default]
    Skip,
    /// Overwrite the existing file.
    Overwrite,
    /// Rename the new file by appending a numeric suffix (e.g., `foo (2).txt`).
    Rename,
    /// Abort the entire extraction.
    Abort,
}

/// Controls which operations a session is allowed to perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permissions {
    /// Allow creating symlinks during extraction.
    /// Disabled by default because symlink extraction carries security risks.
    pub allow_symlinks: bool,

    /// Allow following reparse points found at the destination.
    pub allow_reparse_follow: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Permissions {
            allow_symlinks: false,
            allow_reparse_follow: false,
        }
    }
}

/// Options for progressive source-storage reclamation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReclamationOptions {
    /// If `true`, attempt to reclaim source storage as entries are extracted.
    /// This permanently modifies the source archive and cannot be undone.
    ///
    /// This flag has no effect unless the format adapter reports that
    /// reclamation is safe for this specific archive.
    pub enabled: bool,

    /// Path where the recovery journal should be written.
    /// Defaults to `<archive_path>.streamextract_journal`.
    pub journal_path: Option<PathBuf>,
}

impl Default for ReclamationOptions {
    fn default() -> Self {
        ReclamationOptions {
            enabled: false,
            journal_path: None,
        }
    }
}

/// Configures how disk-space warnings and low-space pauses work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceOptions {
    /// Stop scheduling new work when destination free space falls below
    /// this threshold in bytes. Defaults to 50 MiB.
    pub low_space_threshold_bytes: u64,

    /// Safety margin added to size estimates to account for filesystem
    /// overhead, journal writes, and metadata. Defaults to 10 MiB.
    pub safety_margin_bytes: u64,
}

impl Default for SpaceOptions {
    fn default() -> Self {
        SpaceOptions {
            low_space_threshold_bytes: 50 * 1024 * 1024,   // 50 MiB
            safety_margin_bytes: 10 * 1024 * 1024,          // 10 MiB
        }
    }
}

/// The full set of options passed to an extraction session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionOptions {
    /// Path to the source archive file.
    pub archive_path: PathBuf,

    /// Directory where extracted files will be written.
    pub destination: PathBuf,

    /// Buffer size used for streaming decompression, in bytes.
    ///
    /// This controls the chunk size passed between the decompressor and the
    /// output writer. It is NOT a limit on total process memory (the decoder
    /// may hold additional internal state).
    ///
    /// Valid range: 64 KiB – 32 MiB. Defaults to 1 MiB.
    pub buffer_size: usize,

    /// What to do when an output file already exists.
    pub overwrite_policy: OverwritePolicy,

    /// Permissions for risky operations.
    pub permissions: Permissions,

    /// Options for progressive source-storage reclamation.
    pub reclamation: ReclamationOptions,

    /// Options for space monitoring and low-space pauses.
    pub space: SpaceOptions,

    /// Maximum allowed expansion ratio (uncompressed / compressed).
    ///
    /// If any entry exceeds this ratio, extraction of that entry is aborted
    /// to prevent archive-bomb attacks. `None` disables the check.
    ///
    /// Note: some legitimate archives (highly repetitive data) can have very
    /// high ratios. Tune this value with care.
    pub max_expansion_ratio: Option<f64>,

    /// Maximum total output size in bytes.
    /// `None` disables the check.
    pub max_output_bytes: Option<u64>,

    /// Maximum number of entries to extract.
    /// `None` disables the check.
    pub max_entry_count: Option<u64>,

    /// Whether to preserve timestamps from the archive.
    pub preserve_timestamps: bool,

    /// Whether to emit verbose progress events (one per chunk rather than
    /// one per entry). Useful for large single-file archives.
    pub verbose_progress: bool,
}

impl Default for ExtractionOptions {
    fn default() -> Self {
        ExtractionOptions {
            archive_path: PathBuf::new(),
            destination: PathBuf::new(),
            buffer_size: 1024 * 1024, // 1 MiB
            overwrite_policy: OverwritePolicy::default(),
            permissions: Permissions::default(),
            reclamation: ReclamationOptions::default(),
            space: SpaceOptions::default(),
            max_expansion_ratio: Some(200.0),
            max_output_bytes: None,
            max_entry_count: None,
            preserve_timestamps: true,
            verbose_progress: false,
        }
    }
}

impl ExtractionOptions {
    /// Creates a new `ExtractionOptions` with default settings for the given paths.
    pub fn new(archive_path: impl Into<PathBuf>, destination: impl Into<PathBuf>) -> Self {
        ExtractionOptions {
            archive_path: archive_path.into(),
            destination: destination.into(),
            ..Default::default()
        }
    }

    /// Validates the options, returning an error message if anything is out
    /// of range or logically inconsistent.
    pub fn validate(&self) -> Result<(), String> {
        const MIN_BUFFER: usize = 64 * 1024;      // 64 KiB
        const MAX_BUFFER: usize = 32 * 1024 * 1024; // 32 MiB

        if self.buffer_size < MIN_BUFFER || self.buffer_size > MAX_BUFFER {
            return Err(format!(
                "buffer_size must be between {} and {} bytes, got {}",
                MIN_BUFFER, MAX_BUFFER, self.buffer_size
            ));
        }

        if let Some(ratio) = self.max_expansion_ratio {
            if ratio < 1.0 {
                return Err(format!(
                    "max_expansion_ratio must be ≥ 1.0, got {ratio}"
                ));
            }
        }

        if self.archive_path.as_os_str().is_empty() {
            return Err("archive_path must not be empty".into());
        }

        if self.destination.as_os_str().is_empty() {
            return Err("destination must not be empty".into());
        }

        Ok(())
    }
}
