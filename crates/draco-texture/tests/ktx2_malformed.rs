//! What the reader does with files nobody would write.
//!
//! The byte-exact gates compare decoded output against a reference transcoder,
//! which means they can only ask about files that are valid: a malformed one
//! has no correct output to compare against. This is the other half — every
//! input here is expected to be refused, and the only thing asserted is *how*.
//! No panic, no hang, no allocation the header merely claimed.
//!
//! Deliberately deterministic and bounded rather than random. The fuzz target
//! `ktx2_transcode` does the open-ended search; this runs on every push, so it
//! sweeps the fields an attacker actually controls at a fixed set of extreme
//! values and finishes in well under a second.

use std::path::{Path, PathBuf};

use draco_texture::ktx2::Ktx2;
use draco_texture::transcode::{Target, Transcoder};

/// Every fixture, so a defect in one codec's path cannot hide behind another.
const FIXTURES: [&str; 6] = [
    "facecap.ktx2",
    "2d_etc1s.ktx2",
    "sample_etc1s.ktx2",
    "2d_uastc.ktx2",
    "sample_uastc_zstd.ktx2",
    "2d_rgba8.ktx2",
];

/// The values worth writing into a field: zero, one, and the edges of each width.
const EXTREMES: [u32; 8] = [0, 1, 2, 0x7fff, 0x8000, 0xffff, 0x7fff_ffff, 0xffff_ffff];

/// The same for the 64-bit fields, where the interesting values are the ones
/// that make a sum wrap rather than the ones that are merely large.
///
/// Writing 32-bit extremes into half of a long never reaches this: the other
/// half stays zero, so the value is at most four billion and every sum with it
/// is honest. A mutation test found that gap - replacing the range check with
/// the wrapping form it is written to avoid passed the word sweep and failed
/// only the case built by hand.
const EXTREME_LONGS: [u64; 6] = [0, 1, u64::MAX, u64::MAX - 63, 1 << 63, (1 << 32) + 64];

/// How large an image the sweep will actually decode.
///
/// A mutated header can name a texture the reader is right to accept and this
/// test has no reason to materialise. Parsing is swept at any size.
const MAX_TEXELS: u64 = 1 << 18;

fn read(name: &str) -> Vec<u8> {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/ktx2")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("reading {name}: {error}"))
}

/// Every target either Basis codec can be asked for.
const EVERY_TARGET: [Target; 7] = [
    Target::Rgba8,
    Target::Bc1,
    Target::Bc3,
    Target::Bc7,
    Target::Etc1,
    Target::Etc2,
    Target::Astc,
];

/// The two that between them reach every shared step, for the wide sweeps.
const SOME_TARGETS: [Target; 2] = [Target::Rgba8, Target::Astc];

/// Parse, and if that succeeds decode into `targets`.
///
/// Nothing is asserted about the result. The assertion is the absence of a
/// panic, which is what a failure here looks like.
///
/// `levels` bounds how far in to go, because the wide sweeps are thousands of
/// inputs and every level of every target on each of them buys repetition
/// rather than coverage: what differs between mutants is the header, and the
/// header is read before any of it.
fn exercise(bytes: &[u8], targets: &[Target], levels: u32) {
    let Ok(file) = Ktx2::parse(bytes) else {
        return;
    };
    let texels = (file.width() as u64)
        * (file.height() as u64)
        * (file.layer_count() as u64)
        * (file.face_count() as u64);
    if texels > MAX_TEXELS {
        return;
    }
    let Ok(transcoder) = Transcoder::new(&file) else {
        return;
    };
    for level in 0..file.level_count().min(levels) {
        for target in targets {
            let _ = transcoder.decode(&file, level, 0, 0, *target);
        }
    }
}

