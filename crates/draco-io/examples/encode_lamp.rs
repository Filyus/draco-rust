use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::mesh_encoder::MeshEncoder;
use draco_io::GltfReader;
use std::path::Path;

fn main() {
    let test_file = Path::new(r"D:\Projects\Draco\testdata\IridescenceLamp.glb");

    let reader = GltfReader::open(test_file).expect("Failed to open GLB");
    let meshes = reader.decode_all_meshes().expect("Failed to decode meshes");

    if let Some(first_mesh) = meshes.first() {
        println!(
            "Encoding mesh with {} faces, {} points, {} attrs",
            first_mesh.num_faces(),
            first_mesh.num_points(),
            first_mesh.num_attributes()
        );

        // Print first few positions
        if let Some(pos_attr) = first_mesh.named_attribute(GeometryAttributeType::Position) {
            println!("First 5 original positions (from buffer):");
            let buffer = pos_attr.buffer();
            let stride = pos_attr.byte_stride() as usize;
            for i in 0..5usize.min(pos_attr.size()) {
                let offset = i * stride;
                let bytes = &buffer.data()[offset..offset + 12];
                let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                let z = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
                println!("  orig_point[{}] = ({:.6}, {:.6}, {:.6})", i, x, y, z);
            }
        }

        for i in 0..first_mesh.num_attributes() {
            let attr = first_mesh.attribute(i);
            println!(
                "  Attr {}: {:?}, components={}, data_type={:?}",
                i,
                attr.attribute_type(),
                attr.num_components(),
                attr.data_type()
            );
        }

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(first_mesh.clone());

        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_method", 1); // Edgebreaker

        // Set quantization to match C++ defaults (compression level 7)
        // Position: 11 bits, TexCoord: 10 bits, Normal: 8 bits
        let pos_id = first_mesh.named_attribute_id(GeometryAttributeType::Position);
        let norm_id = first_mesh.named_attribute_id(GeometryAttributeType::Normal);
        let uv_id = first_mesh.named_attribute_id(GeometryAttributeType::TexCoord);

        if pos_id != -1 {
            options.set_attribute_int(pos_id, "quantization_bits", 11);
        }
        if norm_id != -1 {
            options.set_attribute_int(norm_id, "quantization_bits", 8);
        }
        if uv_id != -1 {
            options.set_attribute_int(uv_id, "quantization_bits", 10);
        }

        let mut enc_buffer = EncoderBuffer::new();
        encoder
            .encode(&options, &mut enc_buffer)
            .expect("Encoding failed");

        let output_path = Path::new(r"D:\Projects\Draco\output\lamp_rust_single.drc");
        std::fs::write(output_path, enc_buffer.data()).expect("Failed to write");
        println!(
            "Saved to {:?} ({} bytes)",
            output_path,
            enc_buffer.data().len()
        );
    }
}
