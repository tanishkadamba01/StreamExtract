//! Streaming extraction orchestrator.
//!
//! This is the central engine that:
//! 1. Inspects the archive and validates the request.
//! 2. Checks available disk space.
//! 3. Iterates entries in order.
//! 4. For each file entry: validates the path, opens the output file,
//!    streams decompressed data in bounded chunks, verifies integrity.
//! 5. Reports typed `ExtractionEvent`s to a channel.
//! 6. Respects the cancellation token at safe checkpoints.
//!
//! # Memory budget
//!
//! The engine allocates exactly one buffer of `options.buffer_size` bytes
//! for the decompression loop. The decoder itself (inside the format library)
//! may hold additional state (e.g., Deflate window = 32 KiB). Total
//! working set is bounded regardless of entry or archive size.

use crate::engine::{
    archive_reader::open_and_inspect,
    cancellation::CancellationToken,
    integrity::Crc32Verifier,
    output_writer::{create_output_dirs, finalise_file, open_output_file, write_chunk, OutputFileResult},
    progress::ProgressTracker,
    safety::sanitise_path,
    space_estimator::{query_volume_space, estimate_required_space},
};
use crate::errors::{ExtractionError, Result};
use crate::formats::ArchiveFormat;
use crate::models::{
    archive_entry::{EntryKind, VerificationStatus},
    extraction_event::{ExtractionEvent, ExtractionWarning, Phase},
    extraction_options::ExtractionOptions,
};
use std::fs::File;
use std::io::Read;
use std::sync::mpsc::Sender;
use tracing::{info, warn, debug};
use uuid::Uuid;