#[test]
fn survives_every_header_field_at_every_extreme() {
    // The header is 80 bytes and the level index 24 per level, all of it
    // little-endian words and longs the file's author chose. Sweeping at word
    // granularity covers both, since a long is two words and writing either
    // half of it is a case in its own right.
    for name in FIXTURES {
        let original = read(name);
        let index_end = (80 + 16 * 24).min(original.len() & !3);
        for offset in (0..index_end).step_by(4) {
            for value in EXTREMES {
                let mut bytes = original.clone();
                bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                exercise(&bytes, &SOME_TARGETS, 1);
            }
        }
    }
}

#[test]
fn survives_every_long_field_at_every_extreme() {
    // The level index is three 64-bit values per level and the global data
    // header two more, all of them offsets or lengths that something is
    // indexed by. What matters here is not size but arithmetic: a pair whose
    // sum wraps looks small, and a reader that adds before it compares reads
    // whatever the wrapped number points at.
    for name in FIXTURES {
        let original = read(name);
        let index_end = (80 + 16 * 24).min(original.len() & !7);
        for offset in (64..index_end).step_by(8) {
            for value in EXTREME_LONGS {
                let mut bytes = original.clone();
                bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
                exercise(&bytes, &SOME_TARGETS, 1);
            }
        }
    }
}

#[test]
fn survives_a_level_whose_offset_and_length_disagree() {
    // Both halves of one level entry at once, which is the pair that has to be
    // checked together rather than each on its own.
    for name in FIXTURES {
        let original = read(name);
        for entry in [80usize, 80 + 24] {
            if entry + 16 > original.len() {
                continue;
            }
            for offset in EXTREME_LONGS {
                for length in EXTREME_LONGS {
                    let mut bytes = original.clone();
                    bytes[entry..entry + 8].copy_from_slice(&offset.to_le_bytes());
                    bytes[entry + 8..entry + 16].copy_from_slice(&length.to_le_bytes());
                    exercise(&bytes, &SOME_TARGETS, 1);
                }
            }
        }
    }
}

#[test]
fn survives_every_truncation() {
    // Truncation is the one malformation that costs nothing to produce - a
    // download that stopped is exactly this - so every prefix of every fixture
    // has to be refused rather than read past.
    for name in FIXTURES {
        let original = read(name);
        for length in 0..original.len().min(1024) {
            exercise(&original[..length], &SOME_TARGETS, 1);
        }
        // And the boundaries further in, where the level data begins.
        for length in [
            original.len() / 4,
            original.len() / 2,
            original.len() - original.len() / 8,
            original.len() - 1,
        ] {
            exercise(&original[..length], &EVERY_TARGET, u32::MAX);
        }
    }
}

#[test]
fn survives_a_corrupted_payload() {
    // The header can be entirely valid and the compressed payload nonsense.
    // Zstd frames and Basis codebooks both fail somewhere well inside the
    // decoder rather than at the door.
    for name in FIXTURES {
        let original = read(name);
        for start in [80usize, original.len() / 2, original.len() - 64] {
            for fill in [0x00u8, 0xff, 0x5a] {
                let mut bytes = original.clone();
                for byte in bytes[start.min(original.len())..].iter_mut() {
                    *byte = fill;
                }
                exercise(&bytes, &EVERY_TARGET, u32::MAX);
            }
        }
    }
}

#[test]
fn every_refusal_says_what_was_wrong() {
    // A parser that rejects everything would pass the sweeps above. What keeps
    // this honest is that a refusal has to name a field or a section, so the
    // message is something a caller could act on rather than "invalid file".
    let original = read("2d_uastc.ktx2");
    let mut refusals = 0;
    for offset in (0..80).step_by(4) {
        for value in EXTREMES {
            let mut bytes = original.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            if let Err(error) = Ktx2::parse(&bytes) {
                let message = error.to_string();
                assert!(
                    message.len() > 12 && !message.contains("invalid file"),
                    "a refusal at byte {offset} says only: {message}"
                );
                refusals += 1;
            }
        }
    }
    assert!(
        refusals > 80,
        "the sweep should have been refused far more often than {refusals} times"
    );
}
