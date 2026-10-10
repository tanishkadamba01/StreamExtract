//! High-level archive reader that dispatches to format adapters.

use crate::errors::Result;
use crate::formats::{inspect_archive, InspectionResult};
use crate::models::extraction_options::ExtractionOptions;
use std::path::Path;

/// Opens an archive, detects its format, and returns an inspection result.
pub fn open_and_inspect(
    archive_path: &Path,
    options: &ExtractionOptions,
) -> Result<InspectionResult> {
    inspect_archive(archive_path, options)
}