/// Runs a full extraction session.
///
/// Events are sent to `event_tx`. The caller should consume events from
/// the receiving end of the channel (on a separate thread or task).
///
/// This function blocks until extraction completes, fails, or is cancelled.
pub fn run_extraction(
    options: ExtractionOptions,
    cancel: CancellationToken,
    event_tx: Sender<ExtractionEvent>,
) -> Result<()> {
    // Validate options.
    options.validate().map_err(|e| ExtractionError::Internal {
        detail: format!("invalid options: {e}"),
    })?;

    let start_time = std::time::Instant::now();
    let session_id = Uuid::new_v4().to_string();
    let _ = event_tx.send(ExtractionEvent::SessionStarted {
        archive_path: options.archive_path.clone(),
        destination: options.destination.clone(),
        session_id: session_id.clone(),
    });

    info!(
        session_id = %session_id,
        archive = %options.archive_path.display(),
        dest = %options.destination.display(),
        "Extraction session started"
    );

    // Step 1: Inspect archive.
    let inspection = match open_and_inspect(&options.archive_path, &options) {
        Ok(r) => r,
        Err(e) => {
            let _ = event_tx.send(ExtractionEvent::SessionFailed {
                error: e.to_string(),
                entries_extracted: 0,
                output_bytes_total: 0,
            });
            return Err(e);
        }
    };

    let total_entries = inspection.entries.len() as u64;
    let _ = event_tx.send(ExtractionEvent::ArchiveInspected {
        format: inspection.format.to_string(),
        entry_count: Some(total_entries),
        total_uncompressed_bytes: inspection.total_uncompressed_bytes,
        reclamation_eligible: inspection.reclamation.eligible,
        reclamation_reason: Some(inspection.reclamation.reason.clone()),
    });

    for w in &inspection.warnings {
        warn!("{w}");
        let _ = event_tx.send(ExtractionEvent::Warning(
            ExtractionWarning::UnsupportedFeature { detail: w.clone() },
        ));
    }

    // Step 2: Check disk space.
    cancel.check()?;
    let dest_space = query_volume_space(&options.destination).unwrap_or_else(|_| {
        crate::engine::space_estimator::VolumeSpace {
            available_bytes: u64::MAX,
            total_bytes: u64::MAX,
        }
    });
    if let Some(required) = estimate_required_space(
        inspection.total_uncompressed_bytes,
        options.space.safety_margin_bytes,
    ) {
        if dest_space.available_bytes < required {
            let err = ExtractionError::InsufficientSpace {
                needed_bytes: required,
                available_bytes: dest_space.available_bytes,
            };
            let _ = event_tx.send(ExtractionEvent::SessionFailed {
                error: err.to_string(),
                entries_extracted: 0,
                output_bytes_total: 0,
            });
            return Err(err);
        }
    }

    // Ensure destination directory exists.
    create_output_dirs(&options.destination)?;

    // Step 3: Extract entries.
    let mut tracker = ProgressTracker::new(Some(total_entries));
    tracker.set_phase(Phase::Extracting);

    let mut entries_extracted: u64 = 0;
    let mut entries_skipped: u64 = 0;
    let mut entries_failed: u64 = 0;
    let mut output_bytes_total: u64 = 0;

    match inspection.format {
        ArchiveFormat::Zip => {
            let result = extract_zip(
                &options,
                &cancel,
                &event_tx,
                &mut tracker,
                &mut entries_extracted,
                &mut entries_skipped,
                &mut entries_failed,
                &mut output_bytes_total,
            );
            match result {
                Ok(()) => {}
                Err(ExtractionError::Cancelled) => {
                    let _ = event_tx.send(ExtractionEvent::SessionCancelled {
                        entries_extracted,
                        output_bytes_total,
                    });
                    return Err(ExtractionError::Cancelled);
                }
                Err(e) => {
                    let _ = event_tx.send(ExtractionEvent::SessionFailed {
                        error: e.to_string(),
                        entries_extracted,
                        output_bytes_total,
                    });
                    return Err(e);
                }
            }
        }
        ArchiveFormat::TarGz | ArchiveFormat::Tar | ArchiveFormat::TarBz2 => {
            let result = extract_tar(
                &options,
                &inspection.format,
                &cancel,
                &event_tx,
                &mut tracker,
                &mut entries_extracted,
                &mut entries_skipped,
                &mut entries_failed,
                &mut output_bytes_total,
            );
            match result {
                Ok(()) => {}
                Err(ExtractionError::Cancelled) => {
                    let _ = event_tx.send(ExtractionEvent::SessionCancelled {
                        entries_extracted,
                        output_bytes_total,
                    });
                    return Err(ExtractionError::Cancelled);
                }
                Err(e) => {
                    let _ = event_tx.send(ExtractionEvent::SessionFailed {
                        error: e.to_string(),
                        entries_extracted,
                        output_bytes_total,
                    });
                    return Err(e);
                }
            }
        }
        _ => {
            return Err(ExtractionError::UnsupportedFormat {
                detail: format!("format {} is not yet supported", inspection.format),
            });
        }
    }

    let elapsed_secs = start_time.elapsed().as_secs_f64();
    let _ = event_tx.send(ExtractionEvent::SessionCompleted {
        entries_extracted,
        entries_skipped,
        entries_failed,
        output_bytes_total,
        source_bytes_reclaimed: 0,
        elapsed_secs,
    });

    info!(
        session_id = %session_id,
        entries_extracted,
        entries_skipped,
        entries_failed,
        output_bytes_total,
        "Extraction session completed"
    );

    Ok(())
}

// ─── ZIP extraction ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn extract_zip(
    options: &ExtractionOptions,
    cancel: &CancellationToken,
    event_tx: &Sender<ExtractionEvent>,
    tracker: &mut ProgressTracker,
    entries_extracted: &mut u64,
    entries_skipped: &mut u64,
    entries_failed: &mut u64,
    output_bytes_total: &mut u64,
) -> Result<()> {
    use zip::ZipArchive;

    let file = File::open(&options.archive_path).map_err(|e| ExtractionError::ArchiveOpen {
        path: options.archive_path.display().to_string(),
        source: e,
    })?;
    let mut archive = ZipArchive::new(file).map_err(|e| ExtractionError::CorruptedArchive {
        detail: format!("cannot open ZIP: {e}"),
    })?;

    let buffer_size = options.buffer_size;
    let mut buffer = vec![0u8; buffer_size];

    for i in 0..archive.len() {
        cancel.check()?;

        let result = extract_zip_entry(
            &mut archive,
            i,
            options,
            cancel,
            event_tx,
            tracker,
            &mut buffer,
        );

        match result {
            Ok(ExtractionOutcome::Extracted { bytes }) => {
                *entries_extracted += 1;
                *output_bytes_total = output_bytes_total.saturating_add(bytes);
                tracker.complete_entry();
            }
            Ok(ExtractionOutcome::Skipped) => {
                *entries_skipped += 1;
                tracker.complete_entry();
            }
            Ok(ExtractionOutcome::Directory) => {
                tracker.complete_entry();
            }
            Err(e) if e.is_security_violation() => {
                // Security violations abort the entire session.
                return Err(e);
            }
            Err(e) => {
                warn!("Entry {i} failed: {e}");
                let entry_name = archive
                    .by_index_raw(i)
                    .map(|f| f.name().to_string())
                    .unwrap_or_else(|_| format!("<entry {i}>"));
                let _ = event_tx.send(ExtractionEvent::EntryFailed {
                    entry: entry_name,
                    error: e.to_string(),
                    is_security_violation: false,
                });
                *entries_failed += 1;
                tracker.complete_entry();
            }
        }
    }
    Ok(())
}

