//! A global allocator that counts, for the profiling examples.
//!
//! Allocation shape is one of the few things a sampling profiler reports
//! poorly: `memset`, `Vec::push`, `ptr::write` and `realloc` land as separate
//! leaves with no single source line behind them, and the question "which
//! buffer" is not one a profile can answer. Counting the allocations and
//! sizing them does answer it, and `SAMPLING` turns on a backtrace per large
//! allocation when the count alone is not enough.
//!
//! Dev tooling. The examples install it; nothing in `crates/` depends on it.
//! An example that wants it declares the allocator itself, because
//! `#[global_allocator]` belongs to the binary:
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOC: draco_cpp_test_bridge::counting::Counting =
//!     draco_cpp_test_bridge::counting::Counting;
//! ```
//!
//! The counters cost one relaxed atomic per allocation, which on an encode
//! making tens of allocations is far below the run-to-run spread. Capturing
//! backtraces is not -- keep `SAMPLING` off while timing anything.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

/// Allocations since the counters were last reset.
pub static COUNT: AtomicU64 = AtomicU64::new(0);
/// Bytes requested since the counters were last reset.
pub static BYTES: AtomicU64 = AtomicU64::new(0);
/// Allocations at or above [`LARGE_THRESHOLD`].
pub static LARGE: AtomicU64 = AtomicU64::new(0);
/// Whether to capture a backtrace per large allocation. Off while timing.
pub static SAMPLING: AtomicBool = AtomicBool::new(false);
/// Backtraces captured while `SAMPLING` was on.
pub static SAMPLES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// What counts as a large allocation -- the size above which one buffer is
/// worth naming rather than lumping into a total.
pub const LARGE_THRESHOLD: usize = 64 * 1024;

thread_local! {
    static IN_ALLOC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Zero the counters and drop any captured backtraces.
pub fn reset() {
    COUNT.store(0, Relaxed);
    BYTES.store(0, Relaxed);
    LARGE.store(0, Relaxed);
    if let Ok(mut samples) = SAMPLES.lock() {
        samples.clear();
    }
}

/// `(allocations, bytes, allocations at or above [`LARGE_THRESHOLD`])`.
pub fn totals() -> (u64, u64, u64) {
    (
        COUNT.load(Relaxed),
        BYTES.load(Relaxed),
        LARGE.load(Relaxed),
    )
}

fn maybe_sample(size: usize) {
    if !SAMPLING.load(Relaxed) || size < LARGE_THRESHOLD {
        return;
    }
    // Capturing a backtrace allocates, so a re-entering capture would recurse
    // until the stack runs out.
    IN_ALLOC.with(|flag| {
        if flag.get() {
            return;
        }
        flag.set(true);
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        if let Ok(mut samples) = SAMPLES.lock() {
            samples.push(format!("size={size}\n{trace}"));
        }
        flag.set(false);
    });
}

/// The system allocator, counted.
pub struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNT.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        if layout.size() >= LARGE_THRESHOLD {
            LARGE.fetch_add(1, Relaxed);
        }
        maybe_sample(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        COUNT.fetch_add(1, Relaxed);
        BYTES.fetch_add(new_size as u64, Relaxed);
        if new_size >= LARGE_THRESHOLD {
            LARGE.fetch_add(1, Relaxed);
        }
        maybe_sample(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
