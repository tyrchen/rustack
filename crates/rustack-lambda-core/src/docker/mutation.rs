//! A retained receipt distinguishes definitely unissued from uncertain mutation IO.

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

#[derive(Debug, Clone)]
pub(super) struct MutationReceipt(Arc<AtomicU8>);
pub(super) struct MutationGuard(MutationReceipt);
impl MutationGuard {
    pub(super) fn new() -> Self {
        Self(MutationReceipt(Arc::new(AtomicU8::new(0))))
    }
    pub(super) fn receipt(&self) -> MutationReceipt {
        self.0.clone()
    }
    pub(super) fn issued(&mut self) {
        self.0.0.store(1, Ordering::Release);
    }
}
impl Drop for MutationGuard {
    fn drop(&mut self) {
        self.0.0.fetch_or(2, Ordering::Release);
    }
}
impl MutationReceipt {
    pub(super) fn definitely_unissued(&self) -> bool {
        self.0.load(Ordering::Acquire) == 2
    }
}
