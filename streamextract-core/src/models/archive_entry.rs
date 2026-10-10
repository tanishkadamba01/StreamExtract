//! Represents a single entry (file, directory, symlink) within an archive.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The type of an archive entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link (target is stored separately).
    Symlink,
    /// A hard link (target is stored separately).
    HardLink,
    /// An entry type that StreamExtract does not recognise or support.
    Unknown,
}

impl std::fmt::Display for EntryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntryKind::File => write!(f, "file"),
            EntryKind::Directory => write!(f, "directory"),
            EntryKind::Symlink => write!(f, "symlink"),
            EntryKind::HardLink => write!(f, "hard link"),
            EntryKind::Unknown => write!(f, "unknown"),
        }
    }
}

/// Integrity metadata for an entry, if the archive provides it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityInfo {
    /// CRC-32 checksum as reported by the archive, if present.
    pub crc32: Option<u32>,
    /// The algorithm used by any additional checksum, if present.
    pub extra_algorithm: Option<String>,
    /// The extra checksum value as a hex string.
    pub extra_checksum: Option<String>,
}

/// The verification result for an entry after extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    /// The archive provides no checksum; verification is not possible.
    Unavailable,
    /// The checksum matched the extracted data.
    Passed,
    /// The checksum did not match the extracted data.
    Failed,
    /// Verification has not yet been attempted.
    Pending,
}

/// Metadata for a single archive entry, as reported by the format adapter.
///
/// Sizes are `Option` because some archive formats (particularly streaming TAR)
/// may not report them reliably until after the entry is fully decoded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveEntry {
    /// The path of this entry as stored in the archive (not yet sanitised).
    pub raw_path: String,

    /// The sanitised, platform-safe path relative to the extraction root.
    /// `None` if the path has not yet been validated.
    pub sanitised_path: Option<std::path::PathBuf>,

    /// The kind of entry (file, directory, symlink, …).
    pub kind: EntryKind,

    /// If `kind` is `Symlink` or `HardLink`, the path the link points to.
    pub link_target: Option<String>,

    /// The declared uncompressed size in bytes, if known.
    pub uncompressed_size: Option<u64>,

    /// The compressed size in bytes as stored in the archive, if known.
    pub compressed_size: Option<u64>,

    /// The last-modified timestamp from the archive, if present.
    pub last_modified: Option<DateTime<Utc>>,

    /// Integrity metadata provided by the archive format.
    pub integrity: IntegrityInfo,

    /// The byte offset of this entry's local header within the archive file.
    /// Used by the storage reclamation engine; `None` for formats that do
    /// not expose offsets.
    pub archive_offset: Option<u64>,

    /// The byte offset just past the last byte of this entry's compressed
    /// data within the archive file. Used for reclamation range tracking.
    pub archive_data_end: Option<u64>,

    /// `true` if this entry uses a compression method that requires solid
    /// decoding (i.e., earlier compressed data cannot be discarded while
    /// later entries are still being extracted).
    pub is_solid: bool,

    /// The current verification status. Updated after extraction completes.
    pub verification: VerificationStatus,
}

impl ArchiveEntry {
    /// Returns `true` if this entry is a regular file that contains data.
    pub fn is_file(&self) -> bool {
        self.kind == EntryKind::File
    }

    /// Returns `true` if this entry is a directory.
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Directory
    }

    /// Returns the display-friendly name for logging.
    pub fn display_name(&self) -> &str {
        &self.raw_path
    }
}
