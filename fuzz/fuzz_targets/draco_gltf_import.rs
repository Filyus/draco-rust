#![no_main]

//! Fuzz the full-scene glTF import and Draco decompression pipeline.
//!
//! External resources are unavailable in this target. Self-contained glTF and
//! GLB inputs exercise container parsing, extension validation, every Draco
//! primitive decoder, and the atomic plain-glTF materialization path.

use draco_gltf::import_slice;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(mut import) = import_slice(data, None) else {
        return;
    };

    for primitive in import.draco_primitives() {
        let _ = import.decode_draco_primitive(primitive);
    }

    let _ = import.decompress_in_place();
});
