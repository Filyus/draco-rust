//! End-to-end tests: load a full glTF scene, compress it to Draco, reload, and
//! decode — verifying that scene content survives and geometry round-trips.
//!
//! Uses the CC0 Khronos sample models bundled under `testdata/`.

#![cfg(feature = "test")]

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
        .unwrap()
        .into_keys()
        .collect()
}

fn accessor_bytes(scene: &draco_gltf::Import, accessor: draco_gltf::gltf::Accessor<'_>) -> Vec<u8> {
    let view = accessor.view().expect("fixture accessor is not sparse");
    let buffer = &scene.buffers[view.buffer().index()];
    let row = accessor.dimensions().multiplicity() * accessor.data_type().size();
    let stride = view.stride().unwrap_or(row);
    let base = view
        .offset()
        .checked_add(accessor.offset())
        .expect("accessor base overflow");
    let mut output = Vec::with_capacity(accessor.count() * row);
    for index in 0..accessor.count() {
        let start = base + index * stride;
        output.extend_from_slice(&buffer[start..start + row]);
    }
    output
}

fn animation_and_skin_bytes(scene: &draco_gltf::Import) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let mut animation = Vec::new();
    for animation_clip in scene.document.animations() {
        for sampler in animation_clip.samplers() {
            animation.push(accessor_bytes(scene, sampler.input()));
            animation.push(accessor_bytes(scene, sampler.output()));
        }
    }
    let skin = scene
        .document
        .skins()
        .filter_map(|skin| skin.inverse_bind_matrices())
        .map(|accessor| accessor_bytes(scene, accessor))
        .collect();
    (animation, skin)
}

fn custom_attribute_elements(scene: &draco_gltf::Import, semantic: &str) -> Vec<Vec<u8>> {
    let accessor = scene
        .document
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap()
        .attributes()
        .find(|(candidate, _)| candidate.to_string() == semantic)
        .unwrap()
        .1;
    let row = accessor.dimensions().multiplicity() * accessor.data_type().size();
    let bytes = accessor_bytes(scene, accessor);
    let mut elements: Vec<_> = bytes.chunks_exact(row).map(<[u8]>::to_vec).collect();
    elements.sort_unstable();
    elements
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
    let compressed = scene.compress().expect("compress").data;

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
    let compressed = scene.compress().expect("compress").data;
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

    let compressed = scene.compress().expect("compress").data;
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

/// Axis-aligned bounding box over every primitive's POSITION values.
fn position_bbox(
    document: &draco_gltf::gltf::Document,
    buffers: &[Vec<u8>],
) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for mesh in document.meshes() {
        for prim in mesh.primitives() {
            let reader = prim.reader(|b| buffers.get(b.index()).map(|d| d.as_slice()));
            if let Some(positions) = reader.read_positions() {
                for p in positions {
                    for i in 0..3 {
                        min[i] = min[i].min(p[i]);
                        max[i] = max[i].max(p[i]);
                    }
                }
            }
        }
    }
    (min, max)
}

#[test]
fn compress_preserves_position_values() {
    // Guards the gltf-rs accessor extraction in `compress`: a wrong stride or
    // byte offset would feed garbage coordinates into the encoder. Compare the
    // source POSITION bounding box against the box recovered after a full
    // compress -> reload -> decompress round trip.
    let dir = testdata().join("Lantern").join("glTF");
    let scene = draco_gltf::import(dir.join("Lantern.gltf")).expect("load");
    let (src_min, src_max) = position_bbox(&scene.document, &scene.buffers);

    let compressed = scene.compress().expect("compress").data;
    let mut reloaded = draco_gltf::import_slice(&compressed, Some(&dir)).expect("reload");
    reloaded.decompress_in_place().expect("decompress");
    let (dec_min, dec_max) = position_bbox(&reloaded.document, &reloaded.buffers);

    // Default quantization (~11 bits) perturbs each coordinate by at most about
    // diagonal/2048; a 1% per-axis tolerance is well above that yet far below the
    // gross error a mis-extracted attribute would produce.
    for i in 0..3 {
        let tol = (src_max[i] - src_min[i]).abs().max(1e-6) * 0.01;
        assert!(
            (dec_min[i] - src_min[i]).abs() <= tol && (dec_max[i] - src_max[i]).abs() <= tol,
            "axis {i}: source [{}, {}] vs decoded [{}, {}]",
            src_min[i],
            src_max[i],
            dec_min[i],
            dec_max[i]
        );
    }
}

#[test]
fn official_draco_fixtures_import_and_decode_real_geometry() {
    let box_draco = draco_gltf::import(testdata().join("Box/glTF_Binary/Box_Draco.glb"))
        .expect("load official Box_Draco");
    let box_primitives: Vec<_> = box_draco.draco_primitives().collect();
    assert_eq!(box_primitives.len(), 1);
    let box_mesh = box_draco
        .decode_primitive(&box_primitives[0].1)
        .expect("decode official Box_Draco");
    assert!(box_mesh.num_points() > 0);
    assert!(box_mesh.num_faces() > 0);

    let meta_path = testdata().join("BoxMetaDraco/glTF/BoxMetaDraco.gltf");
    let meta = draco_gltf::import(&meta_path).expect("load BoxMetaDraco");
    let meta_primitives: Vec<_> = meta.draco_primitives().collect();
    assert_eq!(meta_primitives.len(), 1);
    let map = draco_gltf::draco_attribute_map(&meta_primitives[0].1)
        .unwrap()
        .unwrap();
    assert!(map.contains_key("_FEATURE_ID_0"));
    let mesh = meta
        .decode_primitive(&meta_primitives[0].1)
        .expect("decode BoxMetaDraco");
    assert!(mesh.num_points() > 0);
    assert!(mesh.num_faces() > 0);
}

