//! Generate the small KTX2 seed inputs committed under `fuzz/seeds/`.
//!
//! ```text
//! cargo run -p draco-texture --example ktx2_make_seeds -- fuzz/seeds
//! ```
//!
//! The fixtures under `testdata/ktx2` are full 512x512 and 1024x1024 textures.
//! They are the right thing to gate a transcoder against and the wrong thing to
//! seed a fuzzer with: the target decodes each input into seven formats across
//! every mip level, so one execution over a fixture is milliseconds of real
//! transcoding. The first campaign managed four executions per second, which is
//! not a rate at which a mutation engine explores anything.
//!
//! Each seed here is a real file rebuilt around its smallest level: the same
//! header, the same codebooks, the same block data, one level of 16 or 64
//! texels. Nothing is synthesised, so what the fuzzer starts from is still
//! something an encoder wrote.
//!
//! The UASTC seeds come out with the Zstd undone. There is no Zstd encoder in
//! this tree, and it is the better shape anyway: a mutation of a compressed
//! level dies in the frame checksum, while a mutation of an uncompressed one
//! lands on the block bits, which is where the transcoder lives. The
//! compressed path stays covered by the full fixtures, which remain in the
//! corpus.

use std::path::{Path, PathBuf};

/// The KTX2 header, up to and not including the level index.
const HEADER_SIZE: usize = 80;
/// One level index entry: offset, length, uncompressed length.
const LEVEL_INDEX_SIZE: usize = 24;
/// The BasisLZ global data header, ahead of the per-image descriptors.
const GLOBAL_HEADER_SIZE: usize = 20;
/// One per-image descriptor in that global data.
const IMAGE_DESC_SIZE: usize = 20;

fn word(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}

fn long(data: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(data[at..at + 8].try_into().unwrap())
}

/// One level of a file, as the index describes it.
struct Level {
    offset: usize,
    length: usize,
    uncompressed: usize,
}

fn levels(data: &[u8]) -> Vec<Level> {
    let count = word(data, 40).max(1) as usize;
    (0..count)
        .map(|level| {
            let at = HEADER_SIZE + level * LEVEL_INDEX_SIZE;
            Level {
                offset: long(data, at) as usize,
                length: long(data, at + 8) as usize,
                uncompressed: long(data, at + 16) as usize,
            }
        })
        .collect()
}

