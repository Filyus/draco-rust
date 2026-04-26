use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;

fn testdata_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
}

fn decode_mesh(name: &str) -> Mesh {
    let path = testdata_dir().join(name);
    let mut file = File::open(&path).unwrap_or_else(|e| panic!("open {}: {}", name, e));
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("read {}: {}", name, e));

    let mut buffer = DecoderBuffer::new(&bytes);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder
        .decode(&mut buffer, &mut mesh)
        .unwrap_or_else(|e| panic!("decode {}: {:?}", name, e));
    mesh
}

#[test]
fn test_legacy_quantized_mesh_fixture_decodes_through_mesh_transform_shim() {
    let mesh = decode_mesh("test_nm_quant.0.9.0.drc");
    assert_eq!(mesh.num_points(), 99);
    assert_eq!(mesh.num_faces(), 170);
    assert!(mesh.named_attribute_id(GeometryAttributeType::Position) >= 0);
}

#[test]
fn test_legacy_normal_fixture_decodes_through_mesh_transform_shim() {
    let mesh = decode_mesh("test_nm.obj.edgebreaker.0.9.1.drc");
    assert_eq!(mesh.num_points(), 99);
    assert_eq!(mesh.num_faces(), 170);
    assert!(mesh.named_attribute_id(GeometryAttributeType::Normal) >= 0);
}
