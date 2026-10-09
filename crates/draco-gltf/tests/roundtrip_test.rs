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
    DocumentAccessorSource::new(import.document(), &import.resources)
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
    let original_nodes = import.document().as_value()["nodes"].clone();

    let report = import
        .compress_primitive(MeshIndex(0), 0, CompressionOptions::default())
        .unwrap();
    assert!(report.encoded_bytes > 0);

    let bytes = import.to_bytes(OutputFormat::GlbV2).unwrap();
    let reloaded = parse(&bytes, ValidationProfile::Gltf20).unwrap();
    reloaded
        .document()
        .validate(ValidationProfile::Gltf20)
        .unwrap();
    assert_eq!(reloaded.document().as_value()["nodes"], original_nodes);
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
    let nodes = import.document().as_value()["nodes"].clone();

    import
        .compress_primitive(MeshIndex(0), 0, CompressionOptions::default())
        .unwrap();
    let bytes = import.to_bytes(OutputFormat::GlbV2).unwrap();
    let reloaded = parse(&bytes, ValidationProfile::Gltf20).unwrap();
    reloaded
        .document()
        .validate(ValidationProfile::Gltf20)
        .unwrap();

    assert_eq!(reloaded.document().as_value()["nodes"], nodes);
    let skin_index = reloaded.document().as_value()["skins"][0]["inverseBindMatrices"]
        .as_u64()
        .unwrap() as usize;
    let sampler = &reloaded.document().as_value()["animations"][0]["samplers"][0];
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
    let primitive = import.document().primitive(MeshIndex(0), 0).unwrap();
    let extension = DracoPrimitiveExtension::from_json(
        primitive.extension(KHR_DRACO_MESH_COMPRESSION).unwrap(),
    )
    .unwrap();

    let view = &import.document().as_value()["bufferViews"]
        .as_array()
        .unwrap()[extension.buffer_view()];
    let buffer = &import.resources.buffers[view["buffer"].as_u64().unwrap() as usize];
    let start = view.get("byteOffset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let payload = &buffer[start..start + view["byteLength"].as_u64().unwrap() as usize];

    let mut contract = DracoPrimitiveContract::new().with_profile(ValidationProfile::Gltf20);
    for (semantic, index) in primitive.attribute_indices() {
        let accessor = import.document().accessor(index).unwrap();
        contract =
            contract.with_attribute(semantic, accessor.count().unwrap(), accessor.normalized());
    }
    let indices = import
        .document()
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
    let primitive = compressed.document().primitive(MeshIndex(0), 0).unwrap();
    let extension = DracoPrimitiveExtension::from_json(
        primitive.extension(KHR_DRACO_MESH_COMPRESSION).unwrap(),
    )
    .unwrap();
    assert_eq!(encoded.extension(extension.buffer_view()), extension);
    assert_eq!(
        DracoPrimitiveExtension::from_json(&extension.to_json()).unwrap(),
        extension
    );

    let root = compressed.document().as_value();
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

/// Attributes the extension does not list are read as ordinary accessors.
#[test]
fn draco_primitive_keeps_uncompressed_extra_attributes() {
    use draco_gltf::{JsonValue, PrimitiveIndex};

    let first = PrimitiveIndex {
        mesh: MeshIndex(0),
        primitive: 0,
    };
    let mut compressed = open(
        fixture("testdata/Box/glTF_Binary/Box.glb"),
        ValidationProfile::Gltf20,
    )
    .unwrap();
    compressed
        .compress_primitive(MeshIndex(0), 0, CompressionOptions::default())
        .unwrap();
    let decoded = compressed.read_primitive(first).unwrap();
    let vertices = decoded.vertex_count();

    // One float per decoded vertex, in an ordinary accessor.
    let with_extra = |count: usize| {
        let mut import = compressed.clone();
        let values: Vec<f32> = (0..count).map(|i| i as f32).collect();
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let buffer = import.resources.buffers.len();
        let root = import.document_mut().as_value_mut();
        root["buffers"]
            .as_array_mut()
            .unwrap()
            .push(JsonValue::object([(
                "byteLength",
                JsonValue::from(bytes.len()),
            )]));
        let view = root["bufferViews"].as_array().unwrap().len();
        root["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(JsonValue::object([
                ("buffer", JsonValue::from(buffer)),
                ("byteLength", JsonValue::from(bytes.len())),
            ]));
        let accessor = root["accessors"].as_array().unwrap().len();
        root["accessors"]
            .as_array_mut()
            .unwrap()
            .push(JsonValue::object([
                ("bufferView", JsonValue::from(view)),
                ("componentType", JsonValue::from(5126u64)),
                ("count", JsonValue::from(count)),
                ("type", JsonValue::from("SCALAR")),
            ]));
        root["meshes"][0]["primitives"][0]["attributes"]["_INDEX"] = JsonValue::from(accessor);
        import.resources.buffers.push(bytes.clone());
        (import, bytes)
    };

    let (import, bytes) = with_extra(vertices);
    let geometry = import.read_primitive(first).unwrap();
    assert_eq!(geometry.attributes().len(), decoded.attributes().len() + 1);
    let extra = geometry.attributes().last().unwrap();
    assert_eq!(extra.semantic(), "_INDEX");
    assert_eq!(extra.bytes(), bytes.as_slice());
    // The compressed attributes are unchanged.
    assert_eq!(
        &geometry.attributes()[..decoded.attributes().len()],
        decoded.attributes()
    );

    // Decompression keeps it.
    let mut decompressed = import.clone();
    decompressed.decompress_in_place().unwrap();
    assert_eq!(decompressed.read_primitive(first).unwrap(), geometry);

    // A count that does not match the stream is refused.
    let (mismatched, _) = with_extra(vertices + 1);
    assert!(mismatched.read_primitive(first).is_err());
}

