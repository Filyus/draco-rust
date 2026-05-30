use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_io::{
    FbxMemoryReader, FbxWriter, GltfReader, GltfWriter, ObjReader, ObjWriter, PlyReader, PlyWriter,
    ReadFromBytes, Reader, WriteToBytes, Writer,
};

fn triangle_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        3,
    );

    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let buffer = pos_att.buffer_mut();
    for (i, pos) in positions.iter().enumerate() {
        let bytes: Vec<u8> = pos
            .iter()
            .flat_map(|component| component.to_le_bytes())
            .collect();
        buffer.write(i * 12, &bytes);
    }
    mesh.add_attribute(pos_att);

    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh
}

fn assert_triangle_roundtrip(mesh: Mesh) {
    assert_eq!(mesh.num_points(), 3);
    assert_eq!(mesh.num_faces(), 1);
}

#[test]
fn obj_supports_byte_writer_and_reader_traits() {
    let mut writer = ObjWriter::new();
    writer.add_mesh(&triangle_mesh(), Some("Triangle")).unwrap();

    let bytes = WriteToBytes::write_to_vec(&writer).unwrap();
    let mut reader = <ObjReader as ReadFromBytes>::from_bytes(&bytes).unwrap();
    assert_triangle_roundtrip(reader.read_mesh().unwrap());
}

#[test]
fn ply_supports_byte_writer_and_reader_traits() {
    let mut writer = PlyWriter::new();
    writer.add_mesh(&triangle_mesh(), Some("Triangle")).unwrap();

    let bytes = WriteToBytes::write_to_vec(&writer).unwrap();
    let mut reader = <PlyReader as ReadFromBytes>::from_bytes(&bytes).unwrap();
    assert_triangle_roundtrip(reader.read_mesh().unwrap());
}

#[test]
fn gltf_supports_byte_writer_and_reader_traits() {
    let mut writer = GltfWriter::new();
    writer.add_mesh(&triangle_mesh(), Some("Triangle")).unwrap();

    let bytes = WriteToBytes::write_to_vec(&writer).unwrap();
    let mut reader = <GltfReader as ReadFromBytes>::from_bytes(&bytes).unwrap();
    assert_triangle_roundtrip(reader.read_mesh().unwrap());
}

#[test]
fn fbx_supports_byte_writer_and_reader_traits() {
    let mut writer = FbxWriter::new();
    writer.add_mesh(&triangle_mesh(), Some("Triangle")).unwrap();

    let bytes = WriteToBytes::write_to_vec(&writer).unwrap();
    let mut reader = <FbxMemoryReader as ReadFromBytes>::from_bytes(&bytes).unwrap();
    assert_triangle_roundtrip(reader.read_mesh().unwrap());
}
