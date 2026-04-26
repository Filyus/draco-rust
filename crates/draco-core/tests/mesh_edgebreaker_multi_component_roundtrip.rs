// Test uses C++ style array indexing patterns and explicit vec construction for clarity.
// needless_range_loop: for i in 0..n { arr[i] } mirrors C++ test structure
// useless_vec: vec![...] makes test data initialization explicit and easy to modify
#![allow(clippy::needless_range_loop, clippy::useless_vec)]

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::DataType;
use draco_core::EncoderOptions;
use draco_core::{GeometryAttributeType, PointAttribute};

#[test]
fn test_edgebreaker_multi_component_roundtrip() {
    // Create a mesh with two disconnected triangles and shuffled vertex indices.
    let mut mesh = Mesh::new();
    mesh.set_num_points(6);
    mesh.set_num_faces(2);

    // Triangle 1: vertices 0, 1, 2
    // Triangle 2: vertices 3, 4, 5
    // We'll define them in a way that the traversal order is different from the vertex index order.
    mesh.set_face(FaceIndex(0), [PointIndex(2), PointIndex(0), PointIndex(1)]);
    mesh.set_face(FaceIndex(1), [PointIndex(5), PointIndex(3), PointIndex(4)]);

    // Add an attribute to verify reordering.
    let mut attr = PointAttribute::new();
    attr.init(
        GeometryAttributeType::Position,
        1,
        DataType::Float32,
        false,
        6,
    );
    let mut data = vec![0.0f32; 6];
    for i in 0..6 {
        data[i] = i as f32;
    }
    // Convert f32 slice to u8 slice using bytemuck.
    let u8_data = bytemuck::cast_slice(&data);
    attr.buffer_mut().update(u8_data, None);
    mesh.add_attribute(attr);

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1); // Edgebreaker
    options.set_attribute_int(0, "quantization_bits", 14);
    // options.set_global_int("encoding_speed", 10); // Use Difference prediction

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut encoder_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoder_buffer)
        .expect("Encode failed");

    // Decode
    let mut decoder = MeshDecoder::new();
    let mut decoder_buffer = DecoderBuffer::new(encoder_buffer.data());
    let mut decoded_mesh = Mesh::new();
    decoder
        .decode(&mut decoder_buffer, &mut decoded_mesh)
        .expect("Decode failed");

    assert_eq!(decoded_mesh.num_faces(), 2);
    assert_eq!(decoded_mesh.num_points(), 6);

    // Verify faces. The decoder might have different vertex indices, but the topology should be the same.
    // In Edgebreaker, the first triangle will get indices [0, 1, 2] and the second [3, 4, 5].
    let f0 = decoded_mesh.face(FaceIndex(0));
    let f1 = decoded_mesh.face(FaceIndex(1));

    assert_eq!(f0, [PointIndex(0), PointIndex(1), PointIndex(2)]);
    assert_eq!(f1, [PointIndex(3), PointIndex(4), PointIndex(5)]);

    // Verify attribute values.
    // The Edgebreaker connectivity decoder can assign vertex ids in an order
    // that differs from the original face order (especially across multiple
    // disconnected components). The correctness condition here is that each
    // decoded triangle carries the same *set* of vertex attribute values as the
    // corresponding original component: {0,1,2} and {3,4,5}.

    let attr_decoded = decoded_mesh.attribute(0);
    let mut decoded_values = vec![0.0f32; 6];
    for i in 0..6 {
        let start = i * 4;
        let end = start + 4;
        let bytes = &attr_decoded.buffer().data()[start..end];
        let val: f32 = bytemuck::pod_read_unaligned(bytes);
        decoded_values[i] = val;
    }

    let to_i = |v: f32| v.round() as i32;
    let mut f0_vals = vec![
        to_i(decoded_values[0]),
        to_i(decoded_values[1]),
        to_i(decoded_values[2]),
    ];
    let mut f1_vals = vec![
        to_i(decoded_values[3]),
        to_i(decoded_values[4]),
        to_i(decoded_values[5]),
    ];
    f0_vals.sort_unstable();
    f1_vals.sort_unstable();

    let a = vec![0, 1, 2];
    let b = vec![3, 4, 5];
    assert!(
        (f0_vals == a && f1_vals == b) || (f0_vals == b && f1_vals == a),
        "Unexpected per-face attribute value sets. f0={:?} f1={:?}",
        f0_vals,
        f1_vals
    );
}