fn declared_count(import: &Import, index: draco_gltf::AccessorIndex) -> usize {
    import.document().accessor(index).unwrap().count().unwrap() as usize
}

/// Accessors declare what the stream decodes to, on a mesh where the encoder
/// merges vertices and one where the decoder splits seams.
#[test]
fn compressed_accessors_match_the_decoded_stream() {
    use draco_gltf::{CompressionMode, PrimitiveIndex};

    let first = PrimitiveIndex {
        mesh: MeshIndex(0),
        primitive: 0,
    };
    for (file, mode) in [
        ("testdata/Fox/glTF/Fox.gltf", CompressionMode::Fallback),
        ("testdata/Fox/glTF/Fox.gltf", CompressionMode::DracoOnly),
        (
            "testdata/KhronosSampleModels/Duck/glTF/Duck.gltf",
            CompressionMode::Fallback,
        ),
        (
            "testdata/KhronosSampleModels/Duck/glTF/Duck.gltf",
            CompressionMode::DracoOnly,
        ),
    ] {
        let mut import = open(fixture(file), ValidationProfile::Gltf20).unwrap();
        import
            .compress_primitive(
                MeshIndex(0),
                0,
                CompressionOptions {
                    mode,
                    ..CompressionOptions::default()
                },
            )
            .unwrap();
        let primitive = import.document().primitive(MeshIndex(0), 0).unwrap();
        let stream = import.decode_draco_primitive(primitive).unwrap();
        for (semantic, index) in primitive.attribute_indices() {
            assert_eq!(
                declared_count(&import, index),
                stream.num_points(),
                "{file} {mode:?} {semantic}"
            );
        }
        assert_eq!(
            declared_count(&import, primitive.indices().unwrap()),
            stream.num_faces() * 3,
            "{file} {mode:?} indices"
        );

        let decoded = import.read_primitive(first).unwrap();
        if mode == CompressionMode::Fallback {
            // Without the extension, a reader sees the same geometry.
            let mut fallback = import.clone();
            fallback.document_mut().as_value_mut()["meshes"][0]["primitives"][0]
                .as_object_mut()
                .unwrap()
                .retain(|(key, _)| key != "extensions");
            let fallback = fallback.read_primitive(first).unwrap();
            assert_eq!(fallback, decoded, "{file} fallback");
        }
    }
}

