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

/// The host-neutral decoder gives what `read_primitive` gives.
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

    // A count the stream cannot supply is refused.
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

/// The host-neutral encoder writes what `compress_primitive` writes.
#[test]
fn host_neutral_draco_encode_matches_compress_primitive() {
    use draco_gltf::{
        CompressionMode, DracoPrimitiveContract, DracoPrimitiveEncoding, DracoPrimitiveExtension,
        PrimitiveIndex, QuantizationBits, KHR_DRACO_MESH_COMPRESSION,
    };

    let first = PrimitiveIndex {
        mesh: MeshIndex(0),
        primitive: 0,
    };
    let options = CompressionOptions {
        mode: CompressionMode::DracoOnly,
        quantization: QuantizationBits::GLTF,
        ..CompressionOptions::default()
    };
    let source = open(
        fixture("testdata/Box/glTF_Binary/Box.glb"),
        ValidationProfile::Gltf20,
    )
    .unwrap();
    let geometry = source.read_primitive(first).unwrap();
    let encoded = DracoPrimitiveEncoding::encode(&geometry, &options).unwrap();

    let mut compressed = source.clone();
    compressed
        .compress_primitive(MeshIndex(0), 0, options)
        .unwrap();
    let primitive = compressed.document.primitive(MeshIndex(0), 0).unwrap();
    let extension = DracoPrimitiveExtension::from_json(
        primitive.extension(KHR_DRACO_MESH_COMPRESSION).unwrap(),
    )
    .unwrap();
    assert_eq!(encoded.extension(extension.buffer_view()), extension);
    assert_eq!(
        DracoPrimitiveExtension::from_json(&extension.to_json()).unwrap(),
        extension
    );

    let root = compressed.document.as_value();
    let view = &root["bufferViews"].as_array().unwrap()[extension.buffer_view()];
    let buffer = &compressed.resources.buffers[view["buffer"].as_u64().unwrap() as usize];
    let start = view.get("byteOffset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let payload = &buffer[start..start + view["byteLength"].as_u64().unwrap() as usize];
    assert_eq!(encoded.bytes(), payload);

    // Same accessor declarations.
    let accessors = root["accessors"].as_array().unwrap();
    for declared in encoded.accessors() {
        let (_, index) = primitive
            .attribute_indices()
            .find(|(semantic, _)| *semantic == declared.semantic())
            .unwrap();
        let accessor = &accessors[index.0];
        assert_eq!(accessor["count"].as_u64(), Some(declared.count() as u64));
        assert_eq!(
            accessor["componentType"].as_u64(),
            Some(u64::from(declared.component_type().to_gltf()))
        );
        assert_eq!(accessor["type"].as_str(), Some(declared.accessor_type()));
        assert!(accessor.get("bufferView").is_none());
        let bounds = accessor.get("min").map(|min| {
            let values = |v: &draco_gltf::JsonValue| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_f64().unwrap())
                    .collect::<Vec<_>>()
            };
            (values(min), values(&accessor["max"]))
        });
        assert_eq!(
            bounds,
            declared
                .bounds()
                .map(|(min, max)| (min.to_vec(), max.to_vec()))
        );
    }
    let indices = &accessors[primitive.indices().unwrap().0];
    assert_eq!(
        indices["count"].as_u64(),
        Some(encoded.index_count() as u64)
    );

    // Same geometry read back.
    let mut contract = DracoPrimitiveContract::new().with_indices(encoded.index_count() as u64);
    for declared in encoded.accessors() {
        contract = contract.with_attribute(
            declared.semantic(),
            declared.count() as u64,
            declared.normalized(),
        );
    }
    let decoded = encoded
        .extension(0)
        .decode(encoded.bytes(), &contract)
        .unwrap();
    assert_eq!(decoded, compressed.read_primitive(first).unwrap());
    assert_eq!(decoded.vertex_count(), encoded.accessors()[0].count());
}

#[test]
fn host_neutral_draco_encode_refuses_points() {
    use draco_gltf::{
        ComponentType, DracoPrimitiveEncoding, PackedAttribute, PackedGeometry, PrimitiveMode,
    };

    let position =
        PackedAttribute::new("POSITION", 1, 3, ComponentType::F32, false, vec![0; 12]).unwrap();
    let points = PackedGeometry::new(PrimitiveMode::Points, vec![position], None).unwrap();
    assert!(DracoPrimitiveEncoding::encode(&points, &CompressionOptions::default()).is_err());
}
