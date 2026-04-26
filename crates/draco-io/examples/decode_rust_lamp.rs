use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let default_path = r"D:\Projects\Draco\output\lamp_rust_single.drc".to_string();
    let test_file = Path::new(args.get(1).unwrap_or(&default_path));

    let data = std::fs::read(test_file).expect("Failed to read file");
    let mut decoder = MeshDecoder::new();
    let mut buffer = DecoderBuffer::new(&data);
    let mut mesh = Mesh::new();

    decoder
        .decode(&mut buffer, &mut mesh)
        .expect("Failed to decode");

    println!(
        "Decoded mesh: {} faces, {} points, {} attrs",
        mesh.num_faces(),
        mesh.num_points(),
        mesh.num_attributes()
    );

    // Print first 5 faces
    println!("First 5 faces:");
    for f in 0..5.min(mesh.num_faces()) {
        let face = mesh.face(draco_core::geometry_indices::FaceIndex(f as u32));
        println!(
            "  face[{}] = [{}, {}, {}]",
            f, face[0].0, face[1].0, face[2].0
        );
    }

    // Find position attribute and compute bounds
    let pos_id = mesh.named_attribute_id(GeometryAttributeType::Position);
    if pos_id >= 0 {
        let pos_attr = mesh.attribute(pos_id);
        let buffer = pos_attr.buffer();
        let num_components = pos_attr.num_components() as usize;
        let byte_stride = pos_attr.byte_stride() as usize;

        let mut min_vals = [f32::MAX; 3];
        let mut max_vals = [f32::MIN; 3];

        // Print first 20 vertices
        println!("First 20 vertex positions:");
        for i in 0..20.min(pos_attr.size()) {
            let offset = i * byte_stride;
            let mut vals = [0.0f32; 3];
            for (c, val) in vals.iter_mut().enumerate().take(num_components.min(3)) {
                let bytes: [u8; 4] = buffer.data()[offset + c * 4..offset + c * 4 + 4]
                    .try_into()
                    .unwrap();
                *val = f32::from_le_bytes(bytes);
            }
            println!(
                "  v[{}] = ({:.6}, {:.6}, {:.6})",
                i, vals[0], vals[1], vals[2]
            );
        }

        for i in 0..pos_attr.size() {
            let offset = i * byte_stride;
            for c in 0..num_components.min(3) {
                let bytes: [u8; 4] = buffer.data()[offset + c * 4..offset + c * 4 + 4]
                    .try_into()
                    .unwrap();
                let val = f32::from_le_bytes(bytes);
                if val < min_vals[c] {
                    min_vals[c] = val;
                }
                if val > max_vals[c] {
                    max_vals[c] = val;
                }
            }
        }

        println!("Position bounds:");
        println!("  X: {} to {}", min_vals[0], max_vals[0]);
        println!("  Y: {} to {}", min_vals[1], max_vals[1]);
        println!("  Z: {} to {}", min_vals[2], max_vals[2]);
    }
}
