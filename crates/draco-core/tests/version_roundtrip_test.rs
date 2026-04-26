// Test uses array indexing patterns that mirror C++ test structure for clarity.
// needless_range_loop: for i in 0..n { arr[i] } makes index-based operations explicit
#![allow(clippy::needless_range_loop)]

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;
use draco_core::version::{
    DEFAULT_MESH_VERSION, DEFAULT_POINT_CLOUD_KD_TREE_VERSION,
    DEFAULT_POINT_CLOUD_SEQUENTIAL_VERSION,
};

fn create_test_pc() -> PointCloud {
    let mut pc = PointCloud::new();
    let mut pos_att = PointAttribute::new();
    let num_points = 3;
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let buffer = pos_att.buffer_mut();
    for i in 0..9 {
        buffer.write(i * 4, &positions[i].to_le_bytes());
    }
    pc.add_attribute(pos_att);
    pc
}

fn create_test_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    let mut pos_att = PointAttribute::new();
    let num_points = 3;
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let buffer = pos_att.buffer_mut();
    for i in 0..9 {
        buffer.write(i * 4, &positions[i].to_le_bytes());
    }
    mesh.add_attribute(pos_att);
    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [0u32.into(), 1u32.into(), 2u32.into()]);
    mesh
}

#[test]
fn test_mesh_roundtrip_v1_3() {
    let mesh = create_test_mesh();
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    // Use legacy v1.3 header for testing sequential encoding with legacy version.
    // We do NOT set quantization_bits here: the Rust encoder never writes the old
    // v < 2.0 quantization-params-before-symbols layout, so round-tripping
    // quantized attributes at v1.3 is not supported (matches current C++ behavior).
    options.set_version(1, 3);
    options.set_encoding_method(0); // Sequential

    let mut enc_buffer = EncoderBuffer::new();
    assert!(encoder.encode(&options, &mut enc_buffer).is_ok());

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    assert!(decoder.decode(&mut dec_buffer, &mut decoded_mesh).is_ok());

    assert_eq!(decoded_mesh.num_points(), 3);
    assert_eq!(decoded_mesh.num_faces(), 1);
}

#[test]
fn test_mesh_roundtrip_v2_2() {
    let mesh = create_test_mesh();
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    options.set_version(DEFAULT_MESH_VERSION.0, DEFAULT_MESH_VERSION.1);
    options.set_encoding_method(1); // Edgebreaker
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut enc_buffer = EncoderBuffer::new();
    assert!(encoder.encode(&options, &mut enc_buffer).is_ok());

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    assert!(decoder.decode(&mut dec_buffer, &mut decoded_mesh).is_ok());

    assert_eq!(decoded_mesh.num_points(), 3);
    assert_eq!(decoded_mesh.num_faces(), 1);
}

#[test]
fn test_point_cloud_roundtrip_v1_3() {
    let pc = create_test_pc();
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);

    let mut options = EncoderOptions::new();
    options.set_version(
        DEFAULT_POINT_CLOUD_SEQUENTIAL_VERSION.0,
        DEFAULT_POINT_CLOUD_SEQUENTIAL_VERSION.1,
    );
    options.set_encoding_method(0); // Sequential
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut enc_buffer = EncoderBuffer::new();
    assert!(encoder.encode(&options, &mut enc_buffer).is_ok());

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_pc = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    assert!(decoder.decode(&mut dec_buffer, &mut decoded_pc).is_ok());

    assert_eq!(decoded_pc.num_points(), 3);
}

#[test]
fn test_point_cloud_roundtrip_v2_3() {
    let pc = create_test_pc();
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);

    let mut options = EncoderOptions::new();
    options.set_version(
        DEFAULT_POINT_CLOUD_KD_TREE_VERSION.0,
        DEFAULT_POINT_CLOUD_KD_TREE_VERSION.1,
    );
    options.set_encoding_method(1); // KD-Tree
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut enc_buffer = EncoderBuffer::new();
    assert!(encoder.encode(&options, &mut enc_buffer).is_ok());

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_pc = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    assert!(decoder.decode(&mut dec_buffer, &mut decoded_pc).is_ok());

    assert_eq!(decoded_pc.num_points(), 3);
}
