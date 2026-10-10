//! Progress tracking for extraction sessions.
//!
//! Measures throughput, estimates completion time, and assembles
//! `ProgressSnapshot` values for delivery to the event channel.

use crate::models::extraction_event::{Phase, ProgressSnapshot};
use std::time::Instant;

/// Tracks extraction progress and computes derived metrics.
pub struct ProgressTracker {
    phase: Phase,
    entries_done: u64,
    entries_total: Option<u64>,
    compressed_bytes_read: u64,
    output_bytes_written: u64,
    source_bytes_reclaimed: u64,
    started_at: Instant,
    /// Bytes written at the time of the last throughput sample.
    last_sample_bytes: u64,
    /// Wall time at the last throughput sample.
    last_sample_time: Instant,
    /// Exponential-moving-average throughput in bytes/sec.
    throughput_ema_bps: f64,
    /// Smoothing factor for the EMA (0 < alpha ≤ 1).
    alpha: f64,
}

impl ProgressTracker {
    const EMA_ALPHA: f64 = 0.2;

    pub fn new(entries_total: Option<u64>) -> Self {
        let now = Instant::now();
        ProgressTracker {
            phase: Phase::Inspecting,
            entries_done: 0,
            entries_total,
            compressed_bytes_read: 0,
            output_bytes_written: 0,
            source_bytes_reclaimed: 0,
            started_at: now,
            last_sample_bytes: 0,
            last_sample_time: now,
            throughput_ema_bps: 0.0,
            alpha: Self::EMA_ALPHA,
        }
    }

    /// Update the current phase.
    pub fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
    }

    /// Record that `bytes` of compressed input were consumed.
    pub fn add_compressed_bytes(&mut self, bytes: u64) {
        self.compressed_bytes_read = self.compressed_bytes_read.saturating_add(bytes);
    }

    /// Record that `bytes` of decompressed output were written.
    pub fn add_output_bytes(&mut self, bytes: u64) {
        self.output_bytes_written = self.output_bytes_written.saturating_add(bytes);
        self.update_throughput();
    }

    /// Record that an entry was fully completed.
    pub fn complete_entry(&mut self) {
        self.entries_done = self.entries_done.saturating_add(1);
    }

    /// Record confirmed source-storage reclamation.
    pub fn add_reclaimed_bytes(&mut self, bytes: u64) {
        self.source_bytes_reclaimed = self.source_bytes_reclaimed.saturating_add(bytes);
    }

    /// Update the throughput EMA based on recent output bytes.
    fn update_throughput(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_sample_time).as_secs_f64();
        if elapsed > 0.1 {
            let bytes_since = self
                .output_bytes_written
                .saturating_sub(self.last_sample_bytes) as f64;
            let instant_bps = bytes_since / elapsed;
            if self.throughput_ema_bps == 0.0 {
                self.throughput_ema_bps = instant_bps;
            } else {
                self.throughput_ema_bps =
                    self.alpha * instant_bps + (1.0 - self.alpha) * self.throughput_ema_bps;
            }
            self.last_sample_bytes = self.output_bytes_written;
            self.last_sample_time = now;
        }
    }

    /// Compute ETA in seconds, if possible.
    fn eta_secs(&self) -> Option<f64> {
        let total = self.entries_total? as f64;
        let done = self.entries_done as f64;
        if done == 0.0 || self.throughput_ema_bps <= 0.0 {
            return None;
        }
        // Estimate remaining bytes proportionally.
        let fraction_done = done / total;
        if fraction_done >= 1.0 {
            return Some(0.0);
        }
        let total_bytes_est =
            self.output_bytes_written as f64 / fraction_done;
        let remaining_bytes = total_bytes_est - self.output_bytes_written as f64;
        Some((remaining_bytes / self.throughput_ema_bps).max(0.0))
    }

    /// Build a snapshot suitable for emitting as a `ProgressSnapshot` event.
    pub fn snapshot(
        &self,
        current_entry: Option<String>,
        dest_free_bytes: Option<u64>,
        source_free_bytes: Option<u64>,
    ) -> ProgressSnapshot {
        ProgressSnapshot {
            phase: self.phase.clone(),
            current_entry,
            entries_done: self.entries_done,
            entries_total: self.entries_total,
            compressed_bytes_read: self.compressed_bytes_read,
            output_bytes_written: self.output_bytes_written,
            throughput_bps: self.throughput_ema_bps,
            elapsed_secs: self.started_at.elapsed().as_secs_f64(),
            eta_secs: self.eta_secs(),
            dest_free_bytes,
            source_free_bytes,
            source_bytes_reclaimed: self.source_bytes_reclaimed,
        }
    }
}
