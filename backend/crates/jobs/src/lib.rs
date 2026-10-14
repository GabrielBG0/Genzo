//! Scheduling primitives: generation numbers for latest-wins coalescing and
//! cancellation tokens. The priority scheduler (rayon for CPU work, tokio for
//! IO) arrives in M1.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Hands out increasing generation numbers. A render tagged with an older
/// generation than [`Generations::latest`] is stale and can be dropped.
#[derive(Debug, Default)]
pub struct Generations(AtomicU64);

impl Generations {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a new generation and returns its number. The first is 1.
    pub fn next(&self) -> u64 {
        self.0.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn latest(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    pub fn is_stale(&self, generation: u64) -> bool {
        generation < self.latest()
    }
}

/// A cheap, cloneable flag that in-flight work polls to stop early.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generations_increase() {
        let g = Generations::new();
        let a = g.next();
        let b = g.next();
        assert!(b > a);
        assert!(g.is_stale(a));
        assert!(!g.is_stale(b));
    }

    #[test]
    fn cancel_is_shared() {
        let t = CancelToken::new();
        let t2 = t.clone();
        t2.cancel();
        assert!(t.is_cancelled());
    }
}
