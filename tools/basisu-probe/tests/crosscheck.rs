//! A second oracle for `draco-texture`, and a fresher one.
//!
//! The gates in `web/tests` compare against Binomial's prebuilt WASM build,
//! which three.js ships. It has two properties worth improving on: it is dated
//! 2024-11-29, older than the source this crate was ported from, and fed a
//! malformed file it can be left reporting success while writing nothing.
//!
//! `basisu` is a third implementation — pure Rust, published 2026-07-18, and
//! itself verified byte-for-byte against a vendored C++ oracle compiled in its
//! own tree. Agreeing with it as well means agreeing with the C++ transcoder at
//! two removes and two dates, through code that shares nothing with ours.
//!
//! Not in the workspace and not on any push: `basisu` is minutes of compiling.
//!
//! ```text
//! cargo test --manifest-path tools/basisu-probe/Cargo.toml
//! ```

use std::path::{Path, PathBuf};

use basisu::{DecodeFlags, TargetFormat, Transcoder as Reference};
use draco_texture::ktx2::Ktx2;
use draco_texture::transcode::{Target, Transcoder};

/// Ours and theirs for each target, and what a block costs.
const TARGETS: [(Target, TargetFormat); 6] = [
    (Target::Rgba8, TargetFormat::Rgba32),
    (Target::Bc1, TargetFormat::Bc1Rgb),
    (Target::Bc3, TargetFormat::Bc3Rgba),
    (Target::Bc7, TargetFormat::Bc7Rgba),
    (Target::Etc1, TargetFormat::Etc1Rgb),
    (Target::Etc2, TargetFormat::Etc2Rgba),
];

/// ASTC is separate, and only from UASTC.
///
/// From ETC1S the two disagree, and the disagreement is neither one's defect:
/// the reference C++ has two extra branches behind
/// `BASISD_SUPPORT_ASTC_HIGHER_OPAQUE_QUALITY`, which trade a second 64 KiB
/// table for better opaque blocks. Every emscripten build compiles them out,
/// and this crate is gated against one - three.js's - so it does not have them.
/// `basisu` is ported from the native configuration and does: it carries
/// `G_ETC1_TO_ASTC_0_255` and the CEM 4 and CEM 8 packers those branches need.
///
/// So each of us matches a different build of the same reference, and this is
/// the first independent confirmation of that: until now it was read off an
/// `#ifdef`. Implementing them here would cost about 60 KiB against a 175 KiB
/// budget for the pair with the narrowest audience - ASTC without ETC - and
/// would end byte-exactness against the browser oracle. Not worth it, but the
/// reason is a choice rather than an oversight.
const ASTC: (Target, TargetFormat) = (Target::Astc, TargetFormat::Astc4x4Rgba);

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
fn agrees_with_basisu_on_every_fixture_and_target() {
    let mut compared = 0;
    let mut skipped = 0;

    for name in FIXTURES {
        let bytes = std::fs::read(fixture(name)).expect("reading a fixture");
        let file = Ktx2::parse(&bytes).expect("parsing a fixture");
        let ours = Transcoder::new(&file).expect("reading the codebooks");
        // `sample_uastc_zstd` needs Zstd, which this build of `basisu`
        // deliberately does not have - the size measurement is generous to it.
        let Ok(theirs) = Reference::new(&bytes) else {
            skipped += 1;
            continue;
        };

        for level in 0..file.level_count() {
            let is_etc1s = matches!(file.format(), draco_texture::ktx2::Ktx2Format::Etc1s { .. });
            for (mine, reference) in TARGETS.iter().copied().chain([ASTC]) {
                if mine == Target::Astc && is_etc1s {
                    skipped += 1;
                    continue;
                }
                let got = match ours.decode(&file, level, 0, 0, mine) {
                    Ok(decoded) => decoded.bytes,
                    // A target this file's codec cannot reach.
                    Err(_) => continue,
                };
                let want = match theirs.transcode(level, reference, DecodeFlags::default()) {
                    Ok(bytes) => bytes,
                    Err(_) => continue,
                };

                let difference = first_difference(&want, &got);
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
        compared > 100,
        "only {compared} images were compared, which is too few to mean anything"
    );
    println!(
        "crosscheck: {compared} images identical to basisu, {skipped} skipped          (ETC1S to ASTC, where the two reference build profiles differ)"
    );
}
