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
//! static ALLOC: draco_cpp_test_bridge::counting::Counting<std::alloc::System> =
//!     draco_cpp_test_bridge::counting::Counting(std::alloc::System);
//! ```
//!
//! The counters cost one relaxed atomic per allocation, which on an encode
//! making tens of allocations is far below the run-to-run spread. Capturing
//! backtraces is not -- keep `SAMPLING` off while timing anything.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};

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
/// How many allocations of each size were made while `SAMPLING` was on.
///
/// The first instrument to reach for when a count scales with the mesh:
/// thousands of allocations of one size name the buffer immediately, where a
/// backtrace budget gets spent on whatever the encode allocated first.
pub static SIZES: std::sync::Mutex<std::collections::BTreeMap<usize, u64>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// What counts as a large allocation -- the size above which one buffer is
/// worth naming rather than lumping into a total.
pub const LARGE_THRESHOLD: usize = 64 * 1024;

/// The size at or above which `SAMPLING` captures a backtrace. Defaults to
/// [`LARGE_THRESHOLD`], because a big buffer is usually the interesting one --
/// but a count that scales with the mesh points at a small allocation made
/// per element instead, and that one is invisible until this comes down.
pub static SAMPLE_MIN: AtomicUsize = AtomicUsize::new(LARGE_THRESHOLD);

/// The size above which `SAMPLING` stops capturing. With [`SAMPLE_MIN`] this
/// narrows the backtraces to one size, which is what turns a size histogram's
/// answer ("19,460 allocations of 16 bytes") into a call site.
pub static SAMPLE_MAX: AtomicUsize = AtomicUsize::new(usize::MAX);

/// How many backtraces to keep. A per-element allocation would otherwise
/// capture one per element, which is neither readable nor affordable.
pub static SAMPLE_LIMIT: AtomicUsize = AtomicUsize::new(64);

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
    if let Ok(mut sizes) = SIZES.lock() {
        sizes.clear();
    }
}

/// The size histogram, most frequent size first.
pub fn sizes_by_count() -> Vec<(usize, u64)> {
    let sizes = SIZES.lock().expect("sizes");
    let mut out: Vec<(usize, u64)> = sizes.iter().map(|(k, v)| (*k, *v)).collect();
    out.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    out
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
    if !SAMPLING.load(Relaxed) || size < SAMPLE_MIN.load(Relaxed) || size > SAMPLE_MAX.load(Relaxed)
    {
        return;
    }
    // Capturing a backtrace allocates, so a re-entering capture would recurse
    // until the stack runs out.
    IN_ALLOC.with(|flag| {
        if flag.get() {
            return;
        }
        flag.set(true);
        if let Ok(mut sizes) = SIZES.lock() {
            *sizes.entry(size).or_insert(0) += 1;
        }
        if let Ok(mut samples) = SAMPLES.lock() {
            // A per-element allocation would otherwise capture one backtrace
            // per element, which is neither readable nor affordable.
            if samples.len() < SAMPLE_LIMIT.load(Relaxed) {
                let trace = std::backtrace::Backtrace::force_capture().to_string();
                samples.push(format!("size={size}\n{trace}"));
            }
        }
        flag.set(false);
    });
}

/// Any allocator, counted.
///
/// Generic over what it wraps so the example decides: `Counting(System)` for
/// the platform allocator, `Counting(MiMalloc)` to ask whether a gap is the
/// allocator's. Allocator choice belongs to the consumer, as PGO does, so it
/// is made in the binary and never in `draco-core`.
pub struct Counting<A>(pub A);

unsafe impl<A: GlobalAlloc> GlobalAlloc for Counting<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNT.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        if layout.size() >= LARGE_THRESHOLD {
            LARGE.fetch_add(1, Relaxed);
        }
        maybe_sample(layout.size());
        unsafe { self.0.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { self.0.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        COUNT.fetch_add(1, Relaxed);
        BYTES.fetch_add(new_size as u64, Relaxed);
        if new_size >= LARGE_THRESHOLD {
            LARGE.fetch_add(1, Relaxed);
        }
        maybe_sample(new_size);
        unsafe { self.0.realloc(ptr, layout, new_size) }
    }
}
