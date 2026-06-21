//! Tests for document-preserving glTF Draco compression.
//!
//! Built around a self-contained GLB so coverage is deterministic and needs no
//! external fixtures. The central guarantee under test: compressing geometry
//! must not drop materials, textures, images, or samplers.

#![cfg(all(feature = "gltf-reader", feature = "gltf-writer"))]

use draco_io::{compress_gltf_bytes, GltfReader};
use serde_json::{json, Value};

const GLB_MAGIC: u32 = 0x4654_6C67;
const GLB_CHUNK_JSON: u32 = 0x4E4F_534A;
const GLB_CHUNK_BIN: u32 = 0x004E_4942;
const IMAGE_MARKER: &[u8] = b"PNGDATA!"; // stand-in for image bytes

fn push_f32s(buf: &mut Vec<u8>, values: &[f32]) {
    for v in values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
}

fn push_u16s(buf: &mut Vec<u8>, values: &[u16]) {
    for v in values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
}

fn build_glb(json: &Value, bin: &[u8]) -> Vec<u8> {
    let mut json_bytes = serde_json::to_vec(json).unwrap();
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let mut bin_bytes = bin.to_vec();
    while !bin_bytes.len().is_multiple_of(4) {
        bin_bytes.push(0);
    }
    let total = 12 + 8 + json_bytes.len() + 8 + bin_bytes.len();

    let mut out = Vec::new();
    out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&GLB_CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&GLB_CHUNK_BIN.to_le_bytes());
    out.extend_from_slice(&bin_bytes);
    out
}

fn read_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Split a GLB into (parsed JSON, BIN bytes).
fn split_glb(data: &[u8]) -> (Value, Vec<u8>) {
    assert_eq!(read_u32(data, 0), GLB_MAGIC, "output is not GLB");
    let total = read_u32(data, 8) as usize;
    let mut json = None;
    let mut bin = Vec::new();
    let mut pos = 12;
    while pos + 8 <= total {
        let len = read_u32(data, pos) as usize;
        let kind = read_u32(data, pos + 4);
        let start = pos + 8;
        let end = start + len;
        match kind {
            GLB_CHUNK_JSON => json = Some(serde_json::from_slice(&data[start..end]).unwrap()),
            GLB_CHUNK_BIN => bin = data[start..end].to_vec(),
            _ => {}
        }
        pos = end;
    }
    (json.expect("no JSON chunk"), bin)
}

