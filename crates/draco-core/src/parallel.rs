//! Threads for work that splits into pieces that do not depend on one another.
//!
//! Safe `std::thread::scope` and nothing else: no pool, no dependency. Every
//! helper hands out pieces by index or by chunk and gives back results in that
//! order, so what a caller computes is a function of its pieces and never of how
//! many threads ran them or which one took which. That is the property the
//! callers rely on to keep a stream byte-identical from one machine to the next.
//!
//! WebAssembly has no threads to spawn and runs every helper on the calling one.
//!
//! The calling thread is always one of the workers, and every worker takes its
//! pieces from one shared queue until it is empty. A thread the system refuses
//! to start is therefore one worker fewer, never a panic or lost work: the
//! calling thread alone still finishes it.

use std::sync::Mutex;

/// The most threads run, whatever is asked and however many the machine has.
/// Past this the pieces this crate cuts its work into are too few to keep them
/// fed.
const MAX_THREADS: usize = 16;

/// Values below which attributes are encoded or decoded one after another on the
/// calling thread rather than side by side: a few milliseconds of work, which
/// the threads would spend starting.
///
/// A fuzzing build (`--cfg fuzzing`, which cargo-fuzz and ClusterFuzzLite
/// pass) takes this, [`PIECE`] and the decoder's stream-size gate down to
/// sizes a fuzz input of a few hundred bytes passes. The gates choose how fast
/// a cloud is coded, never what is written or read, so the paths a campaign
/// then reaches are the shipped ones, threads and pieces included.
#[cfg(all(any(feature = "encoder", feature = "point_cloud_decode"), not(fuzzing)))]
pub(crate) const ATTRIBUTES_MIN_VALUES: usize = 1 << 17;
#[cfg(all(any(feature = "encoder", feature = "point_cloud_decode"), fuzzing))]
pub(crate) const ATTRIBUTES_MIN_VALUES: usize = 64;

/// Values a piece of a pass over one attribute covers on a thread: a few hundred
/// kilobytes, enough to repay handing it over and small enough that a pass
/// still cuts into more pieces than there are threads.
#[cfg(all(feature = "encoder", not(fuzzing)))]
pub(crate) const PIECE: usize = 1 << 16;
#[cfg(all(feature = "encoder", fuzzing))]
pub(crate) const PIECE: usize = 16;

/// Values below which a pass over one attribute stays on one thread rather than
/// running in pieces: as many pieces as the most threads asked for.
#[cfg(feature = "encoder")]
pub(crate) const PASS_MIN_VALUES: usize = MAX_THREADS * PIECE;

/// How many threads to run for a request of `requested`: `0` is "as many as the
/// machine has", anything else is taken as given, both up to `MAX_THREADS`, and
/// WebAssembly, which has no thread to spawn whatever is asked, is always one.
pub(crate) fn resolve(requested: i32) -> usize {
    if cfg!(target_arch = "wasm32") {
        return 1;
    }
    if requested > 0 {
        return (requested as usize).min(MAX_THREADS);
    }
    available()
}

