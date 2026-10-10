//! TAR format adapter (bare TAR, TAR+GZIP, TAR+BZIP2, TAR+XZ, TAR+ZSTD).
//!
//! Uses the `tar` crate for reading and `flate2` for gzip decompression.
//!
//! # Reclamation eligibility
//!
//! TAR+GZIP (and other wrapped formats) is NOT eligible for storage
//! reclamation because:
//! - The outer gzip stream makes it impossible to identify independent
//!   byte ranges in the compressed source.
//! - The gzip decompressor may need previously consumed bytes as part of
//!   its decoder state.
//!
//! Bare TAR (uncompressed) could theoretically support range operations,
//! but this requires a separate Phase 6 investigation.
//!
//! All TAR variants currently return `reclamation_eligible = false`.

use crate::engine::storage_reclaimer::ReclamationCapabilityReport;
use crate::errors::{ExtractionError, Result};
use crate::formats::{ArchiveFormat, InspectionResult};
use crate::models::archive_entry::{ArchiveEntry, EntryKind, IntegrityInfo, VerificationStatus};
use chrono::{DateTime, Utc};
use std::fs::File;
use std::path::Path;

/// Inspects a TAR (or TAR+compression) archive and returns entry metadata.
pub fn inspect(path: &Path, format: &ArchiveFormat) -> Result<InspectionResult> {
    let file = File::open(path).map_err(|e| ExtractionError::ArchiveOpen {
        path: path.display().to_string(),
        source: e,
    })?;

    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    let mut total_uncompressed: u64 = 0;
    let mut sizes_known = true;

    match format {
        ArchiveFormat::TarGz => {
            let decoder = flate2::read::GzDecoder::new(file);
            let mut archive = tar::Archive::new(decoder);
            inspect_tar_entries(&mut archive, &mut entries, &mut warnings, &mut total_uncompressed, &mut sizes_known)?;
        }
        ArchiveFormat::TarBz2 => {
            let decoder = bzip2::read::BzDecoder::new(file);
            let mut archive = tar::Archive::new(decoder);
            inspect_tar_entries(&mut archive, &mut entries, &mut warnings, &mut total_uncompressed, &mut sizes_known)?;
        }
        ArchiveFormat::Tar => {
            let mut archive = tar::Archive::new(file);
            inspect_tar_entries(&mut archive, &mut entries, &mut warnings, &mut total_uncompressed, &mut sizes_known)?;
        }
        // XZ and Zstd are not yet supported in Phase 1 — report clearly.
        ArchiveFormat::TarXz | ArchiveFormat::TarZst => {
            return Err(ExtractionError::UnsupportedFormat {
                detail: format!(
                    "{format} support is not yet implemented in Phase 1"
                ),
            });
        }
        _ => unreachable!(),
    }

    let reclamation = ReclamationCapabilityReport::unsupported(
        "TAR stream reclamation requires Phase 6 investigation",
    );

    Ok(InspectionResult {
        format: format.clone(),
        entries,
        total_uncompressed_bytes: if sizes_known {
            Some(total_uncompressed)
        } else {
            None
        },
        reclamation,
        warnings,
    })
}

fn inspect_tar_entries<R: std::io::Read>(
    archive: &mut tar::Archive<R>,
    entries: &mut Vec<ArchiveEntry>,
    _warnings: &mut Vec<String>,
    total_uncompressed: &mut u64,
    sizes_known: &mut bool,
) -> Result<()> {
    for entry in archive.entries().map_err(|e| ExtractionError::CorruptedArchive {
        detail: format!("cannot read TAR entries: {e}"),
    })? {
        let entry = entry.map_err(|e| ExtractionError::CorruptedArchive {
            detail: format!("cannot read TAR entry: {e}"),
        })?;

        let header = entry.header();
        let raw_path = entry
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        let kind = match header.entry_type() {
            tar::EntryType::Regular | tar::EntryType::Continuous => EntryKind::File,
            tar::EntryType::Directory => EntryKind::Directory,
            tar::EntryType::Symlink => EntryKind::Symlink,
            tar::EntryType::Link => EntryKind::HardLink,
            _ => EntryKind::Unknown,
        };

        let size = header.size().ok();
        if size.is_none() {
            *sizes_known = false;
        }
        if let Some(s) = size {
            *total_uncompressed = total_uncompressed.saturating_add(s);
        }

        let last_modified: Option<DateTime<Utc>> = header
            .mtime()
            .ok()
            .map(|t| DateTime::from_timestamp(t as i64, 0))
            .flatten();

        entries.push(ArchiveEntry {
            raw_path,
            sanitised_path: None,
            kind,
            link_target: None,
            uncompressed_size: size,
            compressed_size: None, // TAR doesn't store compressed sizes per-entry
            last_modified,
            integrity: IntegrityInfo {
                crc32: None, // TAR does not provide CRC-32 per-entry
                extra_algorithm: None,
                extra_checksum: None,
            },
            archive_offset: None,
            archive_data_end: None,
            is_solid: !matches!(
                // Bare TAR entries are independent, but wrapped streams are not.
                // We mark as not-solid for bare TAR since entries are stored sequentially.
                entry.header().entry_type(),
                tar::EntryType::Regular | tar::EntryType::Continuous
            ) || false, // simplified: mark all as not-solid for Phase 1
            verification: VerificationStatus::Unavailable,
        });
    }
    Ok(())
}