/// A textured triangle with POSITION/NORMAL/TEXCOORD_0 + indices, a material
/// referencing a texture -> bufferView image, and a sampler.
fn textured_triangle_glb() -> Vec<u8> {
    let mut bin = Vec::new();
    // positions (offset 0, len 36)
    push_f32s(&mut bin, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    // normals (offset 36, len 36)
    push_f32s(&mut bin, &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
    // texcoords (offset 72, len 24)
    push_f32s(&mut bin, &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
    // indices (offset 96, len 6) -> pad to 100
    push_u16s(&mut bin, &[0, 1, 2]);
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let image_offset = bin.len(); // 100
    bin.extend_from_slice(IMAGE_MARKER); // len 8

    let json = json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [ { "nodes": [0] } ],
        "nodes": [ { "mesh": 0, "name": "TriangleNode" } ],
        "meshes": [ {
            "name": "Triangle",
            "primitives": [ {
                "attributes": { "POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2 },
                "indices": 3,
                "mode": 4,
                "material": 0
            } ]
        } ],
        "materials": [ {
            "name": "RedMetal",
            "pbrMetallicRoughness": {
                "baseColorFactor": [1.0, 0.2, 0.2, 1.0],
                "baseColorTexture": { "index": 0 },
                "metallicFactor": 0.8
            }
        } ],
        "textures": [ { "source": 0, "sampler": 0 } ],
        "images": [ { "bufferView": 4, "mimeType": "image/png" } ],
        "samplers": [ { "magFilter": 9729, "minFilter": 9987 } ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
              "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0] },
            { "bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3" },
            { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2" },
            { "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" }
        ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0,  "byteLength": 36 },
            { "buffer": 0, "byteOffset": 36, "byteLength": 36 },
            { "buffer": 0, "byteOffset": 72, "byteLength": 24 },
            { "buffer": 0, "byteOffset": 96, "byteLength": 6 },
            { "buffer": 0, "byteOffset": image_offset, "byteLength": IMAGE_MARKER.len() }
        ],
        "buffers": [ { "byteLength": bin.len() } ]
    });

    build_glb(&json, &bin)
}

#[test]
fn compress_preserves_materials_textures_images_samplers() {
    let input = textured_triangle_glb();
    let (input_json, _) = split_glb(&input);

    let output = compress_gltf_bytes(&input, None).expect("compression failed");
    let (doc, bin) = split_glb(&output);

    // Material / texture / sampler blocks survive byte-for-byte (structurally).
    assert_eq!(
        doc["materials"], input_json["materials"],
        "materials must be preserved"
    );
    assert_eq!(
        doc["textures"], input_json["textures"],
        "textures must be preserved"
    );
    assert_eq!(
        doc["samplers"], input_json["samplers"],
        "samplers must be preserved"
    );
    assert_eq!(
        doc["images"][0]["mimeType"], "image/png",
        "image metadata must be preserved"
    );

    // Primitive now carries the Draco extension with a full attribute map.
    let ext = &doc["meshes"][0]["primitives"][0]["extensions"]["KHR_draco_mesh_compression"];
    assert!(ext.is_object(), "primitive must gain the Draco extension");
    let attrs = ext["attributes"].as_object().unwrap();
    assert!(attrs.contains_key("POSITION"));
    assert!(attrs.contains_key("NORMAL"));
    assert!(attrs.contains_key("TEXCOORD_0"));

    // Extension declared.
    let used: Vec<&str> = doc["extensionsUsed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(used.contains(&"KHR_draco_mesh_compression"));
    let required: Vec<&str> = doc["extensionsRequired"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(required.contains(&"KHR_draco_mesh_compression"));

    // Geometry accessors lost their buffer view (data is in Draco now) but kept
    // their descriptive fields.
    let pos = &doc["accessors"][0];
    assert!(
        pos.get("bufferView").is_none(),
        "POSITION bufferView removed"
    );
    assert_eq!(pos["type"], "VEC3");
    assert!(pos.get("min").is_some(), "POSITION min preserved");

    // The image's bytes survive in the repacked binary at its new buffer view.
    let image_bv = doc["images"][0]["bufferView"].as_u64().unwrap() as usize;
    let bv = &doc["bufferViews"][image_bv];
    let off = bv["byteOffset"].as_u64().unwrap() as usize;
    let len = bv["byteLength"].as_u64().unwrap() as usize;
    assert_eq!(&bin[off..off + len], IMAGE_MARKER, "image bytes preserved");
}

#[test]
fn compressed_output_roundtrips_through_decoder() {
    let input = textured_triangle_glb();
    let output = compress_gltf_bytes(&input, None).expect("compression failed");

    let reader = GltfReader::from_glb(&output).expect("compressed GLB must be readable");
    assert!(reader.has_draco_extension());
    let meshes = reader
        .decode_all_meshes()
        .expect("decode compressed meshes");
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].num_faces(), 1, "triangle survives compression");
    assert_eq!(meshes[0].num_points(), 3);
}

#[test]
fn original_geometry_buffer_views_are_pruned() {
    // The repacked binary must not retain the original uncompressed geometry
    // buffer views; only the image and the appended Draco stream remain. (Size
    // is not asserted: Draco has fixed header overhead that exceeds the raw
    // bytes of a single trivial triangle.)
    let input = textured_triangle_glb();
    let output = compress_gltf_bytes(&input, None).unwrap();
    let (doc, _) = split_glb(&output);

    let buffer_views = doc["bufferViews"].as_array().unwrap();
    assert_eq!(
        buffer_views.len(),
        2,
        "only the image and Draco buffer views should remain"
    );

    // The Draco stream is whichever view the extension points at; every other
    // kept view is non-geometry (here, just the image marker).
    let draco_bv = doc["meshes"][0]["primitives"][0]["extensions"]["KHR_draco_mesh_compression"]
        ["bufferView"]
        .as_u64()
        .unwrap() as usize;
    let non_draco_bytes: u64 = buffer_views
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != draco_bv)
        .map(|(_, v)| v["byteLength"].as_u64().unwrap())
        .sum();
    assert_eq!(
        non_draco_bytes,
        IMAGE_MARKER.len() as u64,
        "all original geometry bytes must be pruned; only the image remains"
    );
}

/// A skinned primitive (JOINTS_0/WEIGHTS_0) is compressed: the skinning
/// attributes ride along inside the Draco stream as generic attributes, named
/// in the extension's attribute map, and the material is preserved.
#[test]
fn skinned_primitive_is_compressed_and_roundtrips() {
    let mut bin = Vec::new();
    push_f32s(&mut bin, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]); // pos 0..36
    push_u16s(&mut bin, &[0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0]); // joints (3*VEC4 u16) 36..60
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let w_off = bin.len();
    push_f32s(
        &mut bin,
        &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
    ); // weights 3*VEC4 f32
    let i_off = bin.len();
    push_u16s(&mut bin, &[0, 1, 2]);

    let json = json!({
        "asset": { "version": "2.0" },
        "meshes": [ {
            "primitives": [ {
                "attributes": { "POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2 },
                "indices": 3,
                "mode": 4,
                "material": 0
            } ]
        } ],
        "materials": [ { "name": "KeepMe" } ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
              "min": [0.0,0.0,0.0], "max": [1.0,1.0,0.0] },
            { "bufferView": 1, "componentType": 5123, "count": 3, "type": "VEC4" },
            { "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4" },
            { "bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR" }
        ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
            { "buffer": 0, "byteOffset": 36, "byteLength": 24 },
            { "buffer": 0, "byteOffset": w_off, "byteLength": 48 },
            { "buffer": 0, "byteOffset": i_off, "byteLength": 6 }
        ],
        "buffers": [ { "byteLength": bin.len() } ]
    });
    let input = build_glb(&json, &bin);

    let output = compress_gltf_bytes(&input, None).expect("skinned compression failed");
    let (doc, _) = split_glb(&output);

    // Material preserved.
    assert_eq!(doc["materials"][0]["name"], "KeepMe");

    // Skinning attributes are now carried in the Draco extension's attribute map.
    let attrs = doc["meshes"][0]["primitives"][0]["extensions"]["KHR_draco_mesh_compression"]
        ["attributes"]
        .as_object()
        .expect("skinned primitive must be Draco-compressed");
    assert!(attrs.contains_key("POSITION"));
    assert!(attrs.contains_key("JOINTS_0"));
    assert!(attrs.contains_key("WEIGHTS_0"));

    // Round-trips through the decoder with geometry intact (one triangle).
    let reader = GltfReader::from_glb(&output).expect("compressed skinned GLB must be readable");
    let meshes = reader.decode_all_meshes().expect("decode skinned mesh");
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].num_faces(), 1);
}

