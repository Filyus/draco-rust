use super::*;
use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::{FaceIndex, PointIndex};

fn compressed_document(positions: &[[f32; 3]], faces: &[[u32; 3]]) -> Value {
    let mut position = PointAttribute::new();
    position
        .try_init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            positions.len(),
        )
        .unwrap();
    for (index, row) in positions.iter().enumerate() {
        let bytes: Vec<_> = row.iter().flat_map(|value| value.to_le_bytes()).collect();
        assert!(position.buffer_mut().try_write(index * 12, &bytes));
    }
    let mut mesh = Mesh::new();
    mesh.set_num_points(positions.len());
    mesh.add_attribute(position);
    for face in faces {
        mesh.add_face([
            PointIndex(face[0]),
            PointIndex(face[1]),
            PointIndex(face[2]),
        ]);
    }
    let mut writer = draco_io::GltfWriter::new();
    writer.add_draco_mesh(&mesh, None, None).unwrap();
    serde_json::from_str(&writer.to_gltf_embedded().unwrap()).unwrap()
}

fn decoded_faces(mesh: &Mesh) -> Vec<u32> {
    (0..mesh.num_faces())
        .flat_map(|face| {
            mesh.face(FaceIndex(face as u32))
                .into_iter()
                .map(|point| point.0)
        })
        .collect()
}

fn unique_id_writer_document() -> (Value, BTreeMap<String, Vec<u8>>) {
    fn attribute(
        attribute_type: GeometryAttributeType,
        components: u8,
        data_type: DataType,
        count: usize,
        unique_id: u32,
        bytes: &[u8],
    ) -> PointAttribute {
        let mut attribute = PointAttribute::new();
        attribute
            .try_init(attribute_type, components, data_type, false, count)
            .unwrap();
        assert!(attribute.buffer_mut().try_write(0, bytes));
        attribute.set_unique_id(unique_id);
        attribute
    }

    let positions: Vec<_> = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    let normals: Vec<_> = [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    let feature_ids = vec![3u8, 5, 7];

    let mut mesh = Mesh::new();
    mesh.set_num_points(3);
    mesh.add_attribute_preserve_unique_id(attribute(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        3,
        10,
        &positions,
    ));
    mesh.add_attribute_preserve_unique_id(attribute(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        3,
        20,
        &normals,
    ));
    mesh.add_attribute_preserve_unique_id(attribute(
        GeometryAttributeType::Generic,
        1,
        DataType::Uint8,
        3,
        30,
        &feature_ids,
    ));
    mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);

    let mut writer = draco_io::GltfWriter::new();
    writer
        .add_draco_mesh(
            &mesh,
            Some("unique ids"),
            GltfCompressionOptions {
                quantization: QuantizationOptions {
                    position: None,
                    normal: None,
                    color: None,
                    texcoord: None,
                    generic: None,
                },
                ..GltfCompressionOptions::default()
            },
        )
        .unwrap();
    let mut document: Value = serde_json::from_str(&writer.to_gltf_embedded().unwrap()).unwrap();

    // GltfWriter names a generic attribute `_GENERIC_0`; the full-scene APIs
    // preserve the application semantic carried by the primitive, so exercise
    // the custom `_FEATURE_ID_*` spelling without altering the Draco payload.
    let primitive = &mut document["meshes"][0]["primitives"][0];
    let accessor = primitive["attributes"]
        .as_object_mut()
        .unwrap()
        .remove("_GENERIC_0")
        .unwrap();
    primitive["attributes"]
        .as_object_mut()
        .unwrap()
        .insert("_FEATURE_ID_0".into(), accessor);
    let unique_id = primitive["extensions"][KHR_DRACO]["attributes"]
        .as_object_mut()
        .unwrap()
        .remove("_GENERIC_0")
        .unwrap();
    primitive["extensions"][KHR_DRACO]["attributes"]
        .as_object_mut()
        .unwrap()
        .insert("_FEATURE_ID_0".into(), unique_id);

    (
        document,
        BTreeMap::from([
            ("POSITION".into(), positions),
            ("NORMAL".into(), normals),
            ("_FEATURE_ID_0".into(), feature_ids),
        ]),
    )
}

