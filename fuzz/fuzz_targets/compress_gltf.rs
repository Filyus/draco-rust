#![no_main]

//! Fuzz the document-preserving glTF compressor on untrusted glTF/GLB bytes.
//!
//! `compress_gltf_bytes` parses arbitrary input, so it must never panic, abort,
//! or read external files. This entry point has no resolver, so external
//! resource URIs are refused rather than resolved from the filesystem.

use draco_io::compress_gltf_bytes;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = compress_gltf_bytes(data);
});