/// Runs `worker` on `threads` threads, the calling one among them, and returns
/// once every one has. A thread the system will not start is skipped: `worker`
/// has to be one that finishes the work however few run it.
pub(crate) fn run_workers<T: Send>(threads: usize, worker: impl Fn() -> T + Sync) -> Vec<T> {
    std::thread::scope(|scope| {
        let spawned: Vec<_> = (1..threads)
            .map_while(|_| {
                std::thread::Builder::new()
                    .spawn_scoped(scope, &worker)
                    .ok()
            })
            .collect();
        let mut results = vec![worker()];
        results.extend(spawned.into_iter().map(|thread| {
            thread
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        }));
        results
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn available() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().min(MAX_THREADS))
}

#[cfg(target_arch = "wasm32")]
fn available() -> usize {
    let _ = MAX_THREADS;
    1
}

/// Runs `work(i)` for every `i` in `0..count` and returns the results in index
/// order.
#[cfg(feature = "encoder")]
pub(crate) fn map<T: Send>(
    count: usize,
    threads: usize,
    work: impl Fn(usize) -> T + Sync,
) -> Vec<T> {
    let threads = threads.min(count);
    if threads <= 1 || cfg!(target_arch = "wasm32") {
        return (0..count).map(work).collect();
    }
    let next = Mutex::new(0usize);
    let results: Mutex<Vec<Option<T>>> = Mutex::new((0..count).map(|_| None).collect());
    run_workers(threads, || loop {
        let index = {
            let mut next = next.lock().unwrap();
            let index = *next;
            *next += 1;
            index
        };
        if index >= count {
            break;
        }
        let value = work(index);
        results.lock().unwrap()[index] = Some(value);
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|value| value.expect("every index was worked"))
        .collect()
}

/// Runs `work` on every one of `pieces`, which may be of any lengths. A piece
/// is taken by whichever thread is free next, so a few long ones among many
/// short ones still spread.
#[cfg(feature = "encoder")]
pub(crate) fn for_each_piece_mut<T: Send>(
    pieces: Vec<&mut [T]>,
    threads: usize,
    work: impl Fn(&mut [T]) + Sync,
) {
    let threads = threads.min(pieces.len());
    if threads <= 1 || cfg!(target_arch = "wasm32") {
        for piece in pieces {
            work(piece);
        }
        return;
    }
    let queue = Mutex::new(pieces.into_iter());
    run_workers(threads, || loop {
        let Some(piece) = queue.lock().unwrap().next() else {
            break;
        };
        work(piece);
    });
}

/// The smallest and largest of `values`, `None` for none, folded in pieces on
/// `threads` once there are `PASS_MIN_VALUES` of them. For integers that is
/// exactly what one pass finds.
#[cfg(feature = "encoder")]
pub(crate) fn min_max(values: &[i32], threads: usize) -> Option<(i32, i32)> {
    // Both bounds in one pass over the values, not one pass each.
    let fold = |values: &[i32]| {
        values
            .iter()
            .fold((i32::MAX, i32::MIN), |(min, max), &value| {
                (min.min(value), max.max(value))
            })
    };
    if values.is_empty() {
        return None;
    }
    if threads <= 1 || values.len() < PASS_MIN_VALUES {
        return Some(fold(values));
    }
    map(values.len().div_ceil(PIECE), threads, |piece| {
        fold(&values[piece * PIECE..((piece + 1) * PIECE).min(values.len())])
    })
    .into_iter()
    .reduce(|(min_a, max_a), (min_b, max_b)| (min_a.min(min_b), max_a.max(max_b)))
}

/// Runs `work(chunk_index, chunk)` over consecutive chunks of `data`, `chunk`
/// elements each and the last one shorter. A `chunk` of zero is taken as one,
/// on any number of threads.
pub(crate) fn for_each_chunk_mut<T: Send>(
    data: &mut [T],
    chunk: usize,
    threads: usize,
    work: impl Fn(usize, &mut [T]) + Sync,
) {
    let chunk = chunk.max(1);
    let chunks = data.len().div_ceil(chunk);
    let threads = threads.min(chunks);
    if threads <= 1 || cfg!(target_arch = "wasm32") {
        for (index, piece) in data.chunks_mut(chunk).enumerate() {
            work(index, piece);
        }
        return;
    }
    let queue = Mutex::new(data.chunks_mut(chunk).enumerate());
    run_workers(threads, || loop {
        let Some((index, piece)) = queue.lock().unwrap().next() else {
            break;
        };
        work(index, piece);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "encoder")]
    #[test]
    fn results_come_back_in_index_order_whatever_the_thread_count() {
        for threads in [1, 2, 7, 16] {
            let squares = map(100, threads, |i| i * i);
            assert_eq!(squares, (0..100).map(|i| i * i).collect::<Vec<_>>());
        }
    }

    /// A chunk of zero is a chunk of one whatever the thread count: the count
    /// of chunks was taken that way, so the pieces have to be cut that way.
    #[test]
    fn a_chunk_of_zero_is_a_chunk_of_one_on_any_number_of_threads() {
        for threads in [1, 3] {
            let mut data = vec![0u32; 10];
            for_each_chunk_mut(&mut data, 0, threads, |index, piece| {
                assert_eq!(piece.len(), 1);
                piece[0] = index as u32;
            });
            assert_eq!(data, (0..10).collect::<Vec<u32>>(), "{threads} threads");
        }
    }

    #[test]
    fn every_chunk_is_worked_once_with_its_own_index() {
        for threads in [1, 3, 8] {
            let mut data = vec![0u32; 1000];
            for_each_chunk_mut(&mut data, 64, threads, |index, piece| {
                for value in piece.iter_mut() {
                    *value = index as u32 + 1;
                }
            });
            for (i, value) in data.iter().enumerate() {
                assert_eq!(
                    *value,
                    (i / 64) as u32 + 1,
                    "element {i} at {threads} threads"
                );
            }
        }
    }

    #[test]
    fn a_request_of_zero_means_the_machine_and_anything_else_is_taken_as_given() {
        assert!((1..=MAX_THREADS).contains(&resolve(0)));
        assert_eq!(resolve(1), 1);
        if !cfg!(target_arch = "wasm32") {
            assert_eq!(resolve(3), 3);
            assert_eq!(resolve(100_000), MAX_THREADS, "a request past the most");
        }
    }

    /// The calling thread is one of the workers: `threads` of them run, and
    /// one of them is this one.
    #[test]
    fn the_calling_thread_is_one_of_the_workers() {
        let caller = std::thread::current().id();
        let ran = run_workers(4, || std::thread::current().id());
        assert_eq!(ran.len(), 4);
        assert_eq!(ran[0], caller);
        assert_eq!(run_workers(1, || std::thread::current().id()), [caller]);
    }
}
