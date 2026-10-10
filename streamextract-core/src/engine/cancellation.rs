//! Cancellation token for stopping extraction in progress.
//!
//! The token is designed to be shared across threads/tasks via `Arc`. The
//! extraction engine polls `is_cancelled()` at safe checkpoints between
//! writing chunks and before starting new entries. It never interrupts a
//! critical journal update or leaves file handles open.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A shared cancellation flag.
///
/// Clone the `CancellationToken` to share it between the engine and the caller.
/// Call `cancel()` from the caller side; the engine polls `is_cancelled()`.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates a new, non-cancelled token.
    pub fn new() -> Self {
        CancellationToken {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Signals that the extraction should stop as soon as safely possible.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns `true` if cancellation has been requested.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Returns `Ok(())` if not cancelled, or `Err(ExtractionError::Cancelled)`
    /// if cancellation has been requested.
    pub fn check(&self) -> crate::errors::Result<()> {
        if self.is_cancelled() {
            Err(crate::errors::ExtractionError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_token_is_not_cancelled() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        assert!(token.check().is_ok());
    }

    #[test]
    fn test_cancel_makes_token_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());
        assert!(token.check().is_err());
    }

    #[test]
    fn test_clone_shares_state() {
        let token1 = CancellationToken::new();
        let token2 = token1.clone();
        token1.cancel();
        assert!(token2.is_cancelled());
    }
}
