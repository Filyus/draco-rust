use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::PointIndex;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::{DecoderBuffer, Mesh, MeshDecoder};
use std::fs;

#[test]
fn test_decode_rust_encoded() {
    let mut source_mesh = Mesh::new();

    let mut pos_attr = PointAttribute::new();
    pos_attr.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        3,
    );
    for (i, chunk) in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        .iter()
        .enumerate()
    {
        let bytes: Vec<u8> = chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
        pos_attr.buffer_mut().write(i * 12, &bytes);
    }
    source_mesh.add_attribute(pos_attr);

    source_mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(source_mesh);

    let mut options = EncoderOptions::default();
    options.set_attribute_int(0, "quantization_bits", 14);

    let mut encoded = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoded)
        .expect("Failed to encode Rust mesh");

    let data = encoded.data();

    println!("File size: {} bytes", data.len());
    println!("First 30 bytes: {:?}", &data[..30.min(data.len())]);

    let mut buffer = DecoderBuffer::new(&data);
    let mut decoder = MeshDecoder::new();
    let mut mesh = Mesh::new();

    decoder
        .decode(&mut buffer, &mut mesh)
        .expect("Failed to decode Rust-encoded mesh");

    println!("Decoded successfully!");
    println!("  Num faces: {}", mesh.num_faces());
    println!("  Num points: {}", mesh.num_points());
    println!("  Num attributes: {}", mesh.num_attributes());

    for i in 0..mesh.num_attributes() {
        let att = mesh.attribute(i);
        println!(
            "  Attribute {}: type={:?}, components={}",
            i,
            att.attribute_type(),
            att.num_components()
        );
    }

    assert_eq!(mesh.num_faces(), 1);
    assert_eq!(mesh.num_points(), 3);
    assert!(mesh
        .named_attribute(GeometryAttributeType::Position)
        .is_some());
}

#[test]
fn test_decode_cpp_encoded() {
    // Read the C++ standard edgebreaker encoded file
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/lamp_cpp_std.drc");
    let data = fs::read(path).expect("Failed to read file");

    println!("File size: {} bytes", data.len());
    println!("First 30 bytes: {:?}", &data[..30.min(data.len())]);

    let mut buffer = DecoderBuffer::new(&data);
    let mut decoder = MeshDecoder::new();
    let mut mesh = Mesh::new();

    match decoder.decode(&mut buffer, &mut mesh) {
        Ok(()) => {
            println!("Decoded successfully!");
            println!("  Num faces: {}", mesh.num_faces());
            println!("  Num points: {}", mesh.num_points());
            println!("  Num attributes: {}", mesh.num_attributes());

            for i in 0..mesh.num_attributes() {
                let att = mesh.attribute(i);
                println!(
                    "  Attribute {}: type={:?}, components={}",
                    i,
                    att.attribute_type(),
                    att.num_components()
                );
            }
        }
        Err(e) => {
            println!("Decode FAILED: {:?}", e);
        }
    }
}
