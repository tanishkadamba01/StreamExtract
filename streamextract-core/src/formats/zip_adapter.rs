//! ZIP format adapter.
//!
//! Uses the `zip` crate (v2.x) for reading. In Phase 1, this adapter
//! supports streaming extraction of independent (non-solid) ZIP archives
//! with Deflate, Store, Bzip2, and Zstd compression.
//!
//! # Streaming strategy
//!
//! The `zip` crate reads the central directory first (requiring a seekable
//! source), then streams each entry's compressed data via `ZipFile`, which
//! implements `Read`. We stream decompressed data to the output file in
//! bounded chunks rather than buffering the full entry in memory.
//!
//! # Reclamation eligibility (Phase 1)
//!
//! Reclamation is NOT enabled in Phase 1. The technical investigation
//! required by the specification (Phase 6) has not yet been completed.
//! The adapter reports `reclamation_eligible = false` for all archives.

use crate::engine::storage_reclaimer::ReclamationCapabilityReport;
use crate::errors::{ExtractionError, Result};
use crate::models::archive_entry::{ArchiveEntry, EntryKind, IntegrityInfo, VerificationStatus};
use crate::formats::InspectionResult;
use chrono::{TimeZone, Utc};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

/// Inspects a ZIP archive and returns entry metadata.
pub fn inspect(path: &Path) -> Result<InspectionResult> {
    let file = File::open(path).map_err(|e| ExtractionError::ArchiveOpen {
        path: path.display().to_string(),
        source: e,
    })?;

    let mut archive = ZipArchive::new(file).map_err(|e| ExtractionError::CorruptedArchive {
        detail: format!("cannot open ZIP '{}': {e}", path.display()),
    })?;

    let num_entries = archive.len();
    let mut entries = Vec::with_capacity(num_entries);
    let mut total_uncompressed: u64 = 0;
    let total_known = true;
    let mut warnings = Vec::new();
    let mut has_encrypted = false;
    let mut has_unsupported = false;

    for i in 0..num_entries {
        let zip_file = archive.by_index(i).map_err(|e| ExtractionError::CorruptedArchive {
            detail: format!("cannot read ZIP entry {i}: {e}"),
        })?;

        if zip_file.encrypted() {
            has_encrypted = true;
            warnings.push(format!(
                "Entry '{}' is encrypted; it will be skipped",
                zip_file.name()
            ));
            drop(zip_file);
            continue;
        }

        let uncompressed_size = zip_file.size();
        let compressed_size = zip_file.compressed_size();
        let crc32 = zip_file.crc32();
        let raw_path = zip_file.name().to_string();

        // Determine entry kind.
        let kind = if zip_file.is_dir() {
            EntryKind::Directory
        } else if zip_file.is_symlink() {
            EntryKind::Symlink
        } else {
            EntryKind::File
        };

        // Convert the ZIP last-modified time.
        let last_modified = zip_file
            .last_modified()
            .map(|dt| {
                Utc.with_ymd_and_hms(
                    dt.year().into(),
                    dt.month().into(),
                    dt.day().into(),
                    dt.hour().into(),
                    dt.minute().into(),
                    dt.second().into(),
                )
                .single()
            })
            .flatten();

        // Check for unsupported compression methods.
        match zip_file.compression() {
            zip::CompressionMethod::Stored
            | zip::CompressionMethod::Deflated
            | zip::CompressionMethod::Bzip2
            | zip::CompressionMethod::Zstd => {}
            other => {
                has_unsupported = true;
                warnings.push(format!(
                    "Entry '{}' uses unsupported compression '{other:?}'; it will be skipped",
                    zip_file.name()
                ));
                drop(zip_file);
                continue;
            }
        }

        if kind == EntryKind::File {
            total_uncompressed = total_uncompressed.saturating_add(uncompressed_size);
        }

        entries.push(ArchiveEntry {
            raw_path,
            sanitised_path: None, // validated at extraction time
            kind,
            link_target: None,
            uncompressed_size: Some(uncompressed_size),
            compressed_size: Some(compressed_size),
            last_modified,
            integrity: IntegrityInfo {
                crc32: Some(crc32),
                extra_algorithm: None,
                extra_checksum: None,
            },
            archive_offset: None, // Phase 6 will populate these
            archive_data_end: None,
            is_solid: false, // ZIP is never solid
            verification: VerificationStatus::Pending,
        });

        // Need to explicitly drop before next iteration to release the borrow.
        drop(zip_file);
    }

    let reclamation_reason = if has_encrypted {
        "archive contains encrypted entries"
    } else if has_unsupported {
        "archive contains unsupported compression methods"
    } else {
        "ZIP reclamation investigation not yet complete (Phase 6)"
    };

    let reclamation = ReclamationCapabilityReport::unsupported(reclamation_reason);

    Ok(InspectionResult {
        format: crate::formats::ArchiveFormat::Zip,
        entries,
        total_uncompressed_bytes: if total_known {
            Some(total_uncompressed)
        } else {
            None
        },
        reclamation,
        warnings,
    })
}

