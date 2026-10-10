//! Integrity verification for extracted entries.
//!
//! Currently supports CRC-32 verification (ZIP format). The verifier is
//! designed to be incremental — data is fed chunk by chunk as it is written
//! to disk, so no second pass is required.
//!
//! # Guarantees and limitations
//!
//! - CRC-32 verification detects accidental corruption but is not a
//!   cryptographic guarantee of authenticity.
//! - An entry is only marked `VerificationStatus::Passed` if the archive
//!   provides a checksum AND it matches.
//! - An entry is marked `VerificationStatus::Unavailable` when the archive
//!   format provides no integrity data.
//! - Storage reclamation is never performed for entries marked `Failed` or
//!   `Unavailable`.

use crate::models::archive_entry::VerificationStatus;
use crc32fast::Hasher as Crc32Hasher;

/// Incrementally computes a CRC-32 as data chunks are written.
pub struct Crc32Verifier {
    hasher: Crc32Hasher,
    expected: Option<u32>,
}

impl Crc32Verifier {
    /// Creates a verifier. If `expected_crc32` is `None`, verification is
    /// not possible for this entry (the format provided no checksum).
    pub fn new(expected_crc32: Option<u32>) -> Self {
        Crc32Verifier {
            hasher: Crc32Hasher::new(),
            expected: expected_crc32,
        }
    }

    /// Feed a chunk of decompressed data to the hasher.
    pub fn update(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }

    /// Finalise the computation and return the verification status.
    pub fn finalise(self) -> (u32, VerificationStatus) {
        let computed = self.hasher.finalize();
        let status = match self.expected {
            None => VerificationStatus::Unavailable,
            Some(expected) => {
                if computed == expected {
                    VerificationStatus::Passed
                } else {
                    VerificationStatus::Failed
                }
            }
        };
        (computed, status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_passes_for_correct_data() {
        // CRC-32 of b"hello world" is 0x0D4A1185.
        let expected: u32 = crc32fast::hash(b"hello world");
        let mut v = Crc32Verifier::new(Some(expected));
        v.update(b"hello ");
        v.update(b"world");
        let (_, status) = v.finalise();
        assert_eq!(status, VerificationStatus::Passed);
    }

    #[test]
    fn crc32_fails_for_corrupted_data() {
        let expected: u32 = crc32fast::hash(b"hello world");
        let mut v = Crc32Verifier::new(Some(expected));
        v.update(b"hello CORRUPTED");
        let (_, status) = v.finalise();
        assert_eq!(status, VerificationStatus::Failed);
    }

    #[test]
    fn unavailable_when_no_checksum() {
        let mut v = Crc32Verifier::new(None);
        v.update(b"data");
        let (_, status) = v.finalise();
        assert_eq!(status, VerificationStatus::Unavailable);
    }
}
