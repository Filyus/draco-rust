use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

fn make_unit_quad_mesh() -> (Mesh, [f32; 12]) {
    let mut mesh = Mesh::new();
    let mut pos_att = PointAttribute::new();

    let num_points = 4;
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );

    // A simple quad (2 triangles)
    // 0 --- 1
    // | \   |
    // |  \  |
    // 3 --- 2
    let positions: [f32; 12] = [
        0.0, 0.0, 0.0, // 0
        1.0, 0.0, 0.0, // 1
        1.0, 1.0, 0.0, // 2
        0.0, 1.0, 0.0, // 3
    ];

    let buffer = pos_att.buffer_mut();
    for i in 0..num_points {
        let bytes = [
            positions[i * 3].to_le_bytes(),
            positions[i * 3 + 1].to_le_bytes(),
            positions[i * 3 + 2].to_le_bytes(),
        ]
        .concat();
        buffer.write(i * 12, &bytes);
    }

    mesh.add_attribute(pos_att);

    mesh.set_num_faces(2);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);

    (mesh, positions)
}

fn encode_decode_with_options(mesh: Mesh, options: &EncoderOptions) -> Mesh {
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut enc_buffer = EncoderBuffer::new();
    let status = encoder.encode(options, &mut enc_buffer);
    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());

    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let status = decoder.decode(&mut dec_buffer, &mut decoded_mesh);
    assert!(status.is_ok(), "Decoding failed: {:?}", status.err());

    decoded_mesh
}

fn assert_quad_mesh(decoded_mesh: &Mesh, positions: &[f32; 12]) {
    assert_eq!(decoded_mesh.num_faces(), 2);
    assert_eq!(decoded_mesh.num_points(), 4);

    let f0 = decoded_mesh.face(FaceIndex(0));
    assert_eq!(f0[0], PointIndex(0));
    assert_eq!(f0[1], PointIndex(1));
    assert_eq!(f0[2], PointIndex(2));

    let f1 = decoded_mesh.face(FaceIndex(1));
    assert_eq!(f1[0], PointIndex(0));
    assert_eq!(f1[1], PointIndex(2));
    assert_eq!(f1[2], PointIndex(3));

    let decoded_att = decoded_mesh.attribute(0);
    assert_eq!(
        decoded_att.attribute_type(),
        GeometryAttributeType::Position
    );

    let decoded_buffer = decoded_att.buffer();
    for i in 0..4 {
        let mut bytes = [0u8; 12];
        decoded_buffer.read(i * 12, &mut bytes);

        let x = f32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let y = f32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let z = f32::from_le_bytes(bytes[8..12].try_into().unwrap());

        let ex = positions[i * 3];
        let ey = positions[i * 3 + 1];
        let ez = positions[i * 3 + 2];

        assert!((x - ex).abs() < 0.01);
        assert!((y - ey).abs() < 0.01);
        assert!((z - ez).abs() < 0.01);
    }
}

fn assert_quad_mesh_edgebreaker(decoded_mesh: &Mesh, positions: &[f32; 12]) {
    assert_eq!(decoded_mesh.num_faces(), 2);
    assert_eq!(decoded_mesh.num_points(), 4);

    // Topology check (edgebreaker may permute vertex indices).
    let mut faces: Vec<Vec<u32>> = (0..2)
        .map(|i| {
            let f = decoded_mesh.face(FaceIndex(i));
            let mut v = vec![f[0].0, f[1].0, f[2].0];
            v.sort();
            v
        })
        .collect();
    faces.sort();

    let f0 = &faces[0];
    let f1 = &faces[1];
    let shared: Vec<_> = f0.iter().filter(|v| f1.contains(v)).collect();
    assert_eq!(shared.len(), 2);

    let mut uniq = f0.clone();
    uniq.extend_from_slice(f1);
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 4);

    // Attribute values: match positions as an unordered set.
    let decoded_att = decoded_mesh.attribute(0);
    assert_eq!(
        decoded_att.attribute_type(),
        GeometryAttributeType::Position
    );

    let decoded_buffer = decoded_att.buffer();
    let mut decoded_positions: Vec<[f32; 3]> = Vec::with_capacity(4);
    for i in 0..4 {
        let mut bytes = [0u8; 12];
        decoded_buffer.read(i * 12, &mut bytes);
        decoded_positions.push([
            f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        ]);
    }

    let expected_positions: Vec<[f32; 3]> = (0..4)
        .map(|i| [positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2]])
        .collect();

    let eps = 0.01f32;
    let mut used = vec![false; decoded_positions.len()];
    for exp in expected_positions {
        let mut found = false;
        for (idx, got) in decoded_positions.iter().enumerate() {
            if used[idx] {
                continue;
            }
            if (got[0] - exp[0]).abs() < eps
                && (got[1] - exp[1]).abs() < eps
                && (got[2] - exp[2]).abs() < eps
            {
                used[idx] = true;
                found = true;
                break;
            }
        }
        assert!(found, "Expected position {:?} not found", exp);
    }
}

#[test]
fn test_mesh_encode_decode_constrained_multi_parallelogram_sequential() {
    let (mesh, positions) = make_unit_quad_mesh();

    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 10);
    options.set_global_int("encoding_method", 0); // Sequential

    // Force Constrained Multi-Parallelogram prediction (4)
    options.set_prediction_scheme(4);

    let decoded_mesh = encode_decode_with_options(mesh, &options);
    assert_quad_mesh(&decoded_mesh, &positions);
}

#[test]
fn test_mesh_encode_decode_constrained_multi_parallelogram_edgebreaker() {
    let (mesh, positions) = make_unit_quad_mesh();

    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 10);
    options.set_global_int("encoding_method", 1); // Edgebreaker

    // Force Constrained Multi-Parallelogram prediction (4)
    options.set_prediction_scheme(4);

    let decoded_mesh = encode_decode_with_options(mesh, &options);
    assert_quad_mesh_edgebreaker(&decoded_mesh, &positions);
}
