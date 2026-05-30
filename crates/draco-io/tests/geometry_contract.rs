use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_io::{
    FbxMemoryReader, FbxWriter, GltfReader, ObjReader, ObjWriter, PlyReader, PlyWriter,
    ReadFromBytes, Reader, Writer,
};

fn triangle_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    add_f32_attribute(
        &mut mesh,
        GeometryAttributeType::Position,
        3,
        &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
    );
    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh
}

fn triangle_mesh_with_attributes() -> Mesh {
    let mut mesh = triangle_mesh();
    add_f32_attribute(
        &mut mesh,
        GeometryAttributeType::Normal,
        3,
        &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
    );
    add_u8_attribute(
        &mut mesh,
        GeometryAttributeType::Color,
        4,
        true,
        &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255],
    );
    add_f32_attribute(
        &mut mesh,
        GeometryAttributeType::TexCoord,
        2,
        &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
    );
    mesh
}

fn add_f32_attribute(
    mesh: &mut Mesh,
    attribute_type: GeometryAttributeType,
    components: u8,
    values: &[f32],
) {
    let mut attribute = PointAttribute::new();
    attribute.init(
        attribute_type,
        components,
        DataType::Float32,
        false,
        values.len() / components as usize,
    );
    let bytes: Vec<u8> = values
        .iter()
        .flat_map(|component| component.to_le_bytes())
        .collect();
    attribute.buffer_mut().write(0, &bytes);
    mesh.add_attribute(attribute);
}

fn add_u8_attribute(
    mesh: &mut Mesh,
    attribute_type: GeometryAttributeType,
    components: u8,
    normalized: bool,
    values: &[u8],
) {
    let mut attribute = PointAttribute::new();
    attribute.init(
        attribute_type,
        components,
        DataType::Uint8,
        normalized,
        values.len() / components as usize,
    );
    attribute.buffer_mut().write(0, values);
    mesh.add_attribute(attribute);
}

fn count_attributes(mesh: &Mesh, attribute_type: GeometryAttributeType) -> usize {
    (0..mesh.num_attributes())
        .filter(|&i| mesh.attribute(i).attribute_type() == attribute_type)
        .count()
}

#[test]
fn obj_roundtrips_normals_and_texcoords() {
    let mut writer = ObjWriter::new();
    let mut mesh = triangle_mesh();
    add_f32_attribute(
        &mut mesh,
        GeometryAttributeType::Normal,
        3,
        &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
    );
    add_f32_attribute(
        &mut mesh,
        GeometryAttributeType::TexCoord,
        2,
        &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
    );
    writer.add_mesh(&mesh, Some("triangle")).unwrap();

    let bytes = writer.write_to_vec().unwrap();
    let mut reader = ObjReader::from_bytes(bytes);
    let decoded = reader.read_mesh().unwrap();

    assert_eq!(decoded.num_faces(), 1);
    assert!(decoded
        .named_attribute(GeometryAttributeType::Normal)
        .is_some());
    assert!(decoded
        .named_attribute(GeometryAttributeType::TexCoord)
        .is_some());
}

#[test]
fn ply_roundtrips_normals_colors_and_texcoords() {
    let mut writer = PlyWriter::new();
    writer
        .add_mesh(&triangle_mesh_with_attributes(), Some("triangle"))
        .unwrap();

    let bytes = writer.write_to_vec().unwrap();
    let mut reader = PlyReader::from_bytes(bytes);
    let decoded = reader.read_mesh().unwrap();

    assert_eq!(decoded.num_faces(), 1);
    assert!(decoded
        .named_attribute(GeometryAttributeType::Normal)
        .is_some());
    assert!(decoded
        .named_attribute(GeometryAttributeType::Color)
        .is_some());
    assert!(decoded
        .named_attribute(GeometryAttributeType::TexCoord)
        .is_some());
}

