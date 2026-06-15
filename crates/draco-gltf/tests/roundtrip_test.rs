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
