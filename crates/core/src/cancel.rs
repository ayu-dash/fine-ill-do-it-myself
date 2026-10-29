//! Cooperative cancellation for long-running work (downloads, installs).
//!
//! A token is a cheap clonable handle around one atomic flag: the UI keeps one
//! clone to request a cancel, the worker (and `CurlFetcher`) hold another and
//! poll `is_cancelled()`. No thread, channel or lock is involved.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the worker to stop at its next checkpoint.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_token_is_not_cancelled() {
        assert!(!CancelToken::new().is_cancelled());
    }

    #[test]
    fn clones_share_one_flag() {
        let ui = CancelToken::new();
        let worker = ui.clone();

        ui.cancel();

        assert!(
            worker.is_cancelled(),
            "the worker must see the UI's cancellation request"
        );
    }

    #[test]
    fn cancelling_one_clone_does_not_cancel_another_token() {
        let first = CancelToken::new();
        let second = CancelToken::new();

        first.cancel();

        assert!(!second.is_cancelled(), "tokens must be independent");
    }
}
