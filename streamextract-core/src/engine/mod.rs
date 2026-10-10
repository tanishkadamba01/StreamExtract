//! Engine module: the core extraction machinery.

pub mod archive_reader;
pub mod cancellation;
pub mod integrity;
pub mod output_writer;
pub mod progress;
pub mod recovery_journal;
pub mod safety;
pub mod space_estimator;
pub mod storage_reclaimer;
pub mod streaming;