/// A non-indexed triangle list is compressed: a fresh indices accessor is
/// generated (Draco glTF primitives are indexed), and the result round-trips.
#[test]
fn non_indexed_primitive_is_compressed_with_generated_indices() {
    let mut bin = Vec::new();
    push_f32s(&mut bin, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]); // 3 verts, no indices

    let json = json!({
        "asset": { "version": "2.0" },
        "meshes": [ {
            "primitives": [ {
                "attributes": { "POSITION": 0 },
                "mode": 4,
                "material": 0
            } ]
        } ],
        "materials": [ { "name": "KeepMe" } ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
              "min": [0.0,0.0,0.0], "max": [1.0,1.0,0.0] }
        ],
        "bufferViews": [ { "buffer": 0, "byteOffset": 0, "byteLength": 36 } ],
        "buffers": [ { "byteLength": bin.len() } ]
    });
    let input = build_glb(&json, &bin);

    let output = compress_gltf_bytes(&input, None).expect("must not fail");
    let (doc, _) = split_glb(&output);

    assert_eq!(doc["materials"][0]["name"], "KeepMe");
    let prim = &doc["meshes"][0]["primitives"][0];
    assert!(
        prim["extensions"]["KHR_draco_mesh_compression"].is_object(),
        "non-indexed primitive must be compressed"
    );
    // A SCALAR/UNSIGNED_INT indices accessor was generated, without a bufferView.
    let indices_idx = prim["indices"]
        .as_u64()
        .expect("compressed primitive must have an indices accessor")
        as usize;
    let indices_acc = &doc["accessors"][indices_idx];
    assert_eq!(indices_acc["type"], "SCALAR");
    assert_eq!(indices_acc["componentType"], 5125);
    assert!(indices_acc.get("bufferView").is_none());

    // Round-trips through the decoder (one triangle).
    let reader = GltfReader::from_glb(&output).expect("readable");
    let meshes = reader.decode_all_meshes().expect("decode");
    assert_eq!(meshes.len(), 1);
    assert_eq!(meshes[0].num_faces(), 1);
}

