#![cfg(feature = "test")]

use std::path::PathBuf;

use draco_gltf::{
    open, parse, AccessorData, CompressionOptions, DocumentAccessorSource, Import, MeshIndex,
    OutputFormat, ValidationProfile,
};

fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn accessor(import: &Import, index: usize) -> AccessorData {
    DocumentAccessorSource::new(&import.document, &import.resources)
        .read_accessor(index)
        .unwrap()
}

fn assert_same_accessor(left: &AccessorData, right: &AccessorData) {
    assert_eq!(left.count, right.count);
    assert_eq!(left.components, right.components);
    assert_eq!(left.component_type, right.component_type);
    assert_eq!(left.normalized, right.normalized);
    assert_eq!(left.bytes, right.bytes);
}

#[test]
fn khronos_box_glb_compresses_and_reloads() {
    let mut import = open(
        fixture("testdata/Box/glTF_Binary/Box.glb"),
        ValidationProfile::Gltf20,
    )
    .unwrap();
    let original_nodes = import.document.as_value()["nodes"].clone();

    let report = import
        .compress_primitive(MeshIndex(0), 0, CompressionOptions::default())
        .unwrap();
    assert!(report.encoded_bytes > 0);

    let bytes = import.to_bytes(OutputFormat::GlbV2).unwrap();
    let reloaded = parse(&bytes, ValidationProfile::Gltf20).unwrap();
    reloaded
        .document
        .validate(ValidationProfile::Gltf20)
        .unwrap();
    assert_eq!(reloaded.document.as_value()["nodes"], original_nodes);
    let primitive = reloaded.draco_primitives().next().unwrap();
    assert_eq!(
        reloaded.decode_primitive(primitive).unwrap().num_faces(),
        12
    );
}

#[test]
fn skin_and_animation_fixture_survive_draco_roundtrip() {
    let bytes = std::fs::read(fixture("testdata/simple_skin.gltf")).unwrap();
    let mut import = parse(&bytes, ValidationProfile::Gltf20).unwrap();
    let skin_accessor = accessor(&import, 4);
    let animation_input = accessor(&import, 5);
    let animation_output = accessor(&import, 6);
    let nodes = import.document.as_value()["nodes"].clone();

    import
        .compress_primitive(MeshIndex(0), 0, CompressionOptions::default())
        .unwrap();
    let bytes = import.to_bytes(OutputFormat::GlbV2).unwrap();
    let reloaded = parse(&bytes, ValidationProfile::Gltf20).unwrap();
    reloaded
        .document
        .validate(ValidationProfile::Gltf20)
        .unwrap();

    assert_eq!(reloaded.document.as_value()["nodes"], nodes);
    let skin_index = reloaded.document.as_value()["skins"][0]["inverseBindMatrices"]
        .as_u64()
        .unwrap() as usize;
    let sampler = &reloaded.document.as_value()["animations"][0]["samplers"][0];
    let animation_input_index = sampler["input"].as_u64().unwrap() as usize;
    let animation_output_index = sampler["output"].as_u64().unwrap() as usize;
    assert_same_accessor(&accessor(&reloaded, skin_index), &skin_accessor);
    assert_same_accessor(
        &accessor(&reloaded, animation_input_index),
        &animation_input,
    );
    assert_same_accessor(
        &accessor(&reloaded, animation_output_index),
        &animation_output,
    );
}
