//! Byte-exactness, carried by the ordinary test suite.
//!
//! What the transcoder produces has always been compared against the reference,
//! but only where the reference happens to be: a prebuilt WASM inside a
//! three.js checkout, or a C++ compiler and three megabytes of vendored source.
//! Neither is present in `cargo test`, so the claim lived somewhere else.
//!
//! `testdata/ktx2/goldens.tsv` is what the reference said, one SHA-256 per
//! image, baked by `tools/basis-cpp-oracle`. Checking a hash needs nothing, so
//! the claim lives here now and the oracles are what keep the manifest honest
//! rather than what verify each run.
//!
//! Regenerate deliberately:
//!
//! ```text
//! cargo run --manifest-path tools/basis-cpp-oracle/Cargo.toml --bin bake_goldens
//! ```
//!
//! A changed line means the reference changed or this crate did, and the line
//! says which image to look at.

use std::path::{Path, PathBuf};

use draco_texture::ktx2::Ktx2;
use draco_texture::transcode::{Target, Transcoder};
use sha2::{Digest, Sha256};

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/ktx2")
}

/// The manifest's name for each target.
fn target(label: &str) -> Option<Target> {
    Some(match label {
        "rgba8" => Target::Rgba8,
        "bc1" => Target::Bc1,
        "bc3" => Target::Bc3,
        "bc7" => Target::Bc7,
        "etc1" => Target::Etc1,
        "etc2" => Target::Etc2,
        "astc" => Target::Astc,
        _ => return None,
    })
}

fn sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn every_image_matches_what_the_reference_said() {
    let manifest = std::fs::read_to_string(testdata().join("goldens.tsv"))
        .expect("the golden manifest; bake it with tools/basis-cpp-oracle");

    let mut checked = 0;
    let mut bytes_cache: Option<(String, Vec<u8>)> = None;

    for line in manifest.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        let name = fields.next().expect("a fixture name");
        let level: u32 = fields.next().expect("a level").parse().expect("a number");
        let label = fields.next().expect("a target");
        let length: usize = fields.next().expect("a length").parse().expect("a number");
        let digest = fields.next().expect("a hash");

        let Some(want_target) = target(label) else {
            panic!("{label} is not a target this crate knows");
        };

        // The fixtures are megabytes; reading each once per line would make
        // this the slowest test in the crate for no reason.
        let bytes = match &bytes_cache {
            Some((cached, bytes)) if cached == name => bytes,
            _ => {
                let bytes = std::fs::read(testdata().join(name))
                    .unwrap_or_else(|error| panic!("reading {name}: {error}"));
                bytes_cache = Some((name.to_string(), bytes));
                &bytes_cache.as_ref().unwrap().1
            }
        };

        let file = Ktx2::parse(bytes).unwrap_or_else(|error| panic!("parsing {name}: {error}"));
        let transcoder =
            Transcoder::new(&file).unwrap_or_else(|error| panic!("{name} codebooks: {error}"));
        let decoded = transcoder
            .decode(&file, level, 0, 0, want_target)
            .unwrap_or_else(|error| panic!("{name} level {level} into {label}: {error}"));

        assert_eq!(
            decoded.bytes.len(),
            length,
            "{name} level {level} into {label}: length"
        );
        assert_eq!(
            sha256(&decoded.bytes),
            digest,
            "{name} level {level} into {label}: this is not what the reference produced"
        );
        checked += 1;
    }

    assert!(
        checked > 200,
        "only {checked} images were checked; the manifest looks truncated"
    );
}