fn decompressed_accessor_bytes(import: &Import, semantic: &str) -> Vec<u8> {
    let primitive = import
        .document
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    let accessor = primitive
        .attributes()
        .find(|(candidate, _)| candidate.to_string() == semantic)
        .unwrap()
        .1
        .index();
    resolved_accessor_bytes(import, accessor)
}

fn resolved_accessor_bytes(import: &Import, accessor: usize) -> Vec<u8> {
    let accessor = import.document.accessors().nth(accessor).unwrap();
    let view = accessor.view().unwrap();
    let buffer = &import.buffers[view.buffer().index()];
    let row = accessor.dimensions().multiplicity() * accessor.data_type().size();
    let stride = view.stride().unwrap_or(row);
    let base = view.offset() + accessor.offset();
    let mut bytes = Vec::with_capacity(accessor.count() * row);
    for index in 0..accessor.count() {
        let start = base + index * stride;
        bytes.extend_from_slice(&buffer[start..start + row]);
    }
    bytes
}

fn parsed_output_json(bytes: &[u8]) -> Value {
    let container = draco_io::parse_gltf_container(bytes).unwrap();
    serde_json::from_slice(container.json).unwrap()
}

fn assert_unknown_json(document: &Value, has_draco: bool) {
    assert_eq!(document["vendorRoot"]["answer"], 42);
    let primitive = &document["meshes"][0]["primitives"][0];
    assert_eq!(primitive["vendorPrimitive"]["token"], "keep-me");
    assert_eq!(
        primitive["extras"]["applicationData"],
        serde_json::json!([1, 2, 3])
    );
    assert_eq!(
        primitive["extensions"]["VENDOR_safe"]["payload"],
        "opaque-json"
    );
    assert_eq!(primitive["extensions"].get(KHR_DRACO).is_some(), has_draco);
}

#[test]
fn strict_extension_accepts_non_positional_unique_ids() {
    let parsed = parse_draco_extension(&serde_json::json!({
        "bufferView": 7,
        "attributes": { "POSITION": 10, "NORMAL": 20, "_FEATURE_ID_0": 30 }
    }))
    .unwrap();
    assert_eq!(parsed.buffer_view, 7);
    assert_eq!(parsed.attributes["POSITION"], 10);
    assert_eq!(parsed.attributes["NORMAL"], 20);
    assert_eq!(parsed.attributes["_FEATURE_ID_0"], 30);
}

#[test]
fn writer_both_readers_and_decompression_preserve_unique_ids_and_bytes() {
    let (document, expected) = unique_id_writer_document();
    let bytes = serde_json::to_vec(&document).unwrap();

    let io_reader = draco_io::GltfReader::from_bytes(&bytes).unwrap();
    let primitives = io_reader.draco_primitives();
    assert_eq!(primitives.len(), 1);
    assert_eq!(primitives[0].attributes["POSITION"], 10);
    assert_eq!(primitives[0].attributes["NORMAL"], 20);
    assert_eq!(primitives[0].attributes["_FEATURE_ID_0"], 30);
    let io_mesh = io_reader.decode_draco_mesh(&primitives[0]).unwrap();
    for (semantic, unique_id) in [("POSITION", 10), ("NORMAL", 20), ("_FEATURE_ID_0", 30)] {
        assert_eq!(
            attribute_bytes(&io_mesh, unique_id).unwrap(),
            expected[semantic]
        );
    }

    let mut import = import_slice(&bytes, None).unwrap();
    {
        let primitive = import.draco_primitives().next().unwrap().1;
        let map = draco_attribute_map(&primitive).unwrap().unwrap();
        assert_eq!(map["POSITION"], 10);
        assert_eq!(map["NORMAL"], 20);
        assert_eq!(map["_FEATURE_ID_0"], 30);
        let mesh = import.decode_primitive(&primitive).unwrap();
        for (semantic, unique_id) in [("POSITION", 10), ("NORMAL", 20), ("_FEATURE_ID_0", 30)] {
            assert_eq!(
                attribute_bytes(&mesh, unique_id).unwrap(),
                expected[semantic]
            );
        }
    }

    import.decompress_in_place().unwrap();
    assert_eq!(import.draco_primitives().count(), 0);
    for semantic in ["POSITION", "NORMAL", "_FEATURE_ID_0"] {
        assert_eq!(
            decompressed_accessor_bytes(&import, semantic),
            expected[semantic]
        );
    }
}

