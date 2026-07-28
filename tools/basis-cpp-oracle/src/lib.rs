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
