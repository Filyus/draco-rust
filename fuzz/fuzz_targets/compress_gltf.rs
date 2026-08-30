#![no_main]

//! Fuzz the document-preserving glTF compressor on untrusted glTF/GLB bytes.
//!
//! Parsing and transformation must never panic, abort, or read external files.
//! This entry point has no resolver, so external resource URIs are refused
//! rather than resolved from the filesystem.

use draco_core::decode_limits::DecodeLimits;
use draco_gltf::{
    import_slice_with_options, CompressionOptions, ImportOptions, MeshIndex, ValidationProfile,
};
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
    for mesh in 0..import.document.meshes().len() {
        let primitive_count = import
            .document
            .mesh(MeshIndex(mesh))
            .map_or(0, |mesh| mesh.primitive_count());
        for primitive in 0..primitive_count {
            let _ = import.compress_primitive(
                MeshIndex(mesh),
                primitive,
                CompressionOptions::default(),
            );
        }
    }
});