#[test]
fn box_meta_compresses_reloads_and_preserves_custom_attribute_bytes() {
    let dir = testdata().join("BoxMeta/glTF");
    let source = draco_gltf::import(dir.join("BoxMeta.gltf")).expect("load BoxMeta");
    let feature_ids = custom_attribute_elements(&source, "_FEATURE_ID_0");

    let compressed = source.compress().expect("compress BoxMeta");
    assert_eq!(compressed.report.compressed_primitives.len(), 1);
    assert!(compressed.report.preserved_primitives.is_empty());
    let mut reloaded =
        draco_gltf::import_slice(&compressed.data, Some(&dir)).expect("reload BoxMeta");
    assert_eq!(reloaded.draco_primitives().count(), 1);
    reloaded
        .decompress_in_place()
        .expect("decompress BoxMeta geometry");
    assert_eq!(
        custom_attribute_elements(&reloaded, "_FEATURE_ID_0"),
        feature_ids
    );
}

#[test]
fn simple_skin_preserves_animation_and_inverse_bind_accessor_bytes() {
    let source =
        draco_gltf::import(testdata().join("simple_skin.gltf")).expect("load simple_skin fixture");
    let before = animation_and_skin_bytes(&source);
    assert!(
        !before.0.is_empty(),
        "fixture must contain animation accessors"
    );
    assert!(
        !before.1.is_empty(),
        "fixture must contain inverse bind matrices"
    );

    let compressed = source.compress().expect("compress simple_skin");
    assert_eq!(compressed.report.compressed_primitives.len(), 1);
    let reloaded = draco_gltf::import_slice(&compressed.data, None).expect("reload simple_skin");
    assert_eq!(animation_and_skin_bytes(&reloaded), before);
}

#[test]
fn sparse_fixture_is_preserved_with_typed_reason() {
    let dir = testdata().join("KhronosSampleModels/SimpleSparseAccessor/glTF");
    let source =
        draco_gltf::import(dir.join("SimpleSparseAccessor.gltf")).expect("load sparse fixture");
    let compressed = source.compress().expect("report sparse accessor");
    assert!(compressed.report.compressed_primitives.is_empty());
    assert!(compressed.report.preserved_primitives.iter().any(|entry| {
        matches!(
            entry.reason,
            draco_gltf::PreserveReason::SparseAccessor { .. }
        )
    }));
}

#[test]
fn mixed_plain_draco_document_has_stable_repeat_compression_report() {
    let dir = testdata().join("BoxesMeta/glTF");
    let mut document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("BoxesMeta.gltf")).unwrap()).unwrap();

    // BoxesMeta deliberately shares its accessors between the two primitives.
    // Give the second primitive distinct accessor records so the first remains
    // independently compressible while the second exercises morph preservation.
    let second_attributes = document["meshes"][0]["primitives"][1]["attributes"]
        .as_object()
        .unwrap()
        .clone();
    for (semantic, accessor) in second_attributes {
        let source_index = accessor.as_u64().unwrap() as usize;
        let cloned = document["accessors"][source_index].clone();
        let cloned_index = document["accessors"].as_array().unwrap().len();
        document["accessors"].as_array_mut().unwrap().push(cloned);
        document["meshes"][0]["primitives"][1]["attributes"][semantic] =
            serde_json::json!(cloned_index);
    }
    let source_indices = document["meshes"][0]["primitives"][1]["indices"]
        .as_u64()
        .unwrap() as usize;
    let cloned_indices = document["accessors"][source_indices].clone();
    let cloned_indices_index = document["accessors"].as_array().unwrap().len();
    document["accessors"]
        .as_array_mut()
        .unwrap()
        .push(cloned_indices);
    document["meshes"][0]["primitives"][1]["indices"] = serde_json::json!(cloned_indices_index);

    let second_position = document["meshes"][0]["primitives"][1]["attributes"]["POSITION"]
        .as_u64()
        .unwrap() as usize;
    let target_accessor = document["accessors"].as_array().unwrap().len();
    let target = document["accessors"][second_position].clone();
    document["accessors"].as_array_mut().unwrap().push(target);
    document["meshes"][0]["primitives"][1]["targets"] =
        serde_json::json!([{ "POSITION": target_accessor }]);

    let source = draco_gltf::import_slice(&serde_json::to_vec(&document).unwrap(), Some(&dir))
        .expect("load mixed-source fixture");
    let first = source.compress().expect("first compression");
    assert_eq!(first.report.compressed_primitives.len(), 1);
    assert!(first
        .report
        .preserved_primitives
        .iter()
        .any(|entry| { matches!(entry.reason, draco_gltf::PreserveReason::MorphTargets) }));

    let mixed = draco_gltf::import_slice(&first.data, Some(&dir)).expect("reload mixed document");
    assert_eq!(mixed.draco_primitives().count(), 1);
    assert_eq!(
        mixed
            .document
            .meshes()
            .map(|mesh| mesh.primitives().count())
            .sum::<usize>(),
        2
    );

    let second = mixed.compress().expect("repeat compression");
    assert!(second.report.compressed_primitives.is_empty());
    assert!(second
        .report
        .preserved_primitives
        .iter()
        .any(|entry| { matches!(entry.reason, draco_gltf::PreserveReason::AlreadyDraco) }));
    assert!(second
        .report
        .preserved_primitives
        .iter()
        .any(|entry| { matches!(entry.reason, draco_gltf::PreserveReason::MorphTargets) }));
}
