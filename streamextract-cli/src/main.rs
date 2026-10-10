//! StreamExtract CLI — test harness and command-line interface.
//!
//! # Usage
//!
//! ```text
//! streamextract inspect <ARCHIVE>
//! streamextract extract <ARCHIVE> <DESTINATION> [OPTIONS]
//! ```

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::mpsc;
use streamextract_core::{
    engine::streaming::run_extraction,
    formats::inspect_archive,
    models::extraction_event::ExtractionEvent,
    models::extraction_options::ExtractionOptions,
    engine::cancellation::CancellationToken,
};
use tracing::{error, info};

#[derive(Parser)]
#[command(
    name = "streamextract",
    version,
    about = "Low-disk-overhead streaming archive extractor",
    long_about = None,
)]
struct Cli {
    /// Set verbosity. Use -v for debug, -vv for trace.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Inspect an archive: detect format, list entries, report capabilities.
    Inspect {
        /// Path to the archive file.
        archive: PathBuf,
    },

    /// Extract an archive to a destination directory.
    Extract {
        /// Path to the archive file.
        archive: PathBuf,

        /// Destination directory for extracted files.
        destination: PathBuf,

        /// Buffer size for streaming decompression (in KiB, 64–32768).
        #[arg(long, default_value = "1024", value_name = "KiB")]
        buffer_kib: usize,

        /// Overwrite policy for existing files: skip, overwrite, rename, abort.
        #[arg(long, default_value = "skip", value_name = "POLICY")]
        overwrite: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up structured logging.
    let level = match cli.verbose {
        0 => tracing::Level::INFO,
        1 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .compact()
        .init();

    match cli.command {
        Commands::Inspect { archive } => cmd_inspect(&archive),
        Commands::Extract {
            archive,
            destination,
            buffer_kib,
            overwrite,
        } => cmd_extract(&archive, &destination, buffer_kib, &overwrite),
    }
}

fn cmd_inspect(archive: &PathBuf) -> Result<()> {
    info!("Inspecting: {}", archive.display());

    let options = ExtractionOptions {
        archive_path: archive.clone(),
        destination: PathBuf::from("."),
        ..Default::default()
    };

    let result = inspect_archive(archive, &options)
        .context("Failed to inspect archive")?;

    println!("Format   : {}", result.format);
    println!("Entries  : {}", result.entries.len());
    if let Some(total) = result.total_uncompressed_bytes {
        println!("Total    : {} bytes ({:.1} MiB)", total, total as f64 / 1_048_576.0);
    } else {
        println!("Total    : unknown (sizes not reported by archive)");
    }
    println!(
        "Reclaim  : {} — {}",
        if result.reclamation.eligible { "ELIGIBLE" } else { "NOT ELIGIBLE" },
        result.reclamation.reason
    );

    if !result.warnings.is_empty() {
        println!("\nWarnings:");
        for w in &result.warnings {
            println!("  ⚠ {w}");
        }
    }

    println!("\n{:<60} {:>12} {:>12} {}", "Path", "Compressed", "Uncompressed", "Kind");
    println!("{}", "-".repeat(95));
    for entry in &result.entries {
        println!(
            "{:<60} {:>12} {:>12} {}",
            truncate(&entry.raw_path, 59),
            entry.compressed_size.map(|s| format_bytes(s)).unwrap_or_else(|| "?".into()),
            entry.uncompressed_size.map(|s| format_bytes(s)).unwrap_or_else(|| "?".into()),
            entry.kind,
        );
    }

    Ok(())
}

fn cmd_extract(
    archive: &PathBuf,
    destination: &PathBuf,
    buffer_kib: usize,
    overwrite_str: &str,
) -> Result<()> {
    use streamextract_core::models::extraction_options::OverwritePolicy;

    let overwrite_policy = match overwrite_str.to_lowercase().as_str() {
        "skip" => OverwritePolicy::Skip,
        "overwrite" => OverwritePolicy::Overwrite,
        "rename" => OverwritePolicy::Rename,
        "abort" => OverwritePolicy::Abort,
        other => anyhow::bail!("Unknown overwrite policy: '{other}'. Use skip, overwrite, rename, or abort."),
    };

    let options = ExtractionOptions {
        archive_path: archive.clone(),
        destination: destination.clone(),
        buffer_size: buffer_kib * 1024,
        overwrite_policy,
        ..Default::default()
    };

    options.validate().map_err(|e| anyhow::anyhow!("Invalid options: {e}"))?;

    info!(
        "Extracting '{}' → '{}'",
        archive.display(),
        destination.display()
    );
    info!("Buffer: {} KiB, Overwrite: {overwrite_str}", buffer_kib);

    let (tx, rx) = mpsc::channel::<ExtractionEvent>();
    let cancel = CancellationToken::new();

    // Handle Ctrl-C cancellation.
    let cancel_clone = cancel.clone();
    ctrlc::set_handler(move || {
        eprintln!("\nCancellation requested…");
        cancel_clone.cancel();
    })
    .context("Failed to set Ctrl-C handler")?;

    // Consume events on the main thread (extraction runs synchronously here).
    // In a real async setup, events would be consumed on a separate task.
    let event_thread = std::thread::spawn(move || {
        let mut last_progress = std::time::Instant::now();
        for event in rx {
            match &event {
                ExtractionEvent::Progress(snap) => {
                    if last_progress.elapsed().as_millis() >= 250 {
                        print!(
                            "\r  [{done}/{total}] {mib:.1} MiB | {speed:.1} MB/s",
                            done = snap.entries_done,
                            total = snap.entries_total.map(|t| t.to_string()).unwrap_or("?".into()),
                            mib = snap.output_bytes_written as f64 / 1_048_576.0,
                            speed = snap.throughput_bps / 1_000_000.0,
                        );
                        last_progress = std::time::Instant::now();
                    }
                }
                ExtractionEvent::EntryCompleted { entry, bytes_written, verification, .. } => {
                    info!("  ✓ {entry} ({bytes_written} bytes, {verification:?})");
                }
                ExtractionEvent::EntryFailed { entry, error, .. } => {
                    error!("  ✗ {entry}: {error}");
                }
                ExtractionEvent::Warning(w) => {
                    eprintln!("  ⚠ {w:?}");
                }
                ExtractionEvent::SessionCompleted { entries_extracted, entries_failed, output_bytes_total, elapsed_secs, .. } => {
                    println!();
                    println!("─────────────────────────────────────────────────");
                    println!("  Extracted : {entries_extracted} entries");
                    println!("  Failed    : {entries_failed} entries");
                    println!("  Output    : {:.1} MiB", *output_bytes_total as f64 / 1_048_576.0);
                    println!("  Elapsed   : {elapsed_secs:.1}s");
                    println!("─────────────────────────────────────────────────");
                }
                ExtractionEvent::SessionFailed { error, .. } => {
                    error!("Extraction failed: {error}");
                }
                ExtractionEvent::SessionCancelled { entries_extracted, .. } => {
                    eprintln!("\nCancelled after {entries_extracted} entries.");
                }
                _ => {}
            }
        }
    });

    run_extraction(options, cancel, tx).context("Extraction failed")?;
    event_thread.join().ok();

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("…{}", &s[s.len() - max_len + 1..])
    }
}

fn format_bytes(b: u64) -> String {
    if b >= 1_048_576 {
        format!("{:.1}M", b as f64 / 1_048_576.0)
    } else if b >= 1024 {
        format!("{:.1}K", b as f64 / 1024.0)
    } else {
        format!("{b}B")
    }
}
