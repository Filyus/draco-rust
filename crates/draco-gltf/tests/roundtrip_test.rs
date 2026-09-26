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
        reloaded
            .decode_draco_primitive(primitive)
            .unwrap()
            .num_faces(),
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

/// The host-neutral Draco entry point is the path `read_primitive` takes, so a
/// caller with its own document model gets byte-identical geometry.
#[test]
fn host_neutral_draco_decode_matches_read_primitive() {
    use draco_gltf::{
        DracoPrimitiveContract, DracoPrimitiveExtension, GeometryError, PrimitiveIndex,
        KHR_DRACO_MESH_COMPRESSION,
    };

    let import = open(
        fixture("testdata/Box/glTF_Binary/Box_Draco.glb"),
        ValidationProfile::Gltf20,
    )
    .unwrap();
    let primitive = import.document.primitive(MeshIndex(0), 0).unwrap();
    let extension = DracoPrimitiveExtension::from_json(
        primitive.extension(KHR_DRACO_MESH_COMPRESSION).unwrap(),
    )
    .unwrap();

    let view = &import.document.as_value()["bufferViews"]
        .as_array()
        .unwrap()[extension.buffer_view()];
    let buffer = &import.resources.buffers[view["buffer"].as_u64().unwrap() as usize];
    let start = view.get("byteOffset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let payload = &buffer[start..start + view["byteLength"].as_u64().unwrap() as usize];

    let mut contract = DracoPrimitiveContract::new().with_profile(ValidationProfile::Gltf20);
    for (semantic, index) in primitive.attribute_indices() {
        let accessor = import.document.accessor(index).unwrap();
        contract =
            contract.with_attribute(semantic, accessor.count().unwrap(), accessor.normalized());
    }
    let indices = import
        .document
        .accessor(primitive.indices().unwrap())
        .unwrap()
        .count()
        .unwrap();
    let decoded = extension
        .decode(payload, &contract.clone().with_indices(indices))
        .unwrap();
    let expected = import
        .read_primitive(PrimitiveIndex {
            mesh: MeshIndex(0),
            primitive: 0,
        })
        .unwrap();
    assert_eq!(decoded, expected);
    assert_eq!(extension.attributes().len(), decoded.attributes().len());

    // A declared count the stream cannot supply is refused, as it is for
    // documents this crate parsed itself.
    let error = extension
        .decode(payload, &contract.with_indices(indices + 3))
        .unwrap_err();
    assert!(
        matches!(
            error,
            draco_gltf::Error::Geometry(GeometryError::DracoAccessorCount { .. })
        ),
        "{error}"
    );
}
