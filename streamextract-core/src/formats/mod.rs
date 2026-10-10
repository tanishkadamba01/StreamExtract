//! Format adapters for archive reading.
//!
//! Each adapter implements the `FormatAdapter` trait, which provides:
//! - Format detection
//! - Entry enumeration
//! - Streaming decompression
//! - Capability reporting (including whether reclamation is possible)

pub mod tar_adapter;
pub mod zip_adapter;

use crate::engine::storage_reclaimer::ReclamationCapabilityReport;
use crate::errors::Result;
use crate::models::archive_entry::ArchiveEntry;
use crate::models::extraction_options::ExtractionOptions;
use std::path::Path;

/// The format name returned by a successful detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    Tar,
    Unknown,
}

impl std::fmt::Display for ArchiveFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveFormat::Zip => write!(f, "ZIP"),
            ArchiveFormat::TarGz => write!(f, "TAR+GZIP"),
            ArchiveFormat::TarBz2 => write!(f, "TAR+BZIP2"),
            ArchiveFormat::TarXz => write!(f, "TAR+XZ"),
            ArchiveFormat::TarZst => write!(f, "TAR+ZSTD"),
            ArchiveFormat::Tar => write!(f, "TAR"),
            ArchiveFormat::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Detects the archive format of the file at `path` by reading its magic bytes.
///
/// This does not trust the file extension.
pub fn detect_format(path: &Path) -> Result<ArchiveFormat> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| {
        crate::errors::ExtractionError::ArchiveOpen {
            path: path.display().to_string(),
            source: e,
        }
    })?;

    let mut magic = [0u8; 8];
    let n = f.read(&mut magic).map_err(crate::errors::ExtractionError::Io)?;
    let magic = &magic[..n];

    // ZIP: local file header signature 0x504B0304 or end of central directory 0x504B0506
    if magic.len() >= 4 && &magic[..4] == b"PK\x03\x04" {
        return Ok(ArchiveFormat::Zip);
    }
    // Empty ZIP (end-of-central-directory only)
    if magic.len() >= 4 && &magic[..4] == b"PK\x05\x06" {
        return Ok(ArchiveFormat::Zip);
    }

    // GZIP: 0x1F 0x8B
    if magic.len() >= 2 && magic[0] == 0x1F && magic[1] == 0x8B {
        return Ok(ArchiveFormat::TarGz);
    }

    // BZIP2: "BZh"
    if magic.len() >= 3 && &magic[..3] == b"BZh" {
        return Ok(ArchiveFormat::TarBz2);
    }

    // XZ: 0xFD "7zXZ" 0x00
    if magic.len() >= 6 && &magic[..6] == b"\xFD7zXZ\x00" {
        return Ok(ArchiveFormat::TarXz);
    }

    // ZSTD: 0xFD2FB528 (little-endian magic)
    if magic.len() >= 4 && &magic[..4] == b"\x28\xB5\x2F\xFD" {
        return Ok(ArchiveFormat::TarZst);
    }

    // TAR: POSIX UStar header has "ustar" at offset 257.
    // For a bare TAR without wrapping, we try to read further.
    drop(f);
    if is_likely_tar(path) {
        return Ok(ArchiveFormat::Tar);
    }

    Ok(ArchiveFormat::Unknown)
}

/// Tries to detect a bare TAR by checking the ustar magic at offset 257.
fn is_likely_tar(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    if f.seek(SeekFrom::Start(257)).is_err() {
        return false;
    }
    let mut buf = [0u8; 5];
    if f.read_exact(&mut buf).is_err() {
        return false;
    }
    &buf == b"ustar"
}

/// Result of inspecting an archive.
pub struct InspectionResult {
    pub format: ArchiveFormat,
    pub entries: Vec<ArchiveEntry>,
    pub total_uncompressed_bytes: Option<u64>,
    pub reclamation: ReclamationCapabilityReport,
    /// Warning messages generated during inspection (non-fatal).
    pub warnings: Vec<String>,
}

/// Inspects an archive file and returns its entries and capabilities.
///
/// This function reads but does not extract the archive.
pub fn inspect_archive(
    path: &Path,
    _options: &ExtractionOptions,
) -> Result<InspectionResult> {
    let format = detect_format(path)?;
    match format {
        ArchiveFormat::Zip => zip_adapter::inspect(path),
        ArchiveFormat::Tar
        | ArchiveFormat::TarGz
        | ArchiveFormat::TarBz2
        | ArchiveFormat::TarXz
        | ArchiveFormat::TarZst => tar_adapter::inspect(path, &format),
        ArchiveFormat::Unknown => Err(crate::errors::ExtractionError::UnsupportedFormat {
            detail: format!(
                "file '{}' is not a recognised archive format",
                path.display()
            ),
        }),
    }
}