#[test]
fn unknown_unique_id_is_a_controlled_error_and_decompression_is_atomic() {
    let (mut document, _) = unique_id_writer_document();
    document["meshes"][0]["primitives"][0]["extensions"][KHR_DRACO]["attributes"]["POSITION"] =
        Value::from(99u64);
    let bytes = serde_json::to_vec(&document).unwrap();

    let io_reader = draco_io::GltfReader::from_bytes(&bytes).unwrap();
    let primitive = io_reader.draco_primitives().remove(0);
    assert!(io_reader.decode_draco_mesh(&primitive).is_err());

    let mut import = import_slice(&bytes, None).unwrap();
    let before_document = serde_json::to_value(import.document.clone().into_json()).unwrap();
    let before_buffers = import.buffers.clone();
    assert!(import.decompress_in_place().is_err());
    assert_eq!(
        serde_json::to_value(import.document.clone().into_json()).unwrap(),
        before_document
    );
    assert_eq!(import.buffers, before_buffers);
}

#[test]
fn import_rejects_unique_id_above_u32() {
    let (mut document, _) = unique_id_writer_document();
    document["meshes"][0]["primitives"][0]["extensions"][KHR_DRACO]["attributes"]["POSITION"] =
        Value::from(u64::from(u32::MAX) + 1);
    let bytes = serde_json::to_vec(&document).unwrap();
    assert!(draco_io::GltfReader::from_bytes(&bytes).is_err());
    assert!(import_slice(&bytes, None).is_err());
}

#[test]
fn arbitrary_json_extensions_and_extras_survive_all_import_transforms() {
    let (mut document, _) = unique_id_writer_document();
    document["vendorRoot"] = serde_json::json!({ "answer": 42 });
    document["extensionsUsed"]
        .as_array_mut()
        .unwrap()
        .push(Value::from("VENDOR_safe"));
    let primitive = &mut document["meshes"][0]["primitives"][0];
    primitive["vendorPrimitive"] = serde_json::json!({ "token": "keep-me" });
    primitive["extras"] = serde_json::json!({ "applicationData": [1, 2, 3] });
    primitive["extensions"]["VENDOR_safe"] = serde_json::json!({ "payload": "opaque-json" });

    let bytes = serde_json::to_vec(&document).unwrap();
    let mut import = import_slice(&bytes, None).unwrap();

    let saved = import.to_bytes(OutputFormat::GltfEmbeddedBuffers).unwrap();
    assert_unknown_json(&parsed_output_json(&saved), true);

    let compressed = import
        .compress_with_options(&GltfCompressionOptions {
            output_format: OutputFormat::GltfEmbeddedBuffers,
            ..GltfCompressionOptions::default()
        })
        .unwrap();
    assert_unknown_json(&parsed_output_json(&compressed.data), true);

    import.decompress_in_place().unwrap();
    let decompressed = import.to_bytes(OutputFormat::GltfEmbeddedBuffers).unwrap();
    assert_unknown_json(&parsed_output_json(&decompressed), false);

    let recompressed = import
        .compress_with_options(&GltfCompressionOptions {
            output_format: OutputFormat::GltfEmbeddedBuffers,
            ..GltfCompressionOptions::default()
        })
        .unwrap();
    assert_unknown_json(&parsed_output_json(&recompressed.data), true);
}

