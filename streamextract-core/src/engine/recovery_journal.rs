//! Recovery journal for tracking extraction and reclamation state.
//!
//! The journal provides a durable record of what has been extracted and
//! what (if any) source-storage reclamation has been committed. It is used
//! to detect interrupted sessions and present recovery options on restart.
//!
//! # Design
//!
//! The journal is written as a JSON file alongside the archive. Each write
//! is atomic at the OS level (write to temp file, then rename). This is
//! best-effort durability — it protects against clean process exit but not
//! necessarily against power loss depending on filesystem configuration.
//!
//! # Important limitations
//!
//! - The journal does NOT provide transactional recovery for operations that
//!   have already irreversibly destroyed source data. If a crash occurs after
//!   a reclamation API call succeeds but before the journal is updated, that
//!   range may be lost without the journal reflecting it.
//! - We document this limitation clearly. The journal's purpose is to
//!   support safe re-entry for streaming extraction (which is non-destructive)
//!   and to detect cases where the source archive was modified.

use crate::errors::{ExtractionError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Current schema version. Increment when the journal format changes.
const JOURNAL_VERSION: u32 = 1;

/// The state of a single archive entry in the journal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryState {
    /// Extraction has not yet been attempted.
    Pending,
    /// Extraction is in progress.
    InProgress,
    /// Extraction completed successfully and the entry was verified.
    Completed,
    /// Extraction completed but verification failed or was unavailable.
    Unverified,
    /// Extraction failed.
    Failed { reason: String },
    /// The entry was skipped due to the overwrite policy.
    Skipped,
}

/// Records the state of a single archive entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryRecord {
    pub raw_path: String,
    pub output_path: Option<PathBuf>,
    pub state: EntryState,
    pub bytes_written: u64,
    pub source_offset: Option<u64>,
    pub source_data_end: Option<u64>,
    pub source_reclaimed: bool,
}

/// The top-level journal document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    /// Schema version for forwards-compatibility checks.
    pub version: u32,

    /// Unique identifier for this extraction session.
    pub session_id: Uuid,

    /// Path to the source archive (stored for identity verification).
    pub archive_path: PathBuf,

    /// The size of the archive at journal creation time (for identity check).
    pub archive_size_at_start: u64,

    /// The archive's last-modified time at journal creation (for identity).
    pub archive_mtime_at_start: Option<DateTime<Utc>>,

    /// Destination directory for this extraction.
    pub destination: PathBuf,

    /// The format adapter that was selected.
    pub format: String,

    /// Whether destructive reclamation was requested.
    pub reclamation_requested: bool,

    /// Whether the format adapter approved reclamation.
    pub reclamation_approved: bool,

    /// Reason reclamation was denied, if applicable.
    pub reclamation_denial_reason: Option<String>,

    /// Per-entry records.
    pub entries: Vec<EntryRecord>,

    /// Timestamp of the last journal update.
    pub updated_at: DateTime<Utc>,

    /// `true` if the session completed successfully.
    pub completed: bool,
}

impl Journal {
    /// Creates a new journal for a fresh extraction session.
    pub fn new(
        archive_path: PathBuf,
        destination: PathBuf,
        format: String,
        reclamation_requested: bool,
    ) -> Result<Self> {
        let archive_size = std::fs::metadata(&archive_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let archive_mtime = std::fs::metadata(&archive_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(DateTime::from);

        Ok(Journal {
            version: JOURNAL_VERSION,
            session_id: Uuid::new_v4(),
            archive_path,
            archive_size_at_start: archive_size,
            archive_mtime_at_start: archive_mtime,
            destination,
            format,
            reclamation_requested,
            reclamation_approved: false,
            reclamation_denial_reason: None,
            entries: Vec::new(),
            updated_at: Utc::now(),
            completed: false,
        })
    }

    /// Loads an existing journal from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            ExtractionError::JournalError {
                detail: format!("cannot read journal '{}': {e}", path.display()),
            }
        })?;
        let journal: Journal = serde_json::from_str(&content).map_err(|e| {
            ExtractionError::JournalError {
                detail: format!("cannot parse journal '{}': {e}", path.display()),
            }
        })?;
        if journal.version != JOURNAL_VERSION {
            return Err(ExtractionError::JournalError {
                detail: format!(
                    "journal version mismatch: expected {JOURNAL_VERSION}, \
                     got {}",
                    journal.version
                ),
            });
        }
        Ok(journal)
    }

    /// Persists the journal to `path` using an atomic write (temp + rename).
    pub fn save(&mut self, path: &Path) -> Result<()> {
        self.updated_at = Utc::now();
        let content = serde_json::to_string_pretty(self).map_err(|e| {
            ExtractionError::JournalError {
                detail: format!("cannot serialise journal: {e}"),
            }
        })?;

        // Write to a temp file in the same directory, then rename atomically.
        let tmp_path = path.with_extension("journal.tmp");
        std::fs::write(&tmp_path, &content).map_err(|e| {
            ExtractionError::JournalError {
                detail: format!("cannot write temp journal '{}': {e}", tmp_path.display()),
            }
        })?;
        std::fs::rename(&tmp_path, path).map_err(|e| {
            ExtractionError::JournalError {
                detail: format!(
                    "cannot rename journal '{}' -> '{}': {e}",
                    tmp_path.display(),
                    path.display()
                ),
            }
        })?;

        Ok(())
    }

    /// Verifies that the archive file at `archive_path` matches the identity
    /// recorded in the journal (size and mtime). Returns an error if the
    /// archive appears to have changed since the journal was created.
    pub fn verify_archive_identity(&self) -> Result<()> {
        let meta =
            std::fs::metadata(&self.archive_path).map_err(ExtractionError::Io)?;
        if meta.len() != self.archive_size_at_start {
            return Err(ExtractionError::JournalError {
                detail: format!(
                    "archive size changed since journal was created \
                     (expected {}, got {}); reclamation aborted",
                    self.archive_size_at_start,
                    meta.len()
                ),
            });
        }
        Ok(())
    }
}

/// Computes the default journal path for a given archive path.
pub fn default_journal_path(archive_path: &Path) -> PathBuf {
    let mut p = archive_path.to_path_buf();
    let mut name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".into());
    name.push_str(".streamextract_journal");
    p.set_file_name(name);
    p
}