/// Extracts a single ZIP entry (identified by index) to `output_path`,
/// streaming decompressed data in chunks of `buffer_size` bytes.
///
/// The caller is responsible for:
/// - Path sanitisation (must happen before this call).
/// - Overwrite policy enforcement (open the file before calling this).
/// - Feeding the returned CRC-32 to an integrity verifier.
///
/// Returns the number of bytes written and the computed CRC-32.
pub fn extract_entry_to_file(
    archive_path: &Path,
    entry_index: usize,
    output_path: &Path,
    buffer_size: usize,
    cancel: &crate::engine::cancellation::CancellationToken,
) -> Result<(u64, u32)> {
    use crate::engine::integrity::Crc32Verifier;
    use crate::engine::output_writer::{finalise_file, open_output_file, write_chunk};
    use crate::models::extraction_options::OverwritePolicy;

    let file = File::open(archive_path).map_err(|e| ExtractionError::ArchiveOpen {
        path: archive_path.display().to_string(),
        source: e,
    })?;
    let mut archive = ZipArchive::new(file).map_err(|e| ExtractionError::CorruptedArchive {
        detail: format!("cannot open ZIP: {e}"),
    })?;

    let mut zip_entry = archive.by_index(entry_index).map_err(|e| {
        ExtractionError::CorruptedArchive {
            detail: format!("cannot read entry {entry_index}: {e}"),
        }
    })?;

    let expected_crc32 = zip_entry.crc32();
    let entry_name = zip_entry.name().to_string();

    // Open the output file (create_new semantics; caller pre-checked overwrite policy).
    let output_result =
        open_output_file(output_path, &OverwritePolicy::Overwrite)?;
    let (mut out_file, final_path) = match output_result {
        crate::engine::output_writer::OutputFileResult::Opened { file, path } => (file, path),
        crate::engine::output_writer::OutputFileResult::Renamed { file, path } => (file, path),
        crate::engine::output_writer::OutputFileResult::Skipped => {
            return Ok((0, 0));
        }
    };

    let mut verifier = Crc32Verifier::new(Some(expected_crc32));
    let mut total_written: u64 = 0;
    let mut buffer = vec![0u8; buffer_size];

    loop {
        // Check for cancellation before each chunk.
        cancel.check()?;

        let n = zip_entry.read(&mut buffer).map_err(|e| {
            ExtractionError::DecompressorError {
                entry: entry_name.clone(),
                detail: e.to_string(),
            }
        })?;

        if n == 0 {
            break;
        }

        let chunk = &buffer[..n];
        verifier.update(chunk);
        write_chunk(&mut out_file, chunk, &final_path)?;
        total_written = total_written.saturating_add(n as u64);
    }

    finalise_file(&mut out_file, &final_path)?;

    let (computed_crc32, _status) = verifier.finalise();
    Ok((total_written, computed_crc32))
}