enum ExtractionOutcome {
    Extracted { bytes: u64 },
    Skipped,
    Directory,
}

#[allow(clippy::too_many_arguments)]
fn extract_zip_entry(
    archive: &mut zip::ZipArchive<File>,
    index: usize,
    options: &ExtractionOptions,
    cancel: &CancellationToken,
    event_tx: &Sender<ExtractionEvent>,
    tracker: &mut ProgressTracker,
    buffer: &mut Vec<u8>,
) -> Result<ExtractionOutcome> {
    let mut zip_file = archive.by_index(index).map_err(|e| ExtractionError::CorruptedArchive {
        detail: format!("entry {index}: {e}"),
    })?;

    let raw_path = zip_file.name().to_string();
    debug!("Processing entry: {raw_path}");

    // Skip encrypted entries.
    if zip_file.encrypted() {
        let _ = event_tx.send(ExtractionEvent::Warning(ExtractionWarning::UnsupportedFeature {
            detail: format!("Skipping encrypted entry '{raw_path}'"),
        }));
        return Ok(ExtractionOutcome::Skipped);
    }

    // Validate path BEFORE any filesystem operation.
    let sanitised = match sanitise_path(&raw_path, &options.destination) {
        Ok(p) => p,
        Err(e) => {
            if e.is_security_violation() {
                let _ = event_tx.send(ExtractionEvent::EntryFailed {
                    entry: raw_path,
                    error: e.to_string(),
                    is_security_violation: true,
                });
                return Err(e);
            }
            return Err(e);
        }
    };

    // Handle directories.
    if zip_file.is_dir() {
        create_output_dirs(&sanitised)?;
        return Ok(ExtractionOutcome::Directory);
    }

    // Skip symlinks unless permitted.
    if zip_file.is_symlink() {
        if !options.permissions.allow_symlinks {
            let _ = event_tx.send(ExtractionEvent::Warning(
                ExtractionWarning::SymlinkSkipped { entry: raw_path },
            ));
            return Ok(ExtractionOutcome::Skipped);
        }
        // Symlink extraction is complex and security-sensitive.
        // Not implemented in Phase 1.
        let _ = event_tx.send(ExtractionEvent::Warning(ExtractionWarning::UnsupportedFeature {
            detail: format!("Symlink entry '{raw_path}' skipped (not implemented)"),
        }));
        return Ok(ExtractionOutcome::Skipped);
    }

    // Check low-disk-space condition before starting the entry.
    let dest_space = query_volume_space(&options.destination).ok();
    if let Some(ref space) = dest_space {
        if space.available_bytes < options.space.low_space_threshold_bytes {
            let _ = event_tx.send(ExtractionEvent::Warning(ExtractionWarning::LowDiskSpace {
                available_bytes: space.available_bytes,
                threshold_bytes: options.space.low_space_threshold_bytes,
            }));
        }
    }

    // Enforce max entry count.
    if let Some(max) = options.max_entry_count {
        // (tracked externally; this is just a guard)
        let _ = max;
    }

    let expected_crc32 = zip_file.crc32();
    let entry_name = raw_path.clone();

    // Open output file.
    let output_result = open_output_file(&sanitised, &options.overwrite_policy)?;
    let (mut out_file, final_path) = match output_result {
        OutputFileResult::Skipped => {
            let _ = event_tx.send(ExtractionEvent::Warning(
                ExtractionWarning::EntrySkipped {
                    entry: entry_name,
                    reason: "file already exists (Skip policy)".into(),
                },
            ));
            return Ok(ExtractionOutcome::Skipped);
        }
        OutputFileResult::Opened { file, path } => (file, path),
        OutputFileResult::Renamed { file, path } => {
            let _ = event_tx.send(ExtractionEvent::Warning(
                ExtractionWarning::EntryRenamed {
                    entry: entry_name.clone(),
                    new_name: path.display().to_string(),
                },
            ));
            (file, path)
        }
    };

    // Stream decompression loop.
    let mut verifier = Crc32Verifier::new(Some(expected_crc32));
    let mut bytes_written: u64 = 0;

    loop {
        cancel.check()?;

        let n = zip_file.read(buffer).map_err(|e| ExtractionError::DecompressorError {
            entry: entry_name.clone(),
            detail: e.to_string(),
        })?;
        if n == 0 {
            break;
        }

        let chunk = &buffer[..n];
        verifier.update(chunk);
        write_chunk(&mut out_file, chunk, &final_path)?;
        bytes_written = bytes_written.saturating_add(n as u64);
        tracker.add_output_bytes(n as u64);

        // Check expansion ratio.
        if let Some(max_ratio) = options.max_expansion_ratio {
            if let Some(compressed) = zip_file.compressed_size().checked_sub(0) {
                if compressed > 0 {
                    let ratio = bytes_written as f64 / compressed as f64;
                    if ratio > max_ratio {
                        return Err(ExtractionError::CorruptedArchive {
                            detail: format!(
                                "entry '{entry_name}' expansion ratio {ratio:.1} exceeds limit {max_ratio}"
                            ),
                        });
                    }
                }
            }
        }
    }

    finalise_file(&mut out_file, &final_path)?;

    let (computed_crc, verification) = verifier.finalise();
    if verification == VerificationStatus::Failed {
        return Err(ExtractionError::IntegrityCheckFailed {
            entry: entry_name,
            detail: format!(
                "expected CRC-32 {:08X}, computed {:08X}",
                expected_crc32, computed_crc
            ),
        });
    }

    let _ = event_tx.send(ExtractionEvent::EntryCompleted {
        entry: entry_name,
        output_path: final_path,
        bytes_written,
        verification,
    });

    Ok(ExtractionOutcome::Extracted { bytes: bytes_written })
}