#[test]
fn decompression_clones_accessor_shared_with_animation_and_plain_primitive() {
    let (mut document, expected) = unique_id_writer_document();
    let position_accessor = document["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
        .as_u64()
        .unwrap() as usize;
    let uri = document["buffers"][0]["uri"].as_str().unwrap();
    let mut buffer = draco_io::decode_data_uri(uri, None).unwrap();

    while !buffer.len().is_multiple_of(4) {
        buffer.push(0);
    }
    let position_offset = buffer.len();
    buffer.extend_from_slice(&expected["POSITION"]);
    let position_view = document["bufferViews"].as_array().unwrap().len();
    document["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "buffer": 0,
            "byteOffset": position_offset,
            "byteLength": expected["POSITION"].len()
        }));
    document["accessors"][position_accessor]["bufferView"] = Value::from(position_view as u64);

    while !buffer.len().is_multiple_of(4) {
        buffer.push(0);
    }
    let time_offset = buffer.len();
    let time_bytes: Vec<_> = [0.0f32, 1.0, 2.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    buffer.extend_from_slice(&time_bytes);
    let time_view = document["bufferViews"].as_array().unwrap().len();
    document["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "buffer": 0,
            "byteOffset": time_offset,
            "byteLength": time_bytes.len()
        }));
    let time_accessor = document["accessors"].as_array().unwrap().len();
    document["accessors"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "bufferView": time_view,
            "componentType": 5126,
            "count": 3,
            "type": "SCALAR",
            "min": [0.0],
            "max": [2.0]
        }));

    document["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "attributes": { "POSITION": position_accessor },
            "mode": 4
        }));
    document["animations"] = serde_json::json!([{
        "samplers": [{
            "input": time_accessor,
            "output": position_accessor,
            "interpolation": "LINEAR"
        }],
        "channels": [{
            "sampler": 0,
            "target": { "node": 0, "path": "translation" }
        }]
    }]);
    document["buffers"][0]["byteLength"] = Value::from(buffer.len() as u64);
    document["buffers"][0]["uri"] =
        Value::from(draco_io::encode_data_uri("application/octet-stream", &buffer).unwrap());

    let mut import = import_slice(&serde_json::to_vec(&document).unwrap(), None).unwrap();
    assert_eq!(
        resolved_accessor_bytes(&import, position_accessor),
        expected["POSITION"]
    );
    import.decompress_in_place().unwrap();

    let output = serde_json::to_value(import.document.clone().into_json()).unwrap();
    let decompressed_accessor = output["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
        .as_u64()
        .unwrap() as usize;
    assert_ne!(decompressed_accessor, position_accessor);
    assert_eq!(
        output["meshes"][0]["primitives"][1]["attributes"]["POSITION"],
        position_accessor as u64
    );
    assert_eq!(
        output["animations"][0]["samplers"][0]["output"],
        position_accessor as u64
    );
    assert_eq!(
        resolved_accessor_bytes(&import, position_accessor),
        expected["POSITION"]
    );
    assert_eq!(
        resolved_accessor_bytes(&import, decompressed_accessor),
        expected["POSITION"]
    );
}

#[test]
fn strict_extension_rejects_ids_outside_u32_and_malformed_maps() {
    for value in [
        serde_json::json!({ "bufferView": 0, "attributes": {} }),
        serde_json::json!({ "bufferView": 0, "attributes": { "POSITION": -1 } }),
        serde_json::json!({ "bufferView": 0, "attributes": { "POSITION": 4294967296u64 } }),
        serde_json::json!({ "bufferView": 0, "attributes": { "POSITION": 1 }, "extra": true }),
    ] {
        assert!(parse_draco_extension(&value).is_err(), "accepted {value}");
    }
}

#[test]
fn attribute_materialization_uses_unique_id_not_attribute_position() {
    fn attribute(id: u32, bytes: &[u8]) -> PointAttribute {
        let mut attribute = PointAttribute::new();
        attribute
            .try_init(
                GeometryAttributeType::Generic,
                1,
                DataType::Uint8,
                false,
                bytes.len(),
            )
            .unwrap();
        attribute.buffer_mut().write(0, bytes);
        attribute.set_unique_id(id);
        attribute
    }

    let mut mesh = Mesh::new();
    mesh.set_num_points(2);
    mesh.add_attribute_preserve_unique_id(attribute(20, &[2, 3]));
    mesh.add_attribute_preserve_unique_id(attribute(10, &[7, 9]));
    assert_eq!(attribute_bytes(&mesh, 10).unwrap(), vec![7, 9]);
    assert!(attribute_bytes(&mesh, 30).is_err());
}

#[test]
fn accessor_graph_counts_standard_consumers_but_never_extras() {
    let document = serde_json::json!({
        "accessors": [{}, {}, {}, {}, {}],
        "meshes": [{ "primitives": [{
            "attributes": { "POSITION": 0 },
            "indices": 1,
            "targets": [{ "POSITION": 2 }],
            "extras": { "accessor": 4 }
        }] }],
        "animations": [{ "samplers": [{ "input": 0, "output": 3 }] }],
        "skins": [{ "inverseBindMatrices": 4 }],
        "nodes": [{ "extensions": {
            "EXT_mesh_gpu_instancing": { "attributes": { "TRANSLATION": 2 } }
        }}]
    });
    assert_eq!(
        accessor_usage_counts(&document).unwrap(),
        vec![2, 1, 2, 1, 1]
    );
}

