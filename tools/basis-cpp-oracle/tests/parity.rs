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

use basis_cpp_oracle::{transcode, without_zstd, Target as Reference};
use draco_texture::ktx2::Ktx2;
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
        // help; Zstd is undone because the oracle has no decompressor.
        let for_oracle = without_zstd(&original);

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