#[test]
fn gltf_reads_multiple_attribute_sets_and_custom_generic() {
    let mut buffer = Vec::new();
    let positions_offset = push_f32s(&mut buffer, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    let tex0_offset = push_f32s(&mut buffer, &[0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
    let tex1_offset = push_f32s(&mut buffer, &[0.5, 0.5, 0.75, 0.5, 0.5, 0.75]);
    let color_offset = push_f32s(
        &mut buffer,
        &[1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0],
    );
    let generic_offset = push_f32s(&mut buffer, &[0.25, 0.5, 0.75]);
    let data_uri = format!(
        "data:application/octet-stream;base64,{}",
        base64_for_test(&buffer)
    );
    let json = format!(
        r#"{{
            "asset": {{"version": "2.0"}},
            "buffers": [{{"byteLength": {}, "uri": "{}"}}],
            "bufferViews": [
                {{"buffer": 0, "byteOffset": {}, "byteLength": 36}},
                {{"buffer": 0, "byteOffset": {}, "byteLength": 24}},
                {{"buffer": 0, "byteOffset": {}, "byteLength": 24}},
                {{"buffer": 0, "byteOffset": {}, "byteLength": 48}},
                {{"buffer": 0, "byteOffset": {}, "byteLength": 12}}
            ],
            "accessors": [
                {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
                {{"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"}},
                {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2"}},
                {{"bufferView": 3, "componentType": 5126, "count": 3, "type": "VEC4"}},
                {{"bufferView": 4, "componentType": 5126, "count": 3, "type": "SCALAR"}}
            ],
            "meshes": [{{
                "primitives": [{{
                    "attributes": {{
                        "POSITION": 0,
                        "TEXCOORD_0": 1,
                        "TEXCOORD_1": 2,
                        "COLOR_0": 3,
                        "_GENERIC_0": 4
                    }},
                    "mode": 4
                }}]
            }}]
        }}"#,
        buffer.len(),
        data_uri,
        positions_offset,
        tex0_offset,
        tex1_offset,
        color_offset,
        generic_offset
    );

    let mesh = GltfReader::from_bytes(json.as_bytes())
        .unwrap()
        .decode_all_meshes()
        .unwrap()
        .remove(0);

    assert_eq!(mesh.num_faces(), 1);
    assert_eq!(count_attributes(&mesh, GeometryAttributeType::TexCoord), 2);
    assert_eq!(count_attributes(&mesh, GeometryAttributeType::Color), 1);
    assert_eq!(count_attributes(&mesh, GeometryAttributeType::Generic), 1);
}

#[test]
fn fbx_writer_rejects_attributes_it_cannot_preserve() {
    let mut writer = FbxWriter::new();
    let err = writer
        .add_mesh(&triangle_mesh_with_attributes(), Some("triangle"))
        .unwrap_err();

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("supports only Position"));
}

#[test]
fn obj_to_ply_preserves_common_geometry_attributes() {
    let obj = br#"
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 0.0 1.0 0.0
vt 0.0 0.0
vt 1.0 0.0
vt 0.0 1.0
vn 0.0 0.0 1.0
f 1/1/1 2/2/1 3/3/1
"#;

    let mut obj_reader = ObjReader::from_bytes(obj.as_slice());
    let mesh = obj_reader.read_mesh().unwrap();

    let mut ply_writer = PlyWriter::new();
    ply_writer.add_mesh(&mesh, None).unwrap();
    let bytes = ply_writer.write_to_vec().unwrap();

    let mut ply_reader = PlyReader::from_bytes(bytes);
    let decoded = ply_reader.read_mesh().unwrap();

    assert_eq!(decoded.num_faces(), 1);
    assert!(decoded
        .named_attribute(GeometryAttributeType::Normal)
        .is_some());
    assert!(decoded
        .named_attribute(GeometryAttributeType::TexCoord)
        .is_some());
}

#[test]
fn fbx_to_ply_smoke_preserves_positions_and_faces() {
    let mut fbx_writer = FbxWriter::new();
    fbx_writer
        .add_mesh(&triangle_mesh(), Some("triangle"))
        .unwrap();
    let bytes = fbx_writer.write_to_vec().unwrap();

    let mut fbx_reader = <FbxMemoryReader as ReadFromBytes>::from_bytes(&bytes).unwrap();
    let mesh = fbx_reader.read_mesh().unwrap();

    let mut ply_writer = PlyWriter::new();
    ply_writer.add_mesh(&mesh, None).unwrap();
    let mut ply_reader = PlyReader::from_bytes(ply_writer.write_to_vec().unwrap());
    let decoded = ply_reader.read_mesh().unwrap();

    assert_eq!(decoded.num_points(), 3);
    assert_eq!(decoded.num_faces(), 1);
}

fn push_f32s(buffer: &mut Vec<u8>, values: &[f32]) -> usize {
    let offset = buffer.len();
    buffer.extend(values.iter().flat_map(|value| value.to_le_bytes()));
    offset
}

fn base64_for_test(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;

        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }

    out
}
