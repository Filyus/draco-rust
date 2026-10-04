//! Independent pieces of work run side by side, with their results given back
//! in order.
//!
//! Safe `std::thread::scope` and nothing else: no pool and no dependency, as in
//! `draco-core`. The calling thread is one of the workers, every worker takes
//! the next index from one shared counter, and a thread the system refuses to
//! start is one worker fewer rather than an error. WebAssembly has no threads
//! and runs everything on the calling one.

use std::sync::atomic::{AtomicUsize, Ordering};

/// The most threads any count resolves to, the cap `draco-core` uses.
const MAX_THREADS: usize = 16;

/// Resolves a caller's thread count: `0` is as many as the machine has, a
/// count of one or less is the calling thread alone, and none goes past
/// [`MAX_THREADS`].
pub(crate) fn resolve_threads(threads: i32) -> usize {
    if cfg!(target_arch = "wasm32") {
        return 1;
    }
    let wanted = if threads == 0 {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        usize::try_from(threads).unwrap_or(1)
    };
    wanted.clamp(1, MAX_THREADS)
}

/// Runs `work` for every index below `count` on up to `threads` workers and
/// returns the results in index order.
///
/// On a failure it returns the error of the lowest failing index, which is the
/// one a loop in order would have stopped at. Every index below that one still
/// runs, and no worker starts an index past a failure it has already seen.
pub(crate) fn try_map_indexed<T, E, F>(count: usize, threads: usize, work: F) -> Result<Vec<T>, E>
where
    T: Send,
    E: Send,
    F: Fn(usize) -> Result<T, E> + Sync,
{
    let workers = threads.min(count).max(1);
    if workers == 1 {
        return (0..count).map(work).collect();
    }

    let next = AtomicUsize::new(0);
    let first_failure = AtomicUsize::new(usize::MAX);
    let run = || {
        let mut done = Vec::new();
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            if index >= count || index > first_failure.load(Ordering::Relaxed) {
                break;
            }
            let result = work(index);
            if result.is_err() {
                first_failure.fetch_min(index, Ordering::Relaxed);
            }
            done.push((index, result));
        }
        done
    };

    let mut finished = std::thread::scope(|scope| {
        let helpers: Vec<_> = (1..workers)
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(scope, run).ok())
            .collect();
        let mut finished = run();
        for helper in helpers {
            match helper.join() {
                Ok(done) => finished.extend(done),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        finished
    });

    // Every index below the lowest failure was taken before that failure was
    // recorded, so the sorted results run without a gap up to it.
    finished.sort_unstable_by_key(|(index, _)| *index);
    finished.into_iter().map(|(_, result)| result).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    /// The first few indices take long enough that the later ones finish
    /// first, so the order the results arrive in is not the order asked.
    fn slow_start(index: usize) {
        if index < 3 {
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    #[test]
    fn results_come_back_in_index_order_on_any_count() {
        let expected: Vec<usize> = (0..200).map(|index| index * 3).collect();
        for threads in [1, 2, 3, 7, 16] {
            let results: Result<Vec<usize>, ()> = try_map_indexed(200, threads, |index| {
                slow_start(index);
                Ok(index * 3)
            });
            assert_eq!(results.unwrap(), expected, "threads {threads}");
        }
    }

    #[test]
    fn the_lowest_failing_index_is_the_error_returned() {
        for threads in [1, 2, 4, 16] {
            // Index 1 fails last: the later failures are met first.
            let result: Result<Vec<usize>, usize> = try_map_indexed(500, threads, |index| {
                slow_start(index);
                if index == 1 || index % 97 == 41 {
                    Err(index)
                } else {
                    Ok(index)
                }
            });
            assert_eq!(result, Err(1), "threads {threads}");
        }
    }

    #[test]
    fn every_index_below_the_failure_runs() {
        use std::sync::Mutex;

        for threads in [2, 4, 16] {
            let seen = Mutex::new(Vec::new());
            let result: Result<Vec<usize>, usize> = try_map_indexed(300, threads, |index| {
                seen.lock().unwrap().push(index);
                if index == 150 {
                    Err(index)
                } else {
                    Ok(index)
                }
            });
            assert_eq!(result, Err(150));
            let mut seen = seen.into_inner().unwrap();
            seen.sort_unstable();
            assert_eq!(&seen[..151], &(0..=150).collect::<Vec<_>>()[..]);
        }
    }

    #[test]
    fn thread_counts_resolve_within_one_and_the_cap() {
        assert_eq!(resolve_threads(1), 1);
        assert_eq!(resolve_threads(-3), 1);
        assert_eq!(resolve_threads(64), MAX_THREADS);
        let machine = resolve_threads(0);
        assert!((1..=MAX_THREADS).contains(&machine));
    }
}
