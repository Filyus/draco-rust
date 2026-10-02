//! How a point cloud's decode scales with threads, on streams written once:
//! `one_thread` times each at one thread, best of seven, for an A/B against
//! a build without threads; `scaling` times each on 1, 2, 4, 8 and 16 and
//! checks every count decodes the same cloud.
//!
//! ```text
//! DRACO_STREAM_DIR=/path/to/drc/files \
//!   cargo test --manifest-path crates/Cargo.toml -p draco-core --release \
//!   --test decode_threads_probe -- --ignored --nocapture
//! ```

#![cfg(feature = "point_cloud_decode")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use draco_core::{DecoderBuffer, PointCloud, PointCloudDecoder};

fn streams() -> Vec<(String, Vec<u8>)> {
    let Some(dir) = std::env::var_os("DRACO_STREAM_DIR").map(PathBuf::from) else {
        eprintln!("set DRACO_STREAM_DIR to the streams to decode");
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "drc"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(&p).unwrap())
        })
        .collect()
}

fn decode(bytes: &[u8], threads: i32) -> (f64, u64) {
    std::thread::sleep(Duration::from_millis(300));
    let mut decoder = PointCloudDecoder::new();
    decoder.set_threads(threads);
    let mut decoded = PointCloud::new();
    let started = Instant::now();
    decoder
        .decode(&mut DecoderBuffer::new(bytes), &mut decoded)
        .expect("decodes");
    let seconds = started.elapsed().as_secs_f64();
    let hash = (0..decoded.num_attributes()).fold(0xcbf2_9ce4_8422_2325u64, |h, id| {
        decoded
            .attribute(id)
            .buffer()
            .data()
            .iter()
            .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
    });
    (seconds, hash)
}

#[test]
#[ignore = "needs DRACO_STREAM_DIR: run with --release --ignored --nocapture"]
fn one_thread() {
    for (name, bytes) in streams() {
        let mut best = f64::MAX;
        let mut hash = 0;
        for _ in 0..7 {
            let (seconds, h) = decode(&bytes, 1);
            best = best.min(seconds);
            hash = h;
        }
        println!("{name:<28} {best:.4} s  {hash:016x}");
    }
}

#[test]
#[ignore = "needs DRACO_STREAM_DIR: run with --release --ignored --nocapture"]
fn scaling() {
    println!(
        "{:<28} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "stream", "1", "2", "4", "8", "16"
    );
    for (name, bytes) in streams() {
        let mut reference = None;
        let mut row = Vec::new();
        for threads in [1, 2, 4, 8, 16] {
            let mut best = f64::MAX;
            for _ in 0..5 {
                let (seconds, hash) = decode(&bytes, threads);
                best = best.min(seconds);
                match reference {
                    None => reference = Some(hash),
                    Some(r) => assert_eq!(r, hash, "{name}: {threads} threads decoded otherwise"),
                }
            }
            row.push(best);
        }
        println!(
            "{name:<28} {}",
            row.iter()
                .map(|s| format!("{s:>8.4}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}