#[test]
fn accessor_graph_rejects_malformed_known_extension_references() {
    for accessor in [Value::from("0"), Value::from(4u64)] {
        let document = serde_json::json!({
            "accessors": [{}],
            "nodes": [{ "extensions": {
                "EXT_mesh_gpu_instancing": { "attributes": { "TRANSLATION": accessor } }
            }}]
        });
        assert!(accessor_usage_counts(&document).is_err());
    }
}

#[test]
fn shared_accessor_is_cloned_before_decompression_mutates_it() {
    let mut document = serde_json::json!({
        "accessors": [{ "componentType": 5126, "count": 3, "type": "VEC3" }],
        "meshes": [{ "primitives": [
            { "attributes": { "POSITION": 0 } },
            { "attributes": { "POSITION": 0 } }
        ] }]
    });
    let usage = accessor_usage_counts(&document).unwrap();
    let writable = writable_accessor(&mut document, &usage, 0, 0, Some("POSITION"), 0).unwrap();
    assert_eq!(writable, 1);
    assert_eq!(
        document["meshes"][0]["primitives"][0]["attributes"]["POSITION"],
        1
    );
    assert_eq!(
        document["meshes"][0]["primitives"][1]["attributes"]["POSITION"],
        0
    );
    assert_eq!(document["accessors"][0], document["accessors"][1]);
}

#[test]
fn consolidation_aligns_buffers_and_ignores_extras_keys() {
    let document = serde_json::json!({
        "asset": { "version": "2.0" },
        "buffers": [{ "byteLength": 2 }, { "byteLength": 1 }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 1, "byteLength": 1 },
            { "buffer": 1, "byteLength": 1 }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5121, "count": 1, "type": "SCALAR" },
            { "bufferView": 1, "componentType": 5121, "count": 1, "type": "SCALAR" }
        ],
        "extras": { "bufferView": 123, "byteOffset": 456 }
    });
    let (document, bin) =
        draco_io::consolidate_gltf_buffers(document, &[vec![1, 2], vec![3]]).unwrap();
    assert_eq!(bin, vec![2, 0, 0, 0, 3]);
    assert_eq!(document["bufferViews"][0]["byteOffset"], 0);
    assert_eq!(document["bufferViews"][1]["byteOffset"], 4);
    assert_eq!(document["bufferViews"][1]["buffer"], 0);
    assert_eq!(document["extras"]["bufferView"], 123);
}

#[test]
fn consolidation_rejects_unknown_extension_binary_references() {
    let document = serde_json::json!({
        "asset": { "version": "2.0" },
        "buffers": [],
        "extensions": { "VENDOR_binary": { "bufferView": 0 } }
    });
    assert!(matches!(
        draco_io::consolidate_gltf_buffers(document, &[]),
        Err(GltfError::OpaqueBinaryReference(_))
    ));
}

#[test]
fn import_options_resolve_companions_and_enforce_quotas() {
    let document = serde_json::to_vec(&serde_json::json!({
        "asset": { "version": "2.0" },
        "buffers": [{ "uri": "mesh.bin", "byteLength": 3 }]
    }))
    .unwrap();
    let resolver = |uri: &str| {
        assert_eq!(uri, "mesh.bin");
        Ok(vec![1, 2, 3])
    };
    let options = ImportOptions {
        resolver: Some(&resolver),
        ..ImportOptions::default()
    };
    assert_eq!(
        import_slice_with_options(&document, &options)
            .unwrap()
            .buffers[0],
        vec![1, 2, 3]
    );

    let denied = import_slice_with_options(&document, &ImportOptions::default());
    assert!(matches!(
        denied,
        Err(Error::DracoIo(GltfError::ExternalResourceDenied(_)))
    ));

    let options = ImportOptions {
        resolver: Some(&resolver),
        limits: ResourceLimits {
            max_resource_bytes: Some(2),
            ..ResourceLimits::default()
        },
        ..ImportOptions::default()
    };
    assert!(matches!(
        import_slice_with_options(&document, &options),
        Err(Error::ResourceLimit(_)) | Err(Error::DracoIo(_))
    ));
}