/// Rebuild `original` as a one-level file holding level `keep`.
///
/// The header, the format description and the key/value data are carried
/// through as they are: what changes is the stated size, the level index, and
/// for a BasisLZ file the descriptor array, which is one entry per image and
/// therefore one entry too many once there is one level left.
fn shrink(original: &[u8], keep: usize, payload: Vec<u8>, scheme: u32) -> Vec<u8> {
    let index = levels(original);
    let level = &index[keep];
    let width = (word(original, 20) >> keep).max(1);
    let height = (word(original, 24) >> keep).max(1);

    // Everything after the level index - the format description, the key/value
    // data, the global data - is rewritten in place behind a one-entry index
    // rather than carried at its old offset.
    let new_index_end = HEADER_SIZE + LEVEL_INDEX_SIZE;

    let dfd_offset = word(original, 48) as usize;
    let dfd_length = word(original, 52) as usize;
    let kvd_offset = word(original, 56) as usize;
    let kvd_length = word(original, 60) as usize;
    let sgd_offset = long(original, 64) as usize;
    let sgd_length = long(original, 72) as usize;

    // The global data keeps its codebooks and one descriptor, the one this
    // level's image used.
    let global = if scheme == 1 && sgd_length != 0 {
        let sgd = &original[sgd_offset..sgd_offset + sgd_length];
        let count = index.len();
        let descs = GLOBAL_HEADER_SIZE + count * IMAGE_DESC_SIZE;
        let desc_at = GLOBAL_HEADER_SIZE + keep * IMAGE_DESC_SIZE;
        let mut out = sgd[..GLOBAL_HEADER_SIZE].to_vec();
        out.extend_from_slice(&sgd[desc_at..desc_at + IMAGE_DESC_SIZE]);
        out.extend_from_slice(&sgd[descs..]);
        out
    } else {
        Vec::new()
    };

    let mut bytes = original[..HEADER_SIZE].to_vec();
    bytes.resize(new_index_end, 0);
    bytes.extend_from_slice(&original[dfd_offset..dfd_offset + dfd_length]);
    let new_dfd = new_index_end;
    let mut new_kvd = 0;
    if kvd_length != 0 {
        new_kvd = bytes.len();
        bytes.extend_from_slice(&original[kvd_offset..kvd_offset + kvd_length]);
    }
    let mut new_sgd = 0;
    if !global.is_empty() {
        while !bytes.len().is_multiple_of(8) {
            bytes.push(0);
        }
        new_sgd = bytes.len();
        bytes.extend_from_slice(&global);
    }
    // A level starts on a multiple of the texel block size, which for every
    // format here is sixteen bytes.
    while !bytes.len().is_multiple_of(16) {
        bytes.push(0);
    }
    let new_level = bytes.len();
    bytes.extend_from_slice(&payload);

    let set_word = |bytes: &mut Vec<u8>, at: usize, value: u32| {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    };
    let set_long = |bytes: &mut Vec<u8>, at: usize, value: u64| {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    };

    set_word(&mut bytes, 20, width);
    set_word(&mut bytes, 24, height);
    set_word(&mut bytes, 40, 1);
    set_word(&mut bytes, 44, scheme);
    set_word(&mut bytes, 48, new_dfd as u32);
    set_word(&mut bytes, 52, dfd_length as u32);
    set_word(&mut bytes, 56, new_kvd as u32);
    set_word(&mut bytes, 60, kvd_length as u32);
    set_long(&mut bytes, 64, new_sgd as u64);
    set_long(&mut bytes, 72, global.len() as u64);

    set_long(&mut bytes, HEADER_SIZE, new_level as u64);
    set_long(&mut bytes, HEADER_SIZE + 8, payload.len() as u64);
    // With no supercompression the two lengths describe the same bytes and
    // must agree; only BasisLZ, which carries its own, leaves this zero.
    set_long(
        &mut bytes,
        HEADER_SIZE + 16,
        match scheme {
            0 => payload.len() as u64,
            2 => level.uncompressed as u64,
            _ => 0,
        },
    );

    bytes
}

/// The level's bytes with any supercompression undone.
fn plain_level(original: &[u8], keep: usize, scheme: u32) -> Vec<u8> {
    let level = &levels(original)[keep];
    let raw = &original[level.offset..level.offset + level.length];
    match scheme {
        2 => {
            let mut out = Vec::with_capacity(level.uncompressed);
            ruzstd::FrameDecoder::new()
                .decode_all_to_vec(raw, &mut out)
                .expect("the fixture's own Zstd frame");
            out
        }
        _ => raw.to_vec(),
    }
}

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fuzz/seeds".to_string())
        .into();
    let dir = out.join("ktx2_transcode");
    std::fs::create_dir_all(&dir).expect("creating the seed directory");

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/ktx2");

    // Level 6 of a 512 or 1024 texture is 8 or 16 texels; level 4 is 32 or 64.
    // Small enough that an execution is parsing rather than transcoding, large
    // enough to be more than one block.
    for (name, keep, label) in [
        ("2d_uastc.ktx2", 6, "uastc_8"),
        ("2d_uastc.ktx2", 3, "uastc_64"),
        ("2d_etc1s.ktx2", 6, "etc1s_8"),
        ("2d_etc1s.ktx2", 3, "etc1s_64"),
        ("sample_etc1s.ktx2", 4, "etc1s_alpha_64"),
        ("facecap.ktx2", 4, "etc1s_facecap_64"),
    ] {
        let original = std::fs::read(fixtures.join(name)).expect("reading a fixture");
        let scheme = word(&original, 44);
        let payload = plain_level(&original, keep, scheme);
        // Zstd is undone above and there is no encoder here, so the rebuilt
        // file says so; BasisLZ carries its own compression inside the level
        // and stays as it is.
        let new_scheme = if scheme == 2 { 0 } else { scheme };
        let bytes = shrink(&original, keep, payload, new_scheme);

        let path = dir.join(format!("{label}.ktx2"));
        std::fs::write(&path, &bytes).expect("writing a seed");
        println!("{} bytes -> {}", bytes.len(), path.display());
    }
}
