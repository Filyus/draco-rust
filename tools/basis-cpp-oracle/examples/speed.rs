//! How fast `draco-texture` transcodes against the vendored reference.
//!
//! Every fixture, every level, every target both sides can reach. Each side
//! does a whole call per image -- parse, codebooks, decode -- because that is
//! what the reference does on every call; one-time tables are warmed first on
//! both. Zstd is undone beforehand for both, since the reference has none.
//! Best of seven rounds.
//!
//! ```text
//! cargo run --release --manifest-path tools/basis-cpp-oracle/Cargo.toml --example speed
//! ```

use basis_cpp_oracle::{transcode, without_zstd, Target as Reference};
use draco_texture::ktx2::Ktx2;
use draco_texture::transcode::{Target, Transcoder};
use std::time::{Duration, Instant};

const TARGETS: [(Target, Reference); 11] = [
    (Target::Rgba8, Reference::Rgba32),
    (Target::Bc1, Reference::Bc1Rgb),
    (Target::Bc3, Reference::Bc3Rgba),
    (Target::Bc4, Reference::Bc4R),
    (Target::Bc5, Reference::Bc5Rg),
    (Target::Bc7, Reference::Bc7Rgba),
    (Target::Etc1, Reference::Etc1Rgb),
    (Target::Etc2, Reference::Etc2Rgba),
    (Target::EacR11, Reference::Etc2EacR11),
    (Target::EacRg11, Reference::Etc2EacRg11),
    (Target::Astc, Reference::Astc4x4Rgba),
];
const FIXTURES: [&str; 7] = [
    "facecap.ktx2",
    "2d_etc1s.ktx2",
    "sample_etc1s.ktx2",
    "2d_uastc.ktx2",
    "sample_uastc_zstd.ktx2",
    "etc1s_alpha_v250.ktx2",
    "uastc_alpha_v250.ktx2",
];
const ROUNDS: usize = 7;

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/ktx2");
    let mut per_target_ours = [Duration::MAX; 11];
    let mut per_target_ref = [Duration::MAX; 11];
    let files: Vec<Vec<u8>> = FIXTURES
        .iter()
        .map(|n| without_zstd(&std::fs::read(root.join(n)).unwrap()).unwrap())
        .collect();
    // Warm both: one-time tables on either side are not what is measured.
    for data in &files {
        let file = Ktx2::parse(data).unwrap();
        let tr = Transcoder::new(&file).unwrap();
        for level in 0..file.level_count() {
            for (mine, reference) in TARGETS {
                let _ = tr.decode(&file, level, 0, 0, mine);
                let _ = transcode(data, level, reference);
            }
        }
    }
    let mut calls = 0;
    for _ in 0..ROUNDS {
        let mut ours = [Duration::ZERO; 11];
        let mut refs = [Duration::ZERO; 11];
        calls = 0;
        for data in &files {
            let levels = Ktx2::parse(data).unwrap().level_count();
            for level in 0..levels {
                for (i, (mine, reference)) in TARGETS.iter().enumerate() {
                    // Each side does a whole call: parse, codebooks, decode.
                    let t = Instant::now();
                    let file = Ktx2::parse(data).unwrap();
                    let ok = Transcoder::new(&file)
                        .ok()
                        .and_then(|tr| tr.decode(&file, level, 0, 0, *mine).ok())
                        .is_some();
                    let a = t.elapsed();
                    if !ok {
                        continue;
                    }
                    let t = Instant::now();
                    let theirs = transcode(data, level, *reference);
                    let b = t.elapsed();
                    assert!(theirs.is_some());
                    ours[i] += a;
                    refs[i] += b;
                    calls += 1;
                }
            }
        }
        for i in 0..11 {
            per_target_ours[i] = per_target_ours[i].min(ours[i]);
            per_target_ref[i] = per_target_ref[i].min(refs[i]);
        }
    }
    println!("{calls} calls per round, best of {ROUNDS}");
    println!(
        "{:<8} {:>12} {:>12} {:>7}",
        "target", "ours", "reference", "ratio"
    );
    let (mut a, mut b) = (Duration::ZERO, Duration::ZERO);
    for (i, (mine, _)) in TARGETS.iter().enumerate() {
        a += per_target_ours[i];
        b += per_target_ref[i];
        println!(
            "{:<8} {:>12.3?} {:>12.3?} {:>7.2}",
            format!("{mine:?}"),
            per_target_ours[i],
            per_target_ref[i],
            per_target_ours[i].as_secs_f64() / per_target_ref[i].as_secs_f64()
        );
    }
    println!(
        "{:<8} {:>12.3?} {:>12.3?} {:>7.2}",
        "total",
        a,
        b,
        a.as_secs_f64() / b.as_secs_f64()
    );
}
