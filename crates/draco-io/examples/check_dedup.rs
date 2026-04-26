use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::geometry_indices::PointIndex;
use draco_io::GltfReader;
use std::collections::HashMap;
use std::path::Path;

fn main() {
    let test_file = Path::new(r"D:\Projects\Draco\testdata\IridescenceLamp.glb");

    let reader = GltfReader::open(test_file).expect("Failed to open GLB");
    let meshes = reader.decode_all_meshes().expect("Failed to decode meshes");

    if let Some(mesh) = meshes.first() {
        println!(
            "Mesh has {} faces, {} points, {} attrs",
            mesh.num_faces(),
            mesh.num_points(),
            mesh.num_attributes()
        );

        let pos_id = mesh.named_attribute_id(GeometryAttributeType::Position);
        if pos_id >= 0 {
            let pos_att = mesh.attribute(pos_id);
            println!(
                "Position attribute: {} unique values, {} components",
                pos_att.size(),
                pos_att.num_components()
            );

            // Check for duplicate position values by reading raw bytes
            let buffer = pos_att.buffer();
            let stride = (pos_att.num_components() as usize) * 4; // f32 = 4 bytes

            let mut value_to_points: HashMap<Vec<u8>, Vec<u32>> = HashMap::new();

            for i in 0..mesh.num_points() {
                let pt = PointIndex(i as u32);
                let val_idx = pos_att.mapped_index(pt);

                // Read raw bytes for this position value
                let offset = (val_idx.0 as usize) * stride;
                let bytes = buffer.data()[offset..offset + stride].to_vec();

                value_to_points.entry(bytes).or_default().push(i as u32);
            }

            let unique_positions = value_to_points.len();
            let duplicates = value_to_points
                .iter()
                .filter(|(_, pts)| pts.len() > 1)
                .count();

            println!("Unique positions: {}", unique_positions);
            println!("Position values with duplicates: {}", duplicates);

            if duplicates > 0 {
                println!("\nFirst 10 duplicated positions:");
                for (i, (bytes, pts)) in value_to_points
                    .iter()
                    .filter(|(_, pts)| pts.len() > 1)
                    .take(10)
                    .enumerate()
                {
                    // Parse as f32
                    let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                    let z = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
                    println!(
                        "  {}: ({:.6}, {:.6}, {:.6}) used by {} points: {:?}",
                        i,
                        x,
                        y,
                        z,
                        pts.len(),
                        &pts[..pts.len().min(5)]
                    );
                }
            }
        }
    }
}
