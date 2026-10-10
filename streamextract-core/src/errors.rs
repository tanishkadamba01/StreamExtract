//! Error types for the StreamExtract core engine.
//!
//! All errors carry enough context to produce useful diagnostic messages.
//! Categories are kept coarse-grained so callers can make safety decisions
//! (e.g., "is the archive corrupted?" vs "is a path malicious?") without
//! parsing string messages.

use thiserror::Error;

/// The unified error type for all StreamExtract operations.
#[derive(Debug, Error)]
pub enum ExtractionError {
    // ── Archive / Format errors ───────────────────────────────────────────
    /// The archive file could not be opened.
    #[error("Cannot open archive '{path}': {source}")]
    ArchiveOpen {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// The archive format was not recognised or is not supported.
    #[error("Unsupported or unrecognised archive format: {detail}")]
    UnsupportedFormat { detail: String },

    /// The archive is encrypted and decryption is not supported.
    #[error("Archive is encrypted; decryption is not supported")]
    EncryptedArchive,

    /// The archive is split across multiple volumes (not supported).
    #[error("Multi-volume archives are not supported")]
    MultiVolumeArchive,

    /// The archive data is corrupted, truncated, or otherwise malformed.
    #[error("Archive data is corrupted or truncated: {detail}")]
    CorruptedArchive { detail: String },

    /// An entry failed its integrity check (checksum mismatch).
    #[error("Integrity check failed for '{entry}': {detail}")]
    IntegrityCheckFailed { entry: String, detail: String },

    // ── Path / security errors ─────────────────────────────────────────────
    /// An archive entry contains a path component that would escape the
    /// destination directory (path traversal attack).
    #[error("Path traversal rejected for entry '{entry}': {detail}")]
    PathTraversal { entry: String, detail: String },

    /// An archive entry contains an absolute path, which is rejected.
    #[error("Absolute path rejected for entry '{entry}'")]
    AbsolutePath { entry: String },

    /// An archive entry uses a Windows reserved name (e.g. CON, NUL).
    #[error("Reserved Windows name rejected for entry '{entry}'")]
    ReservedName { entry: String },

    /// An archive entry name contains characters that are illegal on Windows.
    #[error("Illegal filename characters in entry '{entry}': {detail}")]
    IllegalFilename { entry: String, detail: String },

    // ── I/O / filesystem errors ───────────────────────────────────────────
    /// A generic I/O error from the standard library.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The destination volume does not have enough free space.
    #[error("Insufficient disk space on destination volume: need ~{needed_bytes} bytes, have {available_bytes} bytes")]
    InsufficientSpace {
        needed_bytes: u64,
        available_bytes: u64,
    },

    /// A write to an output file failed partway through.
    #[error("Output write failed for '{path}': {source}")]
    OutputWriteFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// The output path already exists and overwrite was not requested.
    #[error("Output path already exists: '{path}'")]
    OutputExists { path: String },

    // ── Decoder / streaming errors ────────────────────────────────────────
    /// The decompressor returned an error.
    #[error("Decompressor error for '{entry}': {detail}")]
    DecompressorError { entry: String, detail: String },

    /// An unexpected end-of-file was encountered while decompressing.
    #[error("Unexpected EOF while reading entry '{entry}'")]
    UnexpectedEof { entry: String },

    // ── Cancellation ──────────────────────────────────────────────────────
    /// The operation was cancelled by the user.
    #[error("Operation cancelled by user")]
    Cancelled,

    // ── Journal / recovery errors ─────────────────────────────────────────
    /// The recovery journal could not be read, written, or validated.
    #[error("Recovery journal error: {detail}")]
    JournalError { detail: String },

    // ── Storage reclamation errors ────────────────────────────────────────
    /// Storage reclamation is not supported for this archive.
    #[error("Storage reclamation not supported: {reason}")]
    ReclamationNotSupported { reason: String },

    /// A Windows API call for storage reclamation returned an error.
    #[error("Windows storage API error (code {code}): {detail}")]
    WindowsApiError { code: u32, detail: String },

    // ── Miscellaneous ─────────────────────────────────────────────────────
    /// An arithmetic overflow was detected in size or offset calculations.
    #[error("Arithmetic overflow in size/offset calculation: {detail}")]
    ArithmeticOverflow { detail: String },

    /// An internal logic error that should not occur in correct code.
    #[error("Internal error: {detail}")]
    Internal { detail: String },
}

/// Convenience type alias used throughout the crate.
pub type Result<T> = std::result::Result<T, ExtractionError>;

impl ExtractionError {
    /// Returns `true` if this error represents a security violation that
    /// should unconditionally abort extraction (as opposed to a recoverable
    /// I/O error that might be skipped or retried).
    pub fn is_security_violation(&self) -> bool {
        matches!(
            self,
            ExtractionError::PathTraversal { .. }
                | ExtractionError::AbsolutePath { .. }
                | ExtractionError::ReservedName { .. }
                | ExtractionError::IllegalFilename { .. }
        )
    }

    /// Returns `true` if extraction was cancelled rather than failed.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, ExtractionError::Cancelled)
    }

    /// Returns `true` if the error is due to data corruption.
    pub fn is_corruption(&self) -> bool {
        matches!(
            self,
            ExtractionError::CorruptedArchive { .. }
                | ExtractionError::IntegrityCheckFailed { .. }
                | ExtractionError::UnexpectedEof { .. }
                | ExtractionError::DecompressorError { .. }
        )
    }
}
