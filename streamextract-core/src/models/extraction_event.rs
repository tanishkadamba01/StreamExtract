//! Typed events emitted during an extraction session.
//!
//! Events are sent from the extraction engine to the consumer (CLI or Tauri
//! frontend) over a channel. They carry enough information to update a
//! progress display, display warnings, or drive a recovery UI.

use crate::models::archive_entry::VerificationStatus;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Human-readable description of the current extraction phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Opening and inspecting the archive.
    Inspecting,
    /// Checking available disk space.
    CheckingSpace,
    /// Actively extracting entries.
    Extracting,
    /// Performing integrity checks after extraction.
    Verifying,
    /// Reclaiming source storage.
    Reclaiming,
    /// All entries processed; session is finishing.
    Finalising,
    /// Session ended (may be success, failure, or cancellation).
    Done,
}

/// A snapshot of extraction progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressSnapshot {
    /// The current phase of the extraction session.
    pub phase: Phase,

    /// Path of the entry currently being processed.
    pub current_entry: Option<String>,

    /// Number of entries fully processed so far.
    pub entries_done: u64,

    /// Total number of entries in the archive (if known).
    pub entries_total: Option<u64>,

    /// Compressed bytes consumed from the source archive so far.
    pub compressed_bytes_read: u64,

    /// Decompressed bytes written to the destination so far.
    pub output_bytes_written: u64,

    /// Current throughput, measured in decompressed bytes per second.
    pub throughput_bps: f64,

    /// Wall-clock seconds elapsed since the session started.
    pub elapsed_secs: f64,

    /// Estimated remaining seconds, if a meaningful estimate is available.
    pub eta_secs: Option<f64>,

    /// Free bytes on the destination volume at the last measurement.
    pub dest_free_bytes: Option<u64>,

    /// Free bytes on the source volume at the last measurement.
    pub source_free_bytes: Option<u64>,

    /// Bytes confirmed reclaimed from the source archive, if reclamation is
    /// active.
    pub source_bytes_reclaimed: u64,
}

/// A warning that does not stop extraction but should be shown to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ExtractionWarning {
    /// An entry was skipped because the output path already existed.
    EntrySkipped { entry: String, reason: String },
    /// An entry was renamed because the output path already existed.
    EntryRenamed { entry: String, new_name: String },
    /// An entry's size was unknown; the estimate is less accurate.
    UnknownEntrySize { entry: String },
    /// Disk space is running low.
    LowDiskSpace {
        available_bytes: u64,
        threshold_bytes: u64,
    },
    /// An entry's symlink or reparse point was skipped due to policy.
    SymlinkSkipped { entry: String },
    /// The archive contains a feature that StreamExtract does not support
    /// for this operation, but which does not prevent basic extraction.
    UnsupportedFeature { detail: String },
}

/// A typed event emitted by the extraction engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ExtractionEvent {
    /// Extraction session has started. Sent once at the beginning.
    SessionStarted {
        archive_path: PathBuf,
        destination: PathBuf,
        session_id: String,
    },

    /// The archive has been inspected. Reports the number of entries (if
    /// known), the format name, and whether reclamation is eligible.
    ArchiveInspected {
        format: String,
        entry_count: Option<u64>,
        total_uncompressed_bytes: Option<u64>,
        reclamation_eligible: bool,
        reclamation_reason: Option<String>,
    },

    /// Regular progress update.
    Progress(ProgressSnapshot),

    /// A non-fatal warning occurred.
    Warning(ExtractionWarning),

    /// A single entry was successfully extracted and verified.
    EntryCompleted {
        entry: String,
        output_path: PathBuf,
        bytes_written: u64,
        verification: VerificationStatus,
    },

    /// An entry could not be extracted. Extraction continues with the
    /// next entry (unless the error is a security violation, in which case
    /// the session aborts).
    EntryFailed {
        entry: String,
        error: String,
        is_security_violation: bool,
    },

    /// Source storage reclamation was confirmed for a range of the archive.
    SourceRangeReclaimed {
        offset: u64,
        length: u64,
        bytes_freed: u64,
    },

    /// The extraction session ended successfully.
    SessionCompleted {
        entries_extracted: u64,
        entries_skipped: u64,
        entries_failed: u64,
        output_bytes_total: u64,
        source_bytes_reclaimed: u64,
        elapsed_secs: f64,
    },

    /// The extraction session failed and could not complete.
    SessionFailed {
        error: String,
        entries_extracted: u64,
        output_bytes_total: u64,
    },

    /// The extraction session was cancelled by the user.
    SessionCancelled {
        entries_extracted: u64,
        output_bytes_total: u64,
    },
}
