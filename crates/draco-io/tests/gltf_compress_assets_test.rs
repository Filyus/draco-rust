//! Real-asset tests for the document-preserving glTF compressor, using the CC0
//! Khronos sample models already bundled under `testdata/`:
//!
//! - `Lantern` (CC0): multi-mesh, textured PBR with TANGENT, no skins/animations
//!   — exercises material/texture preservation, TANGENT compression, and a full
//!   compress -> decode round-trip.
//! - `Fox` (CC0): skinned + animated, non-indexed — exercises lossless
//!   preservation of a complex real asset (skin + animations) that falls
//!   outside the compressor's indexed-triangle scope.

#![cfg(all(feature = "gltf-reader", feature = "gltf-writer"))]

use std::path::{Path, PathBuf};

use draco_io::{compress_gltf_bytes_with_base_path, GltfReader};
use serde_json::Value;

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
}

fn draco_attributes<'a>(
    doc: &'a Value,
    mesh: usize,
    prim: usize,
) -> &'a serde_json::Map<String, Value> {
    doc["meshes"][mesh]["primitives"][prim]["extensions"]["KHR_draco_mesh_compression"]
        ["attributes"]
        .as_object()
        .expect("primitive must be Draco-compressed")
}

#[test]
fn lantern_compresses_preserves_materials_and_roundtrips() {
    let dir = testdata().join("Lantern").join("glTF");
    let path = dir.join("Lantern.gltf");
    let bytes = std::fs::read(&path).expect("testdata/Lantern/glTF/Lantern.gltf must exist");
    let before: Value = serde_json::from_slice(&bytes).unwrap();

    let output = compress_gltf_bytes_with_base_path(&bytes, Some(&dir), None).expect("compress");
    let after: Value = serde_json::from_slice(&output).unwrap();

    // Materials and textures/images carried through untouched.
    assert_eq!(
        after["materials"], before["materials"],
        "materials preserved"
    );
    assert_eq!(after["textures"], before["textures"], "textures preserved");
    assert_eq!(after["images"], before["images"], "images preserved");

    // Every mesh primitive is now Draco-compressed, TANGENT included.
    let mesh_count = before["meshes"].as_array().unwrap().len();
    assert!(mesh_count >= 1);
    for m in 0..mesh_count {
        let attrs = draco_attributes(&after, m, 0);
        for sem in ["POSITION", "NORMAL", "TANGENT", "TEXCOORD_0"] {
            assert!(attrs.contains_key(sem), "mesh {m}: missing {sem}");
        }
    }

    // Full round-trip: Lantern has no skins/animations, so the strict reader
    // accepts the compressed output and decodes the geometry.
    let reader = GltfReader::from_bytes_with_base_path(&output, None)
        .expect("compressed Lantern must be readable");
    let meshes = reader.decode_all_meshes().expect("decode");
    assert_eq!(meshes.len(), mesh_count);
    assert!(meshes.iter().all(|m| m.num_faces() > 0));
}

/// Fox is a non-indexed triangle soup, which is outside the compressor's
/// current scope (it compresses indexed triangle lists). The whole asset —
/// including its skin and three animations — must round-trip losslessly,
/// preserved uncompressed. (Skinned *compression* itself is covered by the
/// synthetic indexed test in `gltf_compress_test.rs`.)
#[test]
fn fox_skinned_animated_is_preserved_losslessly() {
    let dir = testdata().join("Fox").join("glTF");
    let path = dir.join("Fox.gltf");
    let bytes = std::fs::read(&path).expect("testdata/Fox/glTF/Fox.gltf must exist");
    let before: Value = serde_json::from_slice(&bytes).unwrap();

    let output = compress_gltf_bytes_with_base_path(&bytes, Some(&dir), None).expect("compress");
    let after: Value = serde_json::from_slice(&output).unwrap();

    // Skin / animation / image content carried through untouched.
    assert_eq!(
        after["animations"], before["animations"],
        "animations preserved"
    );
    assert_eq!(after["skins"], before["skins"], "skins preserved");
    assert_eq!(after["images"], before["images"], "images preserved");

    // Non-indexed primitive left uncompressed.
    let prim = &after["meshes"][0]["primitives"][0];
    assert!(
        prim.get("extensions").is_none()
            || prim["extensions"]
                .get("KHR_draco_mesh_compression")
                .is_none(),
        "non-indexed Fox must be preserved uncompressed"
    );
    assert!(after.get("extensionsRequired").is_none());
}