/// A non-triangle primitive (POINTS) is outside the compressor's scope and must
/// be preserved uncompressed.
#[test]
fn non_triangle_primitive_is_preserved_uncompressed() {
    let mut bin = Vec::new();
    push_f32s(&mut bin, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);

    let json = json!({
        "asset": { "version": "2.0" },
        "meshes": [ {
            "primitives": [ {
                "attributes": { "POSITION": 0 },
                "mode": 0, // POINTS
                "material": 0
            } ]
        } ],
        "materials": [ { "name": "KeepMe" } ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
              "min": [0.0,0.0,0.0], "max": [1.0,1.0,0.0] }
        ],
        "bufferViews": [ { "buffer": 0, "byteOffset": 0, "byteLength": 36 } ],
        "buffers": [ { "byteLength": bin.len() } ]
    });
    let input = build_glb(&json, &bin);

    let output = compress_gltf_bytes(&input, None).expect("must not fail");
    let (doc, _) = split_glb(&output);

    assert_eq!(doc["materials"][0]["name"], "KeepMe");
    let prim = &doc["meshes"][0]["primitives"][0];
    assert!(
        prim.get("extensions").is_none()
            || prim["extensions"]
                .get("KHR_draco_mesh_compression")
                .is_none(),
        "POINTS primitive must not be compressed"
    );
    assert!(doc.get("extensionsRequired").is_none());
}

/// An asset that *requires* an extension this crate does not implement (here
/// `KHR_materials_unlit`) must still compress: the compressor preserves the
/// extension rather than rejecting the document. Regression test for the
/// document-preserving guarantee.
#[test]
fn compresses_asset_requiring_unknown_extension() {
    let mut bin = Vec::new();
    push_f32s(&mut bin, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]); // pos 0..36
    push_u16s(&mut bin, &[0, 1, 2]); // indices 36..42

    let json = json!({
        "asset": { "version": "2.0" },
        "extensionsUsed": ["KHR_materials_unlit"],
        "extensionsRequired": ["KHR_materials_unlit"],
        "meshes": [ {
            "primitives": [ {
                "attributes": { "POSITION": 0 },
                "indices": 1,
                "mode": 4,
                "material": 0
            } ]
        } ],
        "materials": [ {
            "name": "Unlit",
            "extensions": { "KHR_materials_unlit": {} }
        } ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
              "min": [0.0,0.0,0.0], "max": [1.0,1.0,0.0] },
            { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }
        ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
            { "buffer": 0, "byteOffset": 36, "byteLength": 6 }
        ],
        "buffers": [ { "byteLength": bin.len() } ]
    });
    let input = build_glb(&json, &bin);

    let output = compress_gltf_bytes(&input, None).expect("must compress despite required ext");
    let (doc, _) = split_glb(&output);

    // The unknown extension and its material are preserved.
    assert_eq!(
        doc["materials"][0]["extensions"]["KHR_materials_unlit"],
        json!({})
    );
    // Geometry compressed.
    assert!(
        doc["meshes"][0]["primitives"][0]["extensions"]["KHR_draco_mesh_compression"].is_object()
    );
    // Both extensions are now required (the pre-existing one is kept).
    let required: Vec<&str> = doc["extensionsRequired"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(required.contains(&"KHR_materials_unlit"));
    assert!(required.contains(&"KHR_draco_mesh_compression"));
}