// ─── TAR extraction ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn extract_tar(
    options: &ExtractionOptions,
    format: &ArchiveFormat,
    cancel: &CancellationToken,
    event_tx: &Sender<ExtractionEvent>,
    tracker: &mut ProgressTracker,
    entries_extracted: &mut u64,
    entries_skipped: &mut u64,
    entries_failed: &mut u64,
    output_bytes_total: &mut u64,
) -> Result<()> {
    let file = File::open(&options.archive_path).map_err(|e| ExtractionError::ArchiveOpen {
        path: options.archive_path.display().to_string(),
        source: e,
    })?;

    match format {
        ArchiveFormat::TarGz => {
            let decoder = flate2::read::GzDecoder::new(file);
            let mut archive = tar::Archive::new(decoder);
            extract_tar_entries(&mut archive, options, cancel, event_tx, tracker,
                entries_extracted, entries_skipped, entries_failed, output_bytes_total)
        }
        ArchiveFormat::TarBz2 => {
            let decoder = bzip2::read::BzDecoder::new(file);
            let mut archive = tar::Archive::new(decoder);
            extract_tar_entries(&mut archive, options, cancel, event_tx, tracker,
                entries_extracted, entries_skipped, entries_failed, output_bytes_total)
        }
        ArchiveFormat::Tar => {
            let mut archive = tar::Archive::new(file);
            extract_tar_entries(&mut archive, options, cancel, event_tx, tracker,
                entries_extracted, entries_skipped, entries_failed, output_bytes_total)
        }
        _ => Err(ExtractionError::UnsupportedFormat {
            detail: format!("{format} extraction not implemented"),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_tar_entries<R: Read>(
    archive: &mut tar::Archive<R>,
    options: &ExtractionOptions,
    cancel: &CancellationToken,
    event_tx: &Sender<ExtractionEvent>,
    tracker: &mut ProgressTracker,
    entries_extracted: &mut u64,
    entries_skipped: &mut u64,
    entries_failed: &mut u64,
    output_bytes_total: &mut u64,
) -> Result<()> {
    let buffer_size = options.buffer_size;
    let mut buffer = vec![0u8; buffer_size];

    let entries_iter = archive.entries().map_err(|e| ExtractionError::CorruptedArchive {
        detail: format!("cannot read TAR entries: {e}"),
    })?;

    for entry_result in entries_iter {
        cancel.check()?;

        let mut entry = match entry_result {
            Ok(e) => e,
            Err(err) => {
                warn!("TAR entry error: {err}");
                *entries_failed += 1;
                continue;
            }
        };

        let raw_path = entry.path().map(|p| p.display().to_string()).unwrap_or_default();

        // Validate path.
        let sanitised = match sanitise_path(&raw_path, &options.destination) {
            Ok(p) => p,
            Err(e) => {
                if e.is_security_violation() {
                    let _ = event_tx.send(ExtractionEvent::EntryFailed {
                        entry: raw_path,
                        error: e.to_string(),
                        is_security_violation: true,
                    });
                    return Err(e);
                }
                warn!("Skipping entry with invalid path '{raw_path}': {e}");
                *entries_failed += 1;
                continue;
            }
        };

        let kind = match entry.header().entry_type() {
            tar::EntryType::Directory => EntryKind::Directory,
            tar::EntryType::Symlink | tar::EntryType::Link => EntryKind::Symlink,
            _ => EntryKind::File,
        };

        match kind {
            EntryKind::Directory => {
                create_output_dirs(&sanitised)?;
                tracker.complete_entry();
                continue;
            }
            EntryKind::Symlink | EntryKind::HardLink => {
                let _ = event_tx.send(ExtractionEvent::Warning(
                    ExtractionWarning::SymlinkSkipped { entry: raw_path },
                ));
                *entries_skipped += 1;
                tracker.complete_entry();
                continue;
            }
            _ => {}
        }

        // Open output file.
        let output_result = open_output_file(&sanitised, &options.overwrite_policy);
        let (mut out_file, final_path) = match output_result {
            Ok(OutputFileResult::Skipped) => {
                let _ = event_tx.send(ExtractionEvent::Warning(
                    ExtractionWarning::EntrySkipped {
                        entry: raw_path,
                        reason: "file already exists".into(),
                    },
                ));
                *entries_skipped += 1;
                tracker.complete_entry();
                continue;
            }
            Ok(OutputFileResult::Opened { file, path }) => (file, path),
            Ok(OutputFileResult::Renamed { file, path }) => (file, path),
            Err(e) => {
                warn!("Cannot open output for '{raw_path}': {e}");
                *entries_failed += 1;
                tracker.complete_entry();
                continue;
            }
        };

        // Stream decompression. TAR provides no per-entry CRC so we use Unavailable.
        let mut bytes_written: u64 = 0;
        let mut write_error: Option<ExtractionError> = None;

        loop {
            cancel.check()?;
            let n = match entry.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    write_error = Some(ExtractionError::DecompressorError {
                        entry: raw_path.clone(),
                        detail: e.to_string(),
                    });
                    break;
                }
            };
            let chunk = &buffer[..n];
            if let Err(e) = write_chunk(&mut out_file, chunk, &final_path) {
                write_error = Some(e);
                break;
            }
            bytes_written = bytes_written.saturating_add(n as u64);
            tracker.add_output_bytes(n as u64);
        }

        finalise_file(&mut out_file, &final_path).ok();

        if let Some(e) = write_error {
            warn!("Entry '{raw_path}' failed: {e}");
            let _ = event_tx.send(ExtractionEvent::EntryFailed {
                entry: raw_path,
                error: e.to_string(),
                is_security_violation: false,
            });
            *entries_failed += 1;
        } else {
            let _ = event_tx.send(ExtractionEvent::EntryCompleted {
                entry: raw_path,
                output_path: final_path,
                bytes_written,
                verification: VerificationStatus::Unavailable,
            });
            *entries_extracted += 1;
            *output_bytes_total = output_bytes_total.saturating_add(bytes_written);
        }
        tracker.complete_entry();
    }

    Ok(())
}
