//! Malformed-input hardening for the document-preserving glTF compressor.
//!
//! `compress_gltf_bytes` parses untrusted glTF/GLB bytes, so it must never
//! panic, abort, or read external files on hostile input — it must always
//! return `Ok` or `Err`. This is a deterministic stand-in for fuzzing (which
//! runs separately in CI); every case here either reproduced a hazard or guards
//! a class of them.

#![cfg(all(feature = "gltf-reader", feature = "gltf-writer"))]

use draco_io::compress_gltf_bytes;
use serde_json::{json, Value};

const GLB_MAGIC: u32 = 0x4654_6C67;
const GLB_CHUNK_JSON: u32 = 0x4E4F_534A;
const GLB_CHUNK_BIN: u32 = 0x004E_4942;

fn build_glb(json: &Value, bin: &[u8]) -> Vec<u8> {
    let mut j = serde_json::to_vec(json).unwrap();
    while !j.len().is_multiple_of(4) {
        j.push(b' ');
    }
    let mut b = bin.to_vec();
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    let total = 12 + 8 + j.len() + 8 + b.len();
    let mut out = Vec::new();
    out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(&GLB_CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&j);
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(&GLB_CHUNK_BIN.to_le_bytes());
    out.extend_from_slice(&b);
    out
}

/// A small valid Draco-compressible GLB (triangle + material).
fn valid_glb() -> Vec<u8> {
    let mut bin = Vec::new();
    for f in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bin.extend_from_slice(&f.to_le_bytes());
    }
    for i in [0u16, 1, 2] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let json = json!({
        "asset": { "version": "2.0" },
        "meshes": [ { "primitives": [ {
            "attributes": { "POSITION": 0 }, "indices": 1, "mode": 4, "material": 0
        } ] } ],
        "materials": [ { "name": "M" } ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
              "min": [0.0,0.0,0.0], "max": [1.0,1.0,0.0] },
            { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }
        ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
            { "buffer": 0, "byteOffset": 36, "byteLength": 6 }
        ],
        "buffers": [ { "byteLength": 42 } ]
    });
    build_glb(&json, &bin)
}

/// Calling the compressor must never panic; the result is irrelevant.
fn must_not_panic(input: &[u8]) {
    let _ = compress_gltf_bytes(input, None);
}

#[test]
fn malformed_raw_inputs_do_not_panic() {
    let cases: Vec<Vec<u8>> = vec![
        vec![],
        b"{}".to_vec(),
        b"[]".to_vec(),
        b"null".to_vec(),
        b"not json at all".to_vec(),
        b"glTF".to_vec(),
        vec![0x67, 0x6c, 0x54, 0x46], // GLB magic only
        {
            // GLB header claiming a huge total length.
            let mut v = GLB_MAGIC.to_le_bytes().to_vec();
            v.extend_from_slice(&2u32.to_le_bytes());
            v.extend_from_slice(&u32::MAX.to_le_bytes());
            v
        },
        {
            // GLB with a chunk length past the end.
            let mut v = GLB_MAGIC.to_le_bytes().to_vec();
            v.extend_from_slice(&2u32.to_le_bytes());
            v.extend_from_slice(&20u32.to_le_bytes());
            v.extend_from_slice(&u32::MAX.to_le_bytes()); // chunk len
            v.extend_from_slice(&GLB_CHUNK_JSON.to_le_bytes());
            v
        },
    ];
    for c in &cases {
        must_not_panic(c);
    }
}

#[test]
fn malformed_documents_do_not_panic() {
    let bad_docs = vec![
        // Out-of-range indices everywhere.
        json!({ "asset": {"version":"2.0"}, "meshes": [ { "primitives": [ {
            "attributes": { "POSITION": 99 }, "indices": 99, "mode": 4 } ] } ] }),
        json!({ "asset": {"version":"2.0"},
            "meshes": [ { "primitives": [ { "attributes": { "POSITION": 0 }, "indices": 0, "mode": 4 } ] } ],
            "accessors": [ { "bufferView": 99, "componentType": 5126, "count": 3, "type": "VEC3" } ],
            "bufferViews": [], "buffers": [] }),
        // Accessor with an enormous count but a tiny buffer view.
        json!({ "asset": {"version":"2.0"},
            "meshes": [ { "primitives": [ { "attributes": { "POSITION": 0 }, "indices": 1, "mode": 4 } ] } ],
            "accessors": [
                { "bufferView": 0, "componentType": 5126, "count": 4000000000u64, "type": "VEC3" },
                { "bufferView": 0, "componentType": 5123, "count": 3, "type": "SCALAR" } ],
            "bufferViews": [ { "buffer": 0, "byteOffset": 0, "byteLength": 4 } ],
            "buffers": [ { "byteLength": 4, "uri": "data:application/octet-stream;base64,AAAA" } ] }),
        // Buffer view past the end of the buffer.
        json!({ "asset": {"version":"2.0"},
            "meshes": [ { "primitives": [ { "attributes": { "POSITION": 0 }, "indices": 1, "mode": 4 } ] } ],
            "accessors": [
                { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" },
                { "bufferView": 0, "componentType": 5123, "count": 3, "type": "SCALAR" } ],
            "bufferViews": [ { "buffer": 0, "byteOffset": 0, "byteLength": 999999 } ],
            "buffers": [ { "byteLength": 4, "uri": "data:application/octet-stream;base64,AAAA" } ] }),
        // Wrong types where arrays/objects are expected.
        json!({ "asset": {"version":"2.0"}, "meshes": "nope", "accessors": 5, "bufferViews": {}, "buffers": 1 }),
        // Primitive attributes is not an object.
        json!({ "asset": {"version":"2.0"}, "meshes": [ { "primitives": [ { "attributes": 7 } ] } ] }),
    ];
    for doc in &bad_docs {
        must_not_panic(serde_json::to_vec(doc).unwrap().as_slice());
    }
}

#[test]
fn byte_mutations_of_a_valid_glb_do_not_panic() {
    let base = valid_glb();
    // Flip bytes across the whole file to a few extreme values.
    for pos in (0..base.len()).step_by(3) {
        for val in [0x00u8, 0xFF, 0x7F] {
            let mut m = base.clone();
            m[pos] = val;
            must_not_panic(&m);
        }
    }
    // Truncations at every length.
    for len in 0..base.len() {
        must_not_panic(&base[..len]);
    }
}

/// Hostile `buffer.uri` must not cause a local file read when loading from
/// in-memory bytes (no base path).
#[test]
fn external_buffer_uri_is_refused_without_base_path() {
    for uri in [
        "secret.bin",
        "/etc/passwd",
        "../../etc/passwd",
        "C:/Windows/win.ini",
    ] {
        let doc = json!({ "asset": {"version":"2.0"},
            "meshes": [ { "primitives": [ { "attributes": { "POSITION": 0 }, "indices": 1, "mode": 4 } ] } ],
            "accessors": [
                { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" },
                { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" } ],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
                { "buffer": 0, "byteOffset": 36, "byteLength": 6 } ],
            "buffers": [ { "byteLength": 42, "uri": uri } ] });
        let result = compress_gltf_bytes(&serde_json::to_vec(&doc).unwrap(), None);
        assert!(
            result.is_err(),
            "external uri {uri:?} must be refused without a base path"
        );
    }
}
