#![no_main]

//! Fuzz the document-preserving glTF compressor on untrusted glTF/GLB bytes.
//!
//! Parsing and transformation must never panic, abort, or read external files.
//! This entry point has no resolver, so external resource URIs are refused
//! rather than resolved from the filesystem.

use draco_gltf::{parse, CompressionOptions, MeshIndex, ValidationProfile};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(mut import) = parse(data, ValidationProfile::Gltf21Draft) else {
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
