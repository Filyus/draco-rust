//! How fast `draco-texture` undoes Zstd supercompression, against C zstd.
//!
//! Every Zstd-supercompressed KTX2 fixture in `testdata/ktx2`, every level.
//! Ours is `Ktx2::level_bytes`, exactly as the transcoder calls it; C is
//! `ZSTD_decompress` into a buffer of the level's size, allocation included on
//! both sides. Outputs are compared byte for byte before anything is timed.
//! Best of seven rounds.
//!
//! ```text
//! ZSTD_SOURCE_DIR=<a facebook/zstd checkout> \
//!   cargo run --release --manifest-path tools/zstd-bench/Cargo.toml
//! ```

use std::path::Path;
use std::time::{Duration, Instant};

use draco_texture::ktx2::{Ktx2, Supercompression};

#[cfg(c_zstd)]
mod c {
    extern "C" {
        fn ZSTD_decompress(dst: *mut u8, capacity: usize, src: *const u8, size: usize) -> usize;
        fn ZSTD_isError(code: usize) -> u32;
    }

    /// One level through C zstd, or `None` if it refuses.
    pub fn decompress(raw: &[u8], expected: usize) -> Option<Vec<u8>> {
        let mut out = vec![0u8; expected];
        // SAFETY: both pointers come from live slices and the lengths passed
        // are theirs; `ZSTD_decompress` writes at most `capacity` bytes.
        let written =
            unsafe { ZSTD_decompress(out.as_mut_ptr(), out.len(), raw.as_ptr(), raw.len()) };
        // SAFETY: a pure function of its integer argument.
        (unsafe { ZSTD_isError(written) } == 0 && written == expected).then_some(out)
    }
}

/// The compressed bytes and declared length of every level, from the index.
fn raw_levels(data: &[u8]) -> Vec<(&[u8], usize)> {
    let count = u32::from_le_bytes(data[40..44].try_into().unwrap()).max(1) as usize;
    (0..count)
        .map(|level| {
            let at = 80 + 24 * level;
            let field = |offset: usize| {
                u64::from_le_bytes(data[at + offset..at + offset + 8].try_into().unwrap()) as usize
            };
            (&data[field(0)..field(0) + field(8)], field(16))
        })
        .collect()
}

fn best_of_seven(mut run: impl FnMut()) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..7 {
        let start = Instant::now();
        for _ in 0..10 {
            run();
        }
        best = best.min(start.elapsed() / 10);
    }
    best
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/ktx2");
    let mut names: Vec<_> = std::fs::read_dir(&root)
        .expect("testdata/ktx2")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "ktx2"))
        .collect();
    names.sort();

    let (mut ours_total, mut bytes_total) = (Duration::ZERO, 0);
    // Only added to when there is a C zstd to time.
    #[cfg(c_zstd)]
    let mut c_total = Duration::ZERO;
    #[cfg(not(c_zstd))]
    let c_total = Duration::ZERO;
    println!(
        "{:<24} {:>10} {:>11} {:>11} {:>7}",
        "fixture", "bytes out", "ours", "C zstd", "ratio"
    );
    for path in names {
        let data = std::fs::read(&path).expect("a fixture");
        let file = Ktx2::parse(&data).expect("a fixture parses");
        if file.supercompression() != Supercompression::Zstd {
            continue;
        }
        let levels = raw_levels(&data);
        let bytes: usize = levels.iter().map(|level| level.1).sum();
        let name = path.file_name().unwrap().to_string_lossy();

        #[cfg(c_zstd)]
        for (level, (raw, expected)) in levels.iter().enumerate() {
            let ours = file.level_bytes(level as u32).expect("ours decompresses");
            let theirs = c::decompress(raw, *expected).expect("C decompresses");
            assert_eq!(ours.as_ref(), &theirs[..], "{name} level {level} differs");
        }

        let ours = best_of_seven(|| {
            for level in 0..levels.len() {
                std::hint::black_box(file.level_bytes(level as u32).unwrap());
            }
        });
        ours_total += ours;
        bytes_total += bytes;

        #[cfg(c_zstd)]
        {
            let theirs = best_of_seven(|| {
                for (raw, expected) in &levels {
                    std::hint::black_box(c::decompress(raw, *expected).unwrap());
                }
            });
            c_total += theirs;
            println!(
                "{name:<24} {bytes:>10} {ours:>11.1?} {theirs:>11.1?} {:>6.2}x",
                ours.as_secs_f64() / theirs.as_secs_f64()
            );
        }
        #[cfg(not(c_zstd))]
        println!(
            "{name:<24} {bytes:>10} {ours:>11.1?} {:>11} {:>7}",
            "-", "-"
        );
    }
    let rate = |time: Duration| bytes_total as f64 / time.as_secs_f64() / 1e6;
    if c_total > Duration::ZERO {
        println!(
            "{:<24} {bytes_total:>10} {ours_total:>11.1?} {c_total:>11.1?} {:>6.2}x",
            "all",
            ours_total.as_secs_f64() / c_total.as_secs_f64()
        );
        println!(
            "ours {:.0} MB/s, C zstd {:.0} MB/s",
            rate(ours_total),
            rate(c_total)
        );
    } else {
        println!("{:<24} {bytes_total:>10} {ours_total:>11.1?}", "all");
        println!("ours {:.0} MB/s", rate(ours_total));
    }
}
