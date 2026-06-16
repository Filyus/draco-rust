//! End-to-end tests: load a full glTF scene, compress it to Draco, reload, and
//! decode — verifying that scene content survives and geometry round-trips.
//!
//! Uses the CC0 Khronos sample models bundled under `testdata/`.

use std::path::{Path, PathBuf};

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
}

fn semantics(prim: &draco_gltf::gltf::Primitive<'_>) -> Vec<String> {
    draco_gltf::draco_attribute_map(prim)
        .unwrap()
        .into_iter()
        .map(|(s, _)| s)
        .collect()
}

#[test]
fn lantern_compress_reload_decode() {
    let dir = testdata().join("Lantern").join("glTF");
    let scene = draco_gltf::import(dir.join("Lantern.gltf")).expect("load Lantern");

    let materials = scene.document.materials().count();
    assert!(materials >= 1);
    assert_eq!(
        scene.draco_primitives().count(),
        0,
        "source asset is uncompressed"
    );

    // Compress the whole scene (delegates to draco-io).
    let compressed = draco_gltf::compress(&scene.document, &scene.buffers).expect("compress");

    // Reload the Draco result (base dir resolves the still-external textures).
    let reloaded = draco_gltf::import_slice(&compressed, Some(&dir)).expect("reload");
    assert_eq!(
        reloaded.document.materials().count(),
        materials,
        "materials preserved"
    );

    let draco: Vec<_> = reloaded.draco_primitives().collect();
    assert!(!draco.is_empty(), "geometry is now Draco-compressed");
    for (_, prim) in &draco {
        let mesh = reloaded.decode_primitive(prim).expect("decode geometry");
        assert!(mesh.num_faces() > 0);
        assert!(
            semantics(prim).iter().any(|s| s == "TANGENT"),
            "TANGENT compressed and named in the extension map"
        );
    }
}

#[test]
fn invalid_document_is_rejected_by_validation() {
    // An accessor references bufferView 99, which does not exist. Draco-aware
    // validation must still catch this (it is not a Draco-specific error).
    let doc = serde_json::json!({
        "asset": { "version": "2.0" },
        "accessors": [ { "bufferView": 99, "componentType": 5126, "count": 3, "type": "VEC3" } ],
        "bufferViews": [], "buffers": []
    });
    let bytes = serde_json::to_vec(&doc).unwrap();
    let err = draco_gltf::import_slice(&bytes, None).err();
    assert!(
        matches!(err, Some(draco_gltf::Error::Validation(_))),
        "expected a validation error, got {err:?}"
    );
}

#[test]
fn validation_does_not_panic_on_a_hostile_document() {
    // A primitive references accessor 99 with no accessors present. gltf-rs's
    // own validator would panic here (direct index); draco-gltf pre-checks
    // primitive accessor references and returns a controlled error instead,
    // never reaching the panic (so this holds even under panic=abort on wasm).
    let doc = serde_json::json!({
        "asset": { "version": "2.0" },
        "meshes": [ { "primitives": [ { "attributes": { "POSITION": 99 } } ] } ]
    });
    let bytes = serde_json::to_vec(&doc).unwrap();
    let err = draco_gltf::import_slice(&bytes, None).err();
    assert!(
        matches!(err, Some(draco_gltf::Error::Validation(_))),
        "expected a controlled validation error, got {err:?}"
    );
}

#[test]
fn decompress_in_place_makes_geometry_readable_by_gltf_rs() {
    let dir = testdata().join("Lantern").join("glTF");
    let scene = draco_gltf::import(dir.join("Lantern.gltf")).expect("load");
    let compressed = draco_gltf::compress(&scene.document, &scene.buffers).expect("compress");
    let mut reloaded = draco_gltf::import_slice(&compressed, Some(&dir)).expect("reload");

    assert!(
        reloaded.draco_primitives().count() > 0,
        "reloaded asset is Draco-compressed"
    );

    reloaded.decompress_in_place().expect("decompress");

    // No Draco left anywhere.
    assert_eq!(reloaded.draco_primitives().count(), 0, "Draco removed");
    assert!(!reloaded
        .document
        .extensions_required()
        .any(|e| e == "KHR_draco_mesh_compression"));

    // The plain gltf-rs reader now returns geometry — no Draco awareness needed.
    let buffers = &reloaded.buffers;
    let mut total_positions = 0;
    for mesh in reloaded.document.meshes() {
        for prim in mesh.primitives() {
            let reader = prim.reader(|b| buffers.get(b.index()).map(|d| d.as_slice()));
            let positions = reader.read_positions().expect("positions readable").count();
            assert!(positions > 0);
            let indices = reader
                .read_indices()
                .expect("indices readable")
                .into_u32()
                .count();
            assert_eq!(indices % 3, 0);
            assert!(indices > 0);
            total_positions += positions;
        }
    }
    assert!(total_positions > 0);
}

#[test]
fn fox_skinned_animated_roundtrip() {
    let dir = testdata().join("Fox").join("glTF");
    let scene = draco_gltf::import(dir.join("Fox.gltf")).expect("load Fox");
    assert_eq!(scene.document.skins().count(), 1);
    assert_eq!(scene.document.animations().count(), 3);

    let compressed = draco_gltf::compress(&scene.document, &scene.buffers).expect("compress");
    let reloaded = draco_gltf::import_slice(&compressed, Some(&dir)).expect("reload");

    // Scene content carried through.
    assert_eq!(reloaded.document.skins().count(), 1, "skin preserved");
    assert_eq!(
        reloaded.document.animations().count(),
        3,
        "animations preserved"
    );

    // Skinned geometry compressed, skin attributes named in the extension.
    let draco: Vec<_> = reloaded.draco_primitives().collect();
    assert!(!draco.is_empty());
    let (_, prim) = &draco[0];
    let mesh = reloaded
        .decode_primitive(prim)
        .expect("decode skinned geometry");
    assert!(mesh.num_faces() > 0);
    let sems = semantics(prim);
    assert!(sems.iter().any(|s| s == "JOINTS_0"));
    assert!(sems.iter().any(|s| s == "WEIGHTS_0"));
}
