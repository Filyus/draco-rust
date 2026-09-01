use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

/// Everything a decode produced, flattened, so two decodes can be compared as
/// values rather than field by field.
fn decoded_bytes(mesh: &Mesh) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend((mesh.num_points() as u64).to_le_bytes());
    out.extend((mesh.num_faces() as u64).to_le_bytes());
    for f in 0..mesh.num_faces() {
        for corner in mesh.face(FaceIndex(f as u32)) {
            out.extend(corner.0.to_le_bytes());
        }
    }
    out.extend((mesh.num_attributes() as u32).to_le_bytes());
    for id in 0..mesh.num_attributes() {
        let att = mesh.attribute(id);
        out.push(att.attribute_type() as u8);
        out.push(att.num_components());
        out.push(att.data_type() as u8);
        // The values, through the point map, so a reused attribute whose
        // buffer is longer than this decode needs contributes only what this
        // decode addressed.
        let stride = att.byte_stride() as usize;
        let mut value = vec![0u8; stride];
        for p in 0..mesh.num_points() {
            let index = att.mapped_index(PointIndex(p as u32));
            att.buffer().read(index.0 as usize * stride, &mut value);
            out.extend_from_slice(&value);
        }
    }
    out
}

#[test]
fn a_reused_decoder_and_mesh_decode_what_fresh_ones_do() {
    // `decode` takes `&mut Mesh`, so a caller decoding many files can hand it
    // the same mesh every time, and the mesh it gets back has to be the one a
    // fresh decode would have produced. It was not: every stage appends, so a
    // second decode left the mesh carrying both streams' attributes while the
    // point and face counts stayed right -- which is what makes this the one
    // shape a count cannot catch, and why nothing else in the suite, all of
    // which decodes into a fresh mesh, saw it.
    //
    // The two streams differ in point count, attribute count and encoding
    // method, which is the combination the encoder's own reuse bug needed.
    let mut edgebreaker = EncoderOptions::new();
    edgebreaker.set_global_int("encoding_method", 1);
    edgebreaker.set_attribute_int(0, "quantization_bits", 11);
    edgebreaker.set_attribute_int(1, "quantization_bits", 10);
    let mut sequential = EncoderOptions::new();
    sequential.set_global_int("encoding_method", 0);
    sequential.set_attribute_int(0, "quantization_bits", 8);

    let first = encode(uv_quad(), &edgebreaker);
    let second = encode(triangle(), &sequential);

    let mut from_fresh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&second), &mut from_fresh)
        .expect("fresh decode");

    let mut reused = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder
        .decode(&mut DecoderBuffer::new(&first), &mut reused)
        .expect("first decode");
    decoder
        .decode(&mut DecoderBuffer::new(&second), &mut reused)
        .expect("second decode into the same mesh");

    assert_eq!(
        decoded_bytes(&from_fresh),
        decoded_bytes(&reused),
        "a reused decoder and mesh produced a different mesh than fresh ones"
    );
}

fn encode(mesh: Mesh, options: &EncoderOptions) -> Vec<u8> {
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    encoder.encode(options, &mut buffer).expect("encode");
    buffer.data().to_vec()
}

fn positions(mesh: &mut Mesh, values: &[[f32; 3]]) {
    let mut att = PointAttribute::new();
    att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        values.len(),
    );
    for (i, value) in values.iter().enumerate() {
        let bytes: Vec<u8> = value.iter().flat_map(|v| v.to_le_bytes()).collect();
        att.buffer_mut().write(i * 12, &bytes);
    }
    mesh.add_attribute(att);
}

fn uv_quad() -> Mesh {
    let mut mesh = Mesh::new();
    positions(
        &mut mesh,
        &[
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
    );
    let mut uv = PointAttribute::new();
    uv.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Float32,
        false,
        4,
    );
    for (i, value) in [[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
        .iter()
        .enumerate()
    {
        let bytes: Vec<u8> = value.iter().flat_map(|v| v.to_le_bytes()).collect();
        uv.buffer_mut().write(i * 8, &bytes);
    }
    mesh.add_attribute(uv);
    mesh.set_num_faces(2);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);
    mesh
}

fn triangle() -> Mesh {
    let mut mesh = Mesh::new();
    positions(
        &mut mesh,
        &[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 3.0, 1.0]],
    );
    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh
}

#[test]
fn test_mesh_encode_decode() {
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

    // Faces
    mesh.set_num_faces(2);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);

    // Encode
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    // Use default Edgebreaker encoding (C++ compatible)
    options.set_attribute_int(0, "quantization_bits", 10);

    let mut enc_buffer = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc_buffer);
    assert!(status.is_ok(), "Encoding failed: {:?}", status.err());

    // Decode
    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded_mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let status = decoder.decode(&mut dec_buffer, &mut decoded_mesh);
    assert!(status.is_ok(), "Decoding failed: {:?}", status.err());

    assert_eq!(decoded_mesh.num_faces(), 2);
    assert_eq!(decoded_mesh.num_points(), 4);

    // Note: Edgebreaker encoding reorders vertices based on traversal order,
    // so we cannot check exact face indices. Instead, we verify that:
    // 1. The mesh has the right number of faces and vertices
    // 2. The decoded positions match the original (within quantization error)

    // Check attributes
    let decoded_att = decoded_mesh.attribute(0);
    assert_eq!(
        decoded_att.attribute_type(),
        GeometryAttributeType::Position
    );

    // Helper to read position at index
    let read_pos = |idx: usize| -> [f32; 3] {
        let decoded_buffer = decoded_att.buffer();
        let mut bytes = [0u8; 12];
        decoded_buffer.read(idx * 12, &mut bytes);
        [
            f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        ]
    };

    // Collect all decoded positions
    let decoded_positions: Vec<[f32; 3]> = (0..num_points).map(read_pos).collect();

    // Check that all original positions exist in decoded (within tolerance)
    let original_positions: Vec<[f32; 3]> = (0..num_points)
        .map(|i| [positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2]])
        .collect();

    for orig in &original_positions {
        let found = decoded_positions.iter().any(|dec| {
            (dec[0] - orig[0]).abs() < 0.01
                && (dec[1] - orig[1]).abs() < 0.01
                && (dec[2] - orig[2]).abs() < 0.01
        });
        assert!(
            found,
            "Original position {:?} not found in decoded mesh",
            orig
        );
    }
}