/// A triangle whose `_FEATURE_ID` attribute uses a glTF 2.1 accessor component
/// type (`5124` = `SIGNED_INT`/i32) that Draco cannot encode. See
/// `crates/draco-gltf/GLTF_2_1.md`.
fn triangle_with_2_1_component_type_glb() -> Vec<u8> {
    let mut bin = Vec::new();
    // positions (offset 0, len 36)
    push_f32s(&mut bin, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    // _FEATURE_ID as SIGNED_INT / i32 (offset 36, len 12) — the 2.1 type
    for v in [10i32, 20, 30] {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    // indices (offset 48, len 6)
    push_u16s(&mut bin, &[0, 1, 2]);
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }

    let json = json!({
        "asset": { "version": "2.0" },
        "meshes": [ {
            "primitives": [ {
                "attributes": { "POSITION": 0, "_FEATURE_ID": 1 },
                "indices": 2,
                "mode": 4
            } ]
        } ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" },
            // glTF 2.1 SIGNED_INT (i32) — not a glTF 2.0 vertex attribute type.
            { "bufferView": 1, "componentType": 5124, "count": 3, "type": "SCALAR" },
            { "bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR" }
        ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0,  "byteLength": 36 },
            { "buffer": 0, "byteOffset": 36, "byteLength": 12 },
            { "buffer": 0, "byteOffset": 48, "byteLength": 6 }
        ],
        "buffers": [ { "byteLength": bin.len() } ]
    });

    build_glb(&json, &bin)
}

#[test]
fn compress_skips_primitive_with_gltf_2_1_component_type() {
    // A primitive carrying an attribute typed with a glTF 2.1 component type the
    // Draco encoder cannot represent must be left uncompressed and preserved
    // verbatim — never silently corrupted. (Safe behavior pending real 2.1
    // support; see crates/draco-gltf/GLTF_2_1.md.)
    let input = triangle_with_2_1_component_type_glb();
    let output = compress_gltf_bytes(&input, None).expect("compression must not fail");
    let (doc, _) = split_glb(&output);

    let prim = &doc["meshes"][0]["primitives"][0];

    // Not Draco-compressed.
    assert!(
        prim["extensions"]["KHR_draco_mesh_compression"].is_null(),
        "primitive with an unsupported component type must not be Draco-compressed"
    );

    // The 2.1-typed attribute and its accessor survive unchanged.
    let feature_id = prim["attributes"]["_FEATURE_ID"]
        .as_u64()
        .expect("_FEATURE_ID attribute preserved") as usize;
    assert_eq!(
        doc["accessors"][feature_id]["componentType"], 5124,
        "the SIGNED_INT (5124) accessor must be preserved verbatim"
    );
    // POSITION accessor is preserved too (it was never Draco-encoded).
    let position = prim["attributes"]["POSITION"]
        .as_u64()
        .expect("POSITION preserved") as usize;
    assert_eq!(doc["accessors"][position]["componentType"], 5126);

    // Nothing was compressed, so Draco is not added as a required extension.
    if let Some(required) = doc.get("extensionsRequired").and_then(|v| v.as_array()) {
        assert!(
            !required.iter().any(|v| v == "KHR_draco_mesh_compression"),
            "no Draco requirement should be added when nothing is compressed"
        );
    }
}
