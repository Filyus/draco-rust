//! Test to verify Draco attribute IDs are correctly assigned and match glTF expectations.

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::PointIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

#[test]
fn test_attribute_id_order() {
    // Create a mesh with position, normal, and texcoord (in that order)
    let mut mesh = Mesh::new();

    // Add position attribute (should be ID 0)
    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        3,
    );
    let pos_id = mesh.add_attribute(pos_att);
    eprintln!("Position attribute ID: {}", pos_id);

    // Add normal attribute (should be ID 1)
    let mut norm_att = PointAttribute::new();
    norm_att.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        3,
    );
    let norm_id = mesh.add_attribute(norm_att);
    eprintln!("Normal attribute ID: {}", norm_id);

    // Add texcoord attribute (should be ID 2)
    let mut uv_att = PointAttribute::new();
    uv_att.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Float32,
        false,
        3,
    );
    let uv_id = mesh.add_attribute(uv_att);
    eprintln!("TexCoord attribute ID: {}", uv_id);

    // Add a face
    mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);

    // Verify IDs
    assert_eq!(pos_id, 0, "Position should be attribute 0");
    assert_eq!(norm_id, 1, "Normal should be attribute 1");
    assert_eq!(uv_id, 2, "TexCoord should be attribute 2");

    // Verify named lookups
    assert_eq!(mesh.named_attribute_id(GeometryAttributeType::Position), 0);
    assert_eq!(mesh.named_attribute_id(GeometryAttributeType::Normal), 1);
    assert_eq!(mesh.named_attribute_id(GeometryAttributeType::TexCoord), 2);

    eprintln!("All attribute IDs match expected values!");
}

#[test]
fn test_decoded_attribute_order_matches() {
    // Create mesh with specific positions
    let mut mesh = Mesh::new();

    // Position: vertices at (0,0,0), (1,0,0), (0,1,0)
    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        3,
    );
    {
        let buf = pos_att.buffer_mut();
        buf.write(
            0,
            &[0.0f32, 0.0, 0.0]
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        buf.write(
            12,
            &[1.0f32, 0.0, 0.0]
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        buf.write(
            24,
            &[0.0f32, 1.0, 0.0]
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<_>>(),
        );
    }
    mesh.add_attribute(pos_att);

    // Normal: all pointing +Z
    let mut norm_att = PointAttribute::new();
    norm_att.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        3,
    );
    {
        let buf = norm_att.buffer_mut();
        for i in 0..3 {
            buf.write(
                i * 12,
                &[0.0f32, 0.0, 1.0]
                    .iter()
                    .flat_map(|f| f.to_le_bytes())
                    .collect::<Vec<_>>(),
            );
        }
    }
    mesh.add_attribute(norm_att);

    // TexCoord: (0,0), (1,0), (0,1)
    let mut uv_att = PointAttribute::new();
    uv_att.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Float32,
        false,
        3,
    );
    {
        let buf = uv_att.buffer_mut();
        buf.write(
            0,
            &[0.0f32, 0.0]
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        buf.write(
            8,
            &[1.0f32, 0.0]
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        buf.write(
            16,
            &[0.0f32, 1.0]
                .iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<_>>(),
        );
    }
    mesh.add_attribute(uv_att);

    mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);

    // Encode
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 0); // Sequential
    options.set_attribute_int(0, "quantization_bits", 14);
    options.set_attribute_int(1, "quantization_bits", 10);
    options.set_attribute_int(2, "quantization_bits", 12);

    let mut enc_buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut enc_buffer).unwrap();

    // Decode
    let mut dec_buffer = DecoderBuffer::new(enc_buffer.data());
    let mut decoded = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder.decode(&mut dec_buffer, &mut decoded).unwrap();

    // Check that decoded attribute types match expected order
    eprintln!("Decoded mesh has {} attributes", decoded.num_attributes());

    for i in 0..decoded.num_attributes() {
        let att = decoded.attribute(i);
        eprintln!(
            "Attribute {}: type={:?}, components={}, size={}",
            i,
            att.attribute_type(),
            att.num_components(),
            att.size()
        );
    }

    // Verify attribute order
    assert_eq!(
        decoded.attribute(0).attribute_type(),
        GeometryAttributeType::Position,
        "Attribute 0 should be Position"
    );
    assert_eq!(
        decoded.attribute(1).attribute_type(),
        GeometryAttributeType::Normal,
        "Attribute 1 should be Normal"
    );
    assert_eq!(
        decoded.attribute(2).attribute_type(),
        GeometryAttributeType::TexCoord,
        "Attribute 2 should be TexCoord"
    );

    eprintln!("Decoded attribute order matches encoded order!");
}
