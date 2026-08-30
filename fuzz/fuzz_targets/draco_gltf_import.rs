#![no_main]

//! Fuzz the full-scene glTF import and Draco decompression pipeline.
//!
//! External resources are unavailable in this target. Self-contained glTF and
//! GLB inputs exercise container parsing, extension validation, every Draco
//! primitive decoder, and the atomic plain-glTF materialization path.

use draco_core::decode_limits::DecodeLimits;
use draco_gltf::{import_slice_with_options, ImportOptions, ValidationProfile};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // `DecodeLimits::fuzzing()` is deliberately far tighter than the shipped
    // defaults: the glTF container does not bound the geometry a Draco stream
    // reconstructs, so a header naming a hundred million points is a legal
    // multi-gigabyte decode. Under the shipped defaults that decode is a
    // legitimate `-rss_limit_mb` trip, and real findings drown in that noise.
    let options = ImportOptions {
        profile: ValidationProfile::Gltf21Draft,
        draco_decode_limits: DecodeLimits::fuzzing(),
        ..ImportOptions::default()
    };
    let Ok(mut import) = import_slice_with_options(data, &options) else {
        return;
    };

    for primitive in import.draco_primitives() {
        let _ = import.decode_draco_primitive(primitive);
    }

    let _ = import.decompress_in_place();
});
