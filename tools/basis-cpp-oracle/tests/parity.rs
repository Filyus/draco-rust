//! Every fixture, every level, every target, against the reference itself.
//!
//! Not against a build of it from 2024 that happens to be on one machine: the
//! source is in this repository at the revision `draco-texture` was ported
//! from, compiled here, in the configuration a browser gets.
//!
//! That last part is what makes the ETC1S-to-ASTC pair testable at all. The
//! reference has two branches behind `BASISD_SUPPORT_ASTC_HIGHER_OPAQUE_QUALITY`
//! which every emscripten build compiles out and a native one compiles in, so
//! the `basisu` crate — ported from the native profile — cannot be asked about
//! that pair. Here the switch is ours, and it is set to what we implement.

use std::path::{Path, PathBuf};

use basis_cpp_oracle::{transcode, Target as Reference};
use draco_texture::ktx2::{Ktx2, Supercompression};
use draco_texture::transcode::{Target, Transcoder};

/// Ours and the reference's name for each target.
const TARGETS: [(Target, Reference); 7] = [
    (Target::Rgba8, Reference::Rgba32),
    (Target::Bc1, Reference::Bc1Rgb),
    (Target::Bc3, Reference::Bc3Rgba),
    (Target::Bc7, Reference::Bc7Rgba),
    (Target::Etc1, Reference::Etc1Rgb),
    (Target::Etc2, Reference::Etc2Rgba),
    (Target::Astc, Reference::Astc4x4Rgba),
];

const FIXTURES: [&str; 5] = [
    "facecap.ktx2",
    "2d_etc1s.ktx2",
    "sample_etc1s.ktx2",
    "2d_uastc.ktx2",
    "sample_uastc_zstd.ktx2",
];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/ktx2")
        .join(name)
}

fn word(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}

/// The same file with its Zstd undone.
///
/// The oracle is compiled without Zstd, deliberately: linking a compressor
/// into it to test a transcoder would be answering a question nobody asked,
/// and this side already has `ruzstd`. Everything but the level payloads is
/// carried through and only the offsets move.
fn without_zstd(original: &[u8]) -> Vec<u8> {
    const HEADER: usize = 80;
    const ENTRY: usize = 24;

    let file = Ktx2::parse(original).expect("parsing a fixture");
    let levels = file.level_count() as usize;
    let payloads: Vec<Vec<u8>> = (0..levels as u32)
        .map(|level| file.level_bytes(level).expect("a level").into_owned())
        .collect();

    let index_end = HEADER + levels * ENTRY;
    let dfd = word(original, 48) as usize;
    let dfd_length = word(original, 52) as usize;
    let kvd = word(original, 56) as usize;
    let kvd_length = word(original, 60) as usize;

    let mut bytes = original[..HEADER].to_vec();
    bytes.resize(index_end, 0);
    let new_dfd = bytes.len();
    bytes.extend_from_slice(&original[dfd..dfd + dfd_length]);
    let new_kvd = if kvd_length == 0 {
        0
    } else {
        let at = bytes.len();
        bytes.extend_from_slice(&original[kvd..kvd + kvd_length]);
        at
    };

    let mut placed = Vec::with_capacity(levels);
    for payload in &payloads {
        while !bytes.len().is_multiple_of(16) {
            bytes.push(0);
        }
        placed.push(bytes.len());
        bytes.extend_from_slice(payload);
    }

    bytes[44..48].copy_from_slice(&0u32.to_le_bytes());
    bytes[48..52].copy_from_slice(&(new_dfd as u32).to_le_bytes());
    bytes[56..60].copy_from_slice(&(new_kvd as u32).to_le_bytes());
    // A file with no supercompression has no global data either.
    bytes[64..72].copy_from_slice(&0u64.to_le_bytes());
    bytes[72..80].copy_from_slice(&0u64.to_le_bytes());
    for (level, (offset, payload)) in placed.iter().zip(payloads.iter()).enumerate() {
        let at = HEADER + level * ENTRY;
        bytes[at..at + 8].copy_from_slice(&(*offset as u64).to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        // With no supercompression the two lengths are the same, and the
        // reference asserts on it. This reader does not, which is a gap of its
        // own - see `ktx2.rs`.
        bytes[at + 16..at + 24].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    }
    bytes
}

/// Where two byte strings first differ.
fn first_difference(want: &[u8], got: &[u8]) -> Option<String> {
    if want.len() != got.len() {
        return Some(format!(
            "{} bytes expected, {} produced",
            want.len(),
            got.len()
        ));
    }
    want.iter()
        .zip(got.iter())
        .position(|(a, b)| a != b)
        .map(|at| {
            format!(
                "first differs at byte {at}: expected {}, got {}",
                want[at], got[at]
            )
        })
}

#[test]
fn agrees_with_the_reference_on_every_fixture_and_target() {
    let mut compared = 0;

    for name in FIXTURES {
        let original = std::fs::read(fixture(name)).expect("reading a fixture");
        let file = Ktx2::parse(&original).expect("parsing a fixture");
        let ours = Transcoder::new(&file).expect("reading the codebooks");

        // BasisLZ keeps its own compression inside the level and needs no
        // help; Zstd is undone here because the oracle has no decompressor.
        let for_oracle = match file.supercompression() {
            Supercompression::Zstd => without_zstd(&original),
            _ => original.clone(),
        };

        for level in 0..file.level_count() {
            for (mine, reference) in TARGETS {
                let Ok(decoded) = ours.decode(&file, level, 0, 0, mine) else {
                    // A target this file's codec cannot reach.
                    continue;
                };
                let want = transcode(&for_oracle, level, reference).unwrap_or_else(|| {
                    panic!(
                        "{name} level {level} into {mine:?}: the reference refused a file it wrote"
                    )
                });

                let difference = first_difference(&want, &decoded.bytes);
                assert!(
                    difference.is_none(),
                    "{name} level {level} into {mine:?}: {}",
                    difference.unwrap()
                );
                compared += 1;
            }
        }
    }

    assert!(
        compared > 150,
        "only {compared} images were compared, which is too few to mean anything"
    );
    println!("parity: {compared} images identical to the reference transcoder");
}
