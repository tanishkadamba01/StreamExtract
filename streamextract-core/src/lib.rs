//! StreamExtract Core Library
//!
//! A streaming archive extraction engine for Windows that minimizes additional
//! disk usage. Provides bounded-memory decompression, safe output writing,
//! progress reporting, cancellation, and a framework for progressive
//! source-storage reclamation.
//!
//! # Design Principles
//!
//! - **Correctness first**: never silently destroy user data.
//! - **Bounded memory**: stream archives; do not buffer entire entries in RAM.
//! - **Explicit capabilities**: format adapters report what they support.
//! - **Safe by default**: destructive reclamation requires explicit opt-in and
//!   passes a strict capability gate.
//! - **Independent of UI**: this crate has no knowledge of Tauri or any GUI.

pub mod engine;
pub mod errors;
pub mod formats;
pub mod models;
pub mod platform;

// Re-export the most important types at crate root for convenience.
pub use errors::{ExtractionError, Result};
pub use models::{
    archive_entry::{ArchiveEntry, EntryKind},
    extraction_event::ExtractionEvent,
    extraction_options::{ExtractionOptions, OverwritePolicy},
};
pub use engine::streaming::run_extraction;
pub use formats::inspect_archive;
