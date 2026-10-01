//! Threads for work that splits into pieces that do not depend on one another.
//!
//! Safe `std::thread::scope` and nothing else: no pool, no dependency. Every
//! helper hands out pieces by index or by chunk and gives back results in that
//! order, so what a caller computes is a function of its pieces and never of how
//! many threads ran them or which one took which. That is the property the
//! callers rely on to keep a stream byte-identical from one machine to the next.
//!
//! WebAssembly has no threads to spawn and runs every helper on the calling one.

use std::sync::Mutex;

/// The most threads asked of the machine, however many it has. Past this the
/// pieces this crate cuts its work into are too few to keep them fed.
const MAX_THREADS: usize = 16;

/// How many threads to run for a request of `requested`: `0` is "as many as the
/// machine has", anything else is taken as given, and WebAssembly, which has no
/// thread to spawn whatever is asked, is always one.
pub(crate) fn resolve(requested: i32) -> usize {
    if cfg!(target_arch = "wasm32") {
        return 1;
    }
    if requested > 0 {
        return requested as usize;
    }
    available()
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
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
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
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|value| value.expect("every index was worked"))
        .collect()
}

/// Runs `work(chunk_index, chunk)` over consecutive chunks of `data`, `chunk`
/// elements each and the last one shorter.
pub(crate) fn for_each_chunk_mut<T: Send>(
    data: &mut [T],
    chunk: usize,
    threads: usize,
    work: impl Fn(usize, &mut [T]) + Sync,
) {
    let chunks = data.len().div_ceil(chunk.max(1));
    let threads = threads.min(chunks);
    if threads <= 1 || cfg!(target_arch = "wasm32") {
        for (index, piece) in data.chunks_mut(chunk.max(1)).enumerate() {
            work(index, piece);
        }
        return;
    }
    let queue = Mutex::new(data.chunks_mut(chunk).enumerate());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let Some((index, piece)) = queue.lock().unwrap().next() else {
                    break;
                };
                work(index, piece);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_come_back_in_index_order_whatever_the_thread_count() {
        for threads in [1, 2, 7, 16] {
            let squares = map(100, threads, |i| i * i);
            assert_eq!(squares, (0..100).map(|i| i * i).collect::<Vec<_>>());
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
        assert!(resolve(0) >= 1);
        assert_eq!(resolve(1), 1);
        if !cfg!(target_arch = "wasm32") {
            assert_eq!(resolve(3), 3);
        }
    }
}
