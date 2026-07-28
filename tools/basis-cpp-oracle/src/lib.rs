//! The reference Basis transcoder, vendored and built here, as an oracle.
//!
//! The node gates compare against Binomial's prebuilt WASM, which lives at a
//! path inside a three.js checkout. That has three consequences worth undoing:
//! the gates skip on any machine without it, which is every runner; the build
//! is dated 2024-11-29, older than the source `draco-texture` was ported from;
//! and fed a malformed file it can be left reporting success while writing
//! nothing, which the differential gate has to work around with a canary.
//!
//! This is the same reference at the revision the port was actually made from,
//! compiled from source that is in this repository, in the configuration a
//! browser runs.
//!
//! ```text
//! cargo test --manifest-path tools/basis-cpp-oracle/Cargo.toml
//! ```

unsafe extern "C" {
    fn basis_oracle_init();
    fn basis_oracle_size(data: *const u8, length: u32, level: u32, target: i32) -> u32;
    fn basis_oracle_transcode(
        data: *const u8,
        length: u32,
        level: u32,
        target: i32,
        out: *mut u8,
        out_length: u32,
    ) -> i32;
}

/// The reference's own numbering, which is also the node gates' `TARGET`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Target {
    Etc1Rgb = 0,
    Etc2Rgba = 1,
    Bc1Rgb = 2,
    Bc3Rgba = 3,
    Bc7Rgba = 6,
    Astc4x4Rgba = 10,
    Rgba32 = 13,
}

/// The KTX2 header, up to and not including the level index.
const HEADER: usize = 80;
/// One level index entry: offset, length, uncompressed length.
const ENTRY: usize = 24;

fn word(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}

fn long(data: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(data[at..at + 8].try_into().unwrap())
}

/// How many levels a KTX2 file states.
pub fn level_count(data: &[u8]) -> u32 {
    word(data, 40).max(1)
}

/// The same file with its Zstd undone, or unchanged if it carries none.
///
/// The oracle is compiled without Zstd on purpose: linking a decompressor into
/// it to test a transcoder would be answering a question nobody asked. It is
/// undone here instead, with this crate's own `ruzstd` rather than the one
/// inside the crate under test — an oracle that prepares its input with the
/// implementation it judges is not independent of it.
pub fn without_zstd(original: &[u8]) -> Vec<u8> {
    if word(original, 44) != 2 {
        return original.to_vec();
    }

    let levels = level_count(original) as usize;
    let payloads: Vec<Vec<u8>> = (0..levels)
        .map(|level| {
            let at = HEADER + level * ENTRY;
            let offset = long(original, at) as usize;
            let length = long(original, at + 8) as usize;
            let mut out = Vec::with_capacity(long(original, at + 16) as usize);
            ruzstd::FrameDecoder::new()
                .decode_all_to_vec(&original[offset..offset + length], &mut out)
                .expect("a fixture's own Zstd frame");
            out
        })
        .collect();

    let dfd = word(original, 48) as usize;
    let dfd_length = word(original, 52) as usize;
    let kvd = word(original, 56) as usize;
    let kvd_length = word(original, 60) as usize;

    let mut bytes = original[..HEADER].to_vec();
    bytes.resize(HEADER + levels * ENTRY, 0);
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
        while bytes.len() % 16 != 0 {
            bytes.push(0);
        }
        placed.push(bytes.len());
        bytes.extend_from_slice(payload);
    }

    bytes[44..48].copy_from_slice(&0u32.to_le_bytes());
    bytes[48..52].copy_from_slice(&(new_dfd as u32).to_le_bytes());
    bytes[56..60].copy_from_slice(&(new_kvd as u32).to_le_bytes());
    // No supercompression means no global data either.
    bytes[64..72].copy_from_slice(&0u64.to_le_bytes());
    bytes[72..80].copy_from_slice(&0u64.to_le_bytes());
    for (level, (offset, payload)) in placed.iter().zip(payloads.iter()).enumerate() {
        let at = HEADER + level * ENTRY;
        bytes[at..at + 8].copy_from_slice(&(*offset as u64).to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        // With no supercompression the two lengths describe the same bytes and
        // must agree - the reference asserts on it.
        bytes[at + 16..at + 24].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    }
    bytes
}

/// Whether the file holds ETC1S rather than UASTC, read from its descriptor.
pub fn is_etc1s(data: &[u8]) -> bool {
    let dfd = word(data, 48) as usize;
    // Colour model 163 is ETC1S, 166 is UASTC LDR 4x4.
    (word(data, dfd + 12) & 255) == 163
}

/// Lowercase hex SHA-256, which is what the golden manifest records.
pub fn sha256(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Prepare the reference. Idempotent, and every entry point below calls it.
fn init() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe { basis_oracle_init() });
}

/// What the reference makes of one level, or `None` if it refuses.
///
/// Zstd is not compiled in — it is undone on this side before the bytes get
/// here — so a file still carrying a Zstd level is a refusal rather than an
/// answer.
pub fn transcode(data: &[u8], level: u32, target: Target) -> Option<Vec<u8>> {
    init();
    let length = u32::try_from(data.len()).ok()?;
    let size = unsafe { basis_oracle_size(data.as_ptr(), length, level, target as i32) };
    if size == 0 {
        return None;
    }
    let mut out = vec![0u8; size as usize];
    let ok = unsafe {
        basis_oracle_transcode(
            data.as_ptr(),
            length,
            level,
            target as i32,
            out.as_mut_ptr(),
            size,
        )
    };
    (ok == 1).then_some(out)
}
