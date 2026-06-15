//! Real-asset tests for the document-preserving glTF compressor, using the CC0
//! Khronos sample models already bundled under `testdata/`:
//!
//! - `Lantern` (CC0): multi-mesh, textured PBR with TANGENT, no skins/animations
//!   — exercises material/texture preservation, TANGENT compression, and a full
//!   compress -> decode round-trip.
//! - `Fox` (CC0): skinned + animated, non-indexed — exercises skinning-attribute
//!   compression (JOINTS_0/WEIGHTS_0), indices generation for a non-indexed
//!   mesh, and skin/animation preservation on a complex real asset.

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

fn draco_attributes(doc: &Value, mesh: usize, prim: usize) -> &serde_json::Map<String, Value> {
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

/// Fox is skinned + animated and non-indexed. Its geometry is compressed (a
/// fresh indices accessor is generated and the skinning attributes ride inside
/// the Draco stream), while the skin and three animations are carried through
/// untouched.
#[test]
fn fox_skinned_animated_compresses_and_preserves_scene() {
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

    // The skinned mesh is compressed with its skinning attributes inside Draco.
    let attrs = draco_attributes(&after, 0, 0);
    for sem in ["POSITION", "TEXCOORD_0", "JOINTS_0", "WEIGHTS_0"] {
        assert!(attrs.contains_key(sem), "missing {sem} in draco attributes");
    }
    // A fresh indices accessor was generated for the originally non-indexed mesh.
    assert!(
        after["meshes"][0]["primitives"][0]["indices"].is_u64(),
        "generated indices accessor expected"
    );
    let used = after["extensionsUsed"].as_array().unwrap();
    assert!(used
        .iter()
        .any(|v| v.as_str() == Some("KHR_draco_mesh_compression")));

    // The strict reader rejects skinned/animated assets, but the lenient reader
    // decodes their geometry — closing the round-trip for the compressed output.
    let reader =
        GltfReader::from_bytes_lenient(&output).expect("lenient read of compressed skinned glTF");
    let meshes = reader.decode_all_meshes().expect("decode skinned geometry");
    assert!(!meshes.is_empty() && meshes.iter().all(|m| m.num_faces() > 0));
}
