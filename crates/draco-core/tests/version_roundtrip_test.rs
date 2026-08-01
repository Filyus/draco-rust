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
use draco_core::version::{DEFAULT_MESH_VERSION, DEFAULT_POINT_CLOUD_VERSION};

/// The oldest point-cloud bitstream this crate still writes when asked for it
/// explicitly. It is no longer any method's default -- upstream writes 2.3 for
/// every point cloud -- so this test names it rather than importing a constant.
const LEGACY_POINT_CLOUD_VERSION: (u8, u8) = (1, 3);

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
    options.set_version(LEGACY_POINT_CLOUD_VERSION.0, LEGACY_POINT_CLOUD_VERSION.1);
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
    options.set_version(DEFAULT_POINT_CLOUD_VERSION.0, DEFAULT_POINT_CLOUD_VERSION.1);
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

/// A claimed version must survive the index-width boundary it straddles.
///
/// Sequential connectivity picks its index width from the point count: u8 below
/// 256, u16 below 65536, then varints — but the decoder only reads varints from
/// 2.2, so at 1.3 the encoder wrote varints the decoder read as u32. The stream
/// decoded without error and with the right face count, and the faces were
/// different ones. Counts are exactly what a test must not stop at here.
#[test]
#[cfg(feature = "legacy_bitstream_encode")]
fn a_sequential_mesh_at_1_3_survives_the_varint_index_boundary() {
    use draco_core::geometry_indices::FaceIndex;

    // 257 x 257 = 66,049 points, just past the 65,536 boundary.
    let mesh = grid_mesh(257);
    assert!(
        mesh.num_points() > 65536,
        "the boundary is the point of this"
    );

    let mut options = EncoderOptions::new();
    options.set_version(1, 3);
    options.set_encoding_method(0);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("1.3 encode");

    let mut decoded = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(buffer.data()), &mut decoded)
        .expect("1.3 decode");

    assert_eq!(decoded.num_faces(), mesh.num_faces(), "face count");
    for face_id in 0..mesh.num_faces() {
        let index = FaceIndex(face_id as u32);
        assert_eq!(
            decoded.face(index),
            mesh.face(index),
            "face {face_id} differs, so the index width disagreed"
        );
    }
}

/// A square grid of `n` by `n` points with two triangles per cell.
#[cfg(feature = "legacy_bitstream_encode")]
fn grid_mesh(n: usize) -> Mesh {
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};

    let mut mesh = Mesh::new();
    let num_points = n * n;
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    for y in 0..n {
        for x in 0..n {
            let offset = (y * n + x) * 12;
            position
                .buffer_mut()
                .write(offset, &(x as f32).to_le_bytes());
            position
                .buffer_mut()
                .write(offset + 4, &(y as f32).to_le_bytes());
            position
                .buffer_mut()
                .write(offset + 8, &(((x + y) % 5) as f32).to_le_bytes());
        }
    }
    mesh.set_num_points(num_points);
    mesh.add_attribute(position);

    mesh.set_num_faces((n - 1) * (n - 1) * 2);
    let mut face_id = 0;
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let p0 = (y * n + x) as u32;
            let p1 = (y * n + x + 1) as u32;
            let p2 = ((y + 1) * n + x) as u32;
            let p3 = ((y + 1) * n + x + 1) as u32;
            mesh.set_face_from_indices(face_id, [p0, p1, p2]);
            face_id += 1;
            mesh.set_face_from_indices(face_id, [p1, p3, p2]);
            face_id += 1;
        }
    }
    mesh
}