#[test]
fn triangle_strip_decompresses_to_equivalent_oriented_triangles() {
    let mut document = compressed_document(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ],
        &[[0, 1, 2], [2, 1, 3]],
    );
    let index_accessor = document["meshes"][0]["primitives"][0]["indices"]
        .as_u64()
        .unwrap() as usize;
    document["meshes"][0]["primitives"][0]["mode"] = Value::from(5u64);
    document["accessors"][index_accessor]["count"] = Value::from(4u64);

    let bytes = serde_json::to_vec(&document).unwrap();
    let mut import = import_slice(&bytes, None).unwrap();
    let primitive = import.draco_primitives().next().unwrap().1;
    let expected = decoded_faces(&import.decode_primitive(&primitive).unwrap());
    import.decompress_in_place().unwrap();

    let primitive = import
        .document
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    assert_eq!(primitive.mode(), gltf::mesh::Mode::Triangles);
    let actual: Vec<_> = primitive
        .reader(|buffer| import.buffers.get(buffer.index()).map(Vec::as_slice))
        .read_indices()
        .unwrap()
        .into_u32()
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn nonindexed_draco_primitive_gains_triangle_indices() {
    let mut document = compressed_document(
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
        ],
        &[[0, 1, 2], [3, 4, 5]],
    );
    let index_accessor = document["meshes"][0]["primitives"][0]["indices"]
        .as_u64()
        .unwrap() as usize;
    document["meshes"][0]["primitives"][0]
        .as_object_mut()
        .unwrap()
        .remove("indices");
    let accessors = document["accessors"].as_array_mut().unwrap();
    assert_eq!(index_accessor + 1, accessors.len());
    accessors.pop();

    let bytes = serde_json::to_vec(&document).unwrap();
    let mut import = import_slice(&bytes, None).unwrap();
    let primitive = import.draco_primitives().next().unwrap().1;
    let expected = decoded_faces(&import.decode_primitive(&primitive).unwrap());
    import.decompress_in_place().unwrap();

    let primitive = import
        .document
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    let actual: Vec<_> = primitive
        .reader(|buffer| import.buffers.get(buffer.index()).map(Vec::as_slice))
        .read_indices()
        .unwrap()
        .into_u32()
        .collect();
    assert_eq!(actual, expected);
    assert!(!actual.is_empty());
}

#[test]
fn decompressed_import_serializes_as_valid_glb_and_same_as_input() {
    let document = compressed_document(
        &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        &[[0, 1, 2]],
    );
    let mut import = import_slice(&serde_json::to_vec(&document).unwrap(), None).unwrap();
    import.decompress_in_place().unwrap();

    let json = import.to_bytes(OutputFormat::SameAsInput).unwrap();
    assert_ne!(json.get(..4), Some(b"glTF".as_slice()));
    import_slice(&json, None).unwrap();

    let glb = import.to_bytes(OutputFormat::Glb).unwrap();
    assert_eq!(glb.get(..4), Some(b"glTF".as_slice()));
    let roundtrip = import_slice(&glb, None).unwrap();
    let primitive = roundtrip
        .document
        .meshes()
        .next()
        .unwrap()
        .primitives()
        .next()
        .unwrap();
    let reader =
        primitive.reader(|buffer| roundtrip.buffers.get(buffer.index()).map(Vec::as_slice));
    assert_eq!(reader.read_positions().unwrap().count(), 3);
    assert_eq!(reader.read_indices().unwrap().into_u32().count(), 3);
}

#[test]
fn native_import_decodes_draco_without_gltf_rs_types() {
    let document = compressed_document(
        &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        &[[0, 1, 2]],
    );
    let import = parse_native(
        &serde_json::to_vec(&document).unwrap(),
        ValidationProfile::Gltf20,
    )
    .unwrap();
    let primitive = import.draco_primitives().next().unwrap();
    let mesh = import
        .decode_primitive(primitive, &ExtensionRegistry::default())
        .unwrap();
    assert_eq!(mesh.num_faces(), 1);
    assert_eq!(mesh.num_points(), 3);
}
