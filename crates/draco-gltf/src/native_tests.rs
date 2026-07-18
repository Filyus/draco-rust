use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::{Mesh, PointIndex};

use crate::*;

fn compressed_document() -> serde_json::Value {
    let mut positions = PointAttribute::new();
    positions
        .try_init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            3,
        )
        .unwrap();
    for (index, point) in [[0.0_f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        .iter()
        .enumerate()
    {
        let bytes: Vec<_> = point.iter().flat_map(|value| value.to_le_bytes()).collect();
        assert!(positions.buffer_mut().try_write(index * 12, &bytes));
    }
    let mut mesh = Mesh::new();
    mesh.set_num_points(3);
    mesh.add_attribute(positions);
    mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);
    let mut writer = draco_io::GltfWriter::new();
    writer.add_draco_mesh(&mesh, None, None).unwrap();
    serde_json::from_str(&writer.to_gltf_embedded().unwrap()).unwrap()
}

#[test]
fn native_import_preserves_unknown_json_and_decodes_draco() {
    let mut value = compressed_document();
    value["customDraftField"] = serde_json::json!({"keep": [1, 2, 3]});
    let input = serde_json::to_vec(&value).unwrap();
    let import = parse_native(&input, ValidationProfile::Gltf20).unwrap();
    assert_eq!(import.document.as_value()["customDraftField"]["keep"][2], 3);
    assert_eq!(import.document.to_json_bytes().unwrap(), input);
    let primitive = import.draco_primitives().next().unwrap();
    let mesh = import.decode_primitive(primitive).unwrap();
    assert_eq!(mesh.num_faces(), 1);
    assert_eq!(mesh.num_points(), 3);
}

#[test]
fn native_decompression_materializes_plain_geometry() {
    let input = serde_json::to_vec(&compressed_document()).unwrap();
    let mut import = parse_native(&input, ValidationProfile::Gltf20).unwrap();
    import.decompress_in_place().unwrap();
    assert_eq!(import.draco_primitives().count(), 0);
    let primitive = import.document.primitive(MeshIndex(0), 0).unwrap();
    assert_eq!(primitive.value()["mode"], 4);
    assert!(primitive.value()["indices"].is_u64());
    assert!(primitive.value()["attributes"]["POSITION"].is_u64());
    let serialized = import.to_bytes(OutputFormat::GltfEmbeddedBuffers).unwrap();
    parse_native(&serialized, ValidationProfile::Gltf20).unwrap();
}

#[test]
fn draft_profile_accepts_files_shapes_and_nonsequential_semantics() {
    let value = serde_json::json!({
        "asset": { "version": "2.1" },
        "files": [{ "uri": "child.gltf" }],
        "shapes": [{ "type": "box", "uid": "shape-1" }],
        "meshes": [{ "primitives": [{ "attributes": { "TEXCOORD_1": 0 } }] }],
        "accessors": [{ "componentType": 5134, "count": 1, "type": "SCALAR" }]
    });
    let document = Document::from_value(value).unwrap();
    document.validate(ValidationProfile::Gltf21Draft).unwrap();
    assert_eq!(document.files().len(), 1);
    assert_eq!(
        document.shapes().get(ShapeIndex(0)).unwrap().uid(),
        Some("shape-1")
    );
}

#[test]
fn native_import_reads_glb_v3() {
    let json = serde_json::json!({ "asset": { "version": "2.1" } });
    let bytes = draco_io::build_glb_v3_container(&json, &[]).unwrap();
    let import = parse_native(&bytes, ValidationProfile::Gltf21Draft).unwrap();
    assert_eq!(import.input_format, GltfContainerFormat::GlbV3);
}
