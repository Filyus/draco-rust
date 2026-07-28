//! Write down what the reference produces, so it can be checked without it.
//!
//! ```text
//! cargo run --manifest-path tools/basis-cpp-oracle/Cargo.toml --bin bake_goldens
//! ```
//!
//! The idea is taken from the `basisu` crate, which keeps a `manifest.tsv` of
//! hashes for the same purpose. Comparing against the reference needs a C++
//! compiler and three megabytes of vendored source; comparing against a hash of
//! what the reference said needs neither, so the ordinary test suite can carry
//! the byte-exactness claim rather than delegating it to a job that has to be
//! set up.
//!
//! Regenerating is a deliberate act. If a line changes, either the reference
//! changed or this repository did, and the diff says which images.

use std::fmt::Write as _;

use basis_cpp_oracle::{sha256, transcode, Target};

/// The same list the parity test walks.
const FIXTURES: [&str; 7] = [
    "facecap.ktx2",
    "2d_etc1s.ktx2",
    "sample_etc1s.ktx2",
    "2d_uastc.ktx2",
    "sample_uastc_zstd.ktx2",
    // Written by the current encoder rather than collected years ago; see
    // testdata/ktx2/README.md.
    "etc1s_alpha_v250.ktx2",
    "uastc_alpha_v250.ktx2",
];

const TARGETS: [(&str, Target); 7] = [
    ("rgba8", Target::Rgba32),
    ("bc1", Target::Bc1Rgb),
    ("bc3", Target::Bc3Rgba),
    ("bc7", Target::Bc7Rgba),
    ("etc1", Target::Etc1Rgb),
    ("etc2", Target::Etc2Rgba),
    ("astc", Target::Astc4x4Rgba),
];

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = String::from(
        "# What the reference Basis transcoder produces, one line per image.\n\
         #\n\
         # Baked by tools/basis-cpp-oracle, which vendors the reference at the\n\
         # revision draco-texture was ported from and builds it in the profile a\n\
         # browser gets. Checked by crates/draco-texture/tests/ktx2_goldens.rs,\n\
         # which needs neither that compiler nor a browser.\n\
         #\n\
         # fixture\tlevel\ttarget\tbytes\tsha256\n",
    );

    let mut count = 0;
    for name in FIXTURES {
        let original = std::fs::read(root.join("testdata/ktx2").join(name)).expect("a fixture");
        let plain = basis_cpp_oracle::without_zstd(&original);
        let levels = basis_cpp_oracle::level_count(&plain);
        // Only the pairs draco-texture claims. The reference reaches more -
        // ETC1S to BC7, for one - and recording those would put lines in the
        // manifest that nothing is expected to match, which is worse than
        // leaving them out: every line here should be a claim.
        let etc1s = basis_cpp_oracle::is_etc1s(&plain);
        for level in 0..levels {
            for (label, target) in TARGETS {
                let claimed = match label {
                    "rgba8" | "etc1" | "etc2" | "astc" => true,
                    "bc1" | "bc3" => etc1s,
                    "bc7" => !etc1s,
                    _ => false,
                };
                if !claimed {
                    continue;
                }
                let Some(bytes) = transcode(&plain, level, target) else {
                    continue;
                };
                writeln!(
                    out,
                    "{name}\t{level}\t{label}\t{}\t{}",
                    bytes.len(),
                    sha256(&bytes)
                )
                .unwrap();
                count += 1;
            }
        }
    }

    let path = root.join("testdata/ktx2/goldens.tsv");
    std::fs::write(&path, out).expect("writing the manifest");
    println!("{count} images -> {}", path.display());
}
