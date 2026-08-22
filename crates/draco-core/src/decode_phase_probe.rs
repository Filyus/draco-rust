//! Coarse wall-clock decomposition of a mesh decode, for the performance
//! harness.
//!
//! Enabled by `DECODE_PHASES=1` in the environment, read once per process.
//! Disabled -- the default -- every probe point is one predictable branch, so
//! the instrument can stay committed instead of being re-inserted by hand
//! (this is its third use; the first two were written and reverted).
//!
//! Phase accounting is hierarchical: `Setup`, `Values` and `MapFix` run
//! inside `Attributes`, so their times are subsets of it, not siblings.
//! `take()` hands the totals over and clears them, which is how the harness
//! reads per-cell numbers out of one process.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

#[derive(Clone, Copy)]
pub enum Phase {
    Connectivity = 0,
    Attributes = 1,
    Setup = 2,
    Values = 3,
    MapFix = 4,
}

pub const PHASE_NAMES: [&str; 5] = ["conn", "attrs", "setup", "values", "mapfix"];

static NANOS: [AtomicU64; 5] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("DECODE_PHASES").is_ok_and(|v| v == "1"))
}

/// Times one phase for the duration of its scope. `start` returns `None` when
/// the probe is disabled, and a dropped `None` costs nothing.
pub struct PhaseTimer(Option<(Phase, Instant)>);

impl PhaseTimer {
    pub fn start(phase: Phase) -> Self {
        if enabled() {
            Self(Some((phase, Instant::now())))
        } else {
            Self(None)
        }
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        if let Some((phase, start)) = self.0 {
            NANOS[phase as usize].fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
    }
}

/// Returns the accumulated nanoseconds per phase and clears them.
pub fn take() -> [u64; 5] {
    let mut out = [0u64; 5];
    for (slot, counter) in out.iter_mut().zip(NANOS.iter()) {
        *slot = counter.swap(0, Ordering::Relaxed);
    }
    out
}