/// A primitive with morph targets keeps its vertex order; forcing EdgeBreaker
/// is refused.
#[test]
fn compressing_morph_targets_keeps_vertex_order() {
    use draco_gltf::{PrimitiveIndex, QuantizationBits};

    let first = PrimitiveIndex {
        mesh: MeshIndex(0),
        primitive: 0,
    };
    let path =
        fixture("testdata/KhronosSampleModels/AnimatedMorphCube/glTF/AnimatedMorphCube.gltf");
    let source = open(&path, ValidationProfile::Gltf20).unwrap();
    let positions = |geometry: &draco_gltf::PackedGeometry| {
        geometry
            .attributes()
            .iter()
            .find(|attribute| attribute.semantic() == "POSITION")
            .unwrap()
            .bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .collect::<Vec<_>>()
    };
    let before = positions(&source.read_primitive(first).unwrap());

    for (quantization, tolerance) in [
        (QuantizationBits::NONE, 0.0),
        (QuantizationBits::GLTF, 1e-3),
    ] {
        let mut import = source.clone();
        import
            .compress_primitive(
                MeshIndex(0),
                0,
                CompressionOptions {
                    quantization,
                    ..CompressionOptions::default()
                },
            )
            .unwrap();
        let after = positions(&import.read_primitive(first).unwrap());
        assert_eq!(after.len(), before.len());
        for (before, after) in before.iter().zip(&after) {
            assert!((before - after).abs() <= tolerance, "{before} -> {after}");
        }
        let primitive = import.document().primitive(MeshIndex(0), 0).unwrap();
        assert_eq!(primitive.morph_targets().count(), 2);
    }

    let mut forced = source.clone();
    let error = forced
        .compress_primitive(
            MeshIndex(0),
            0,
            CompressionOptions {
                encoding_method: 2,
                ..CompressionOptions::default()
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("morph targets"), "{error}");
}

/// The host-neutral encoder's declarations, `decoded()` and
/// `keeps_vertex_order()`.
#[test]
fn host_neutral_draco_encode_describes_the_decoded_stream() {
    use draco_gltf::{DracoPrimitiveContract, DracoPrimitiveEncoding, PrimitiveIndex};

    let first = PrimitiveIndex {
        mesh: MeshIndex(0),
        primitive: 0,
    };
    let duck = open(
        fixture("testdata/KhronosSampleModels/Duck/glTF/Duck.gltf"),
        ValidationProfile::Gltf20,
    )
    .unwrap();
    let geometry = duck.read_primitive(first).unwrap();

    let encoded =
        DracoPrimitiveEncoding::encode(&geometry, &CompressionOptions::default()).unwrap();
    let decoded = encoded.decoded().unwrap();
    for accessor in encoded.accessors() {
        assert_eq!(accessor.count(), decoded.vertex_count());
    }
    assert_eq!(encoded.index_count(), decoded.indices().unwrap().count());
    // EdgeBreaker renumbers vertices here.
    assert!(!encoded.keeps_vertex_order(&geometry).unwrap());

    let mut contract = DracoPrimitiveContract::new().with_indices(encoded.index_count() as u64);
    for accessor in encoded.accessors() {
        contract = contract.with_attribute(
            accessor.semantic(),
            accessor.count() as u64,
            accessor.normalized(),
        );
    }
    assert_eq!(
        &encoded
            .extension(0)
            .decode(encoded.bytes(), &contract)
            .unwrap(),
        decoded
    );

    let sequential = DracoPrimitiveEncoding::encode(
        &geometry,
        &CompressionOptions {
            encoding_method: 1,
            ..CompressionOptions::default()
        },
    )
    .unwrap();
    assert!(sequential.keeps_vertex_order(&geometry).unwrap());
}

/// glTF requires POSITION bounds to be finite, so a position that is not
/// cannot be compressed into a valid document.
#[test]
fn draco_encode_refuses_positions_that_are_not_finite() {
    use draco_gltf::{
        ComponentType, DracoPrimitiveEncoding, PackedAttribute, PackedGeometry, PrimitiveMode,
    };

    let values = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, f32::INFINITY, 0.0];
    let bytes = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let position =
        PackedAttribute::new("POSITION", 3, 3, ComponentType::F32, false, bytes).unwrap();
    let triangle = PackedGeometry::new(PrimitiveMode::Triangles, vec![position], None).unwrap();
    let Err(error) = DracoPrimitiveEncoding::encode(&triangle, &CompressionOptions::default())
    else {
        panic!("an infinite position must be refused");
    };
    assert!(error.to_string().contains("not finite"), "got: {error}");
}
