//! Storage reclamation subsystem.
//!
//! This module is the entry point for the progressive source-storage
//! reclamation feature. In Phase 1, it only performs a capability check
//! and returns a report. No destructive operations are implemented yet.
//!
//! See `docs/storage-reclamation.md` for the full technical design.

use serde::{Deserialize, Serialize};

/// Describes whether and how storage reclamation can be performed for a
/// specific archive instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReclamationCapabilityReport {
    /// `true` if reclamation is technically feasible for this archive.
    pub eligible: bool,

    /// Human-readable explanation of the eligibility decision.
    pub reason: String,

    /// `true` if the archive uses independent (non-solid) compression,
    /// meaning earlier entries do not affect later entry decoding.
    pub independent_entries: bool,

    /// `true` if all entries provide reliable source-range information.
    pub source_ranges_known: bool,

    /// `true` if all entries provide integrity data suitable for
    /// pre-reclamation verification.
    pub integrity_verifiable: bool,

    /// `true` if the filesystem is known to support sparse file operations.
    pub filesystem_supports_sparse: bool,
}

impl ReclamationCapabilityReport {
    /// Returns a report indicating reclamation is not supported,
    /// with the given reason.
    pub fn unsupported(reason: impl Into<String>) -> Self {
        ReclamationCapabilityReport {
            eligible: false,
            reason: reason.into(),
            independent_entries: false,
            source_ranges_known: false,
            integrity_verifiable: false,
            filesystem_supports_sparse: false,
        }
    }
}
