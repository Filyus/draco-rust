//! Example creating a simple mesh and writing it to an FBX file using the writer.
//!
//! Run with: cargo run --example fbx_demo
//! Run with compression: cargo run --example fbx_demo --features compression

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_io::FbxReader;
use draco_io::{FbxWriter, Writer};

fn create_triangle_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    let mut pos_att = PointAttribute::new();

    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        3,
    );
    let buffer = pos_att.buffer_mut();
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    for (i, pos) in positions.iter().enumerate() {
        let bytes: Vec<u8> = pos.iter().flat_map(|v| v.to_le_bytes()).collect();
        buffer.write(i * 12, &bytes);
    }
    mesh.add_attribute(pos_att);

    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);

    mesh
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating triangle mesh...");
    let mesh = create_triangle_mesh();
    println!("  Vertices: {}", mesh.num_points());
    println!("  Faces: {}", mesh.num_faces());

    let out_dir = std::path::Path::new("output");
    std::fs::create_dir_all(out_dir)?;

    // Write without compression using FbxWriter
    let fbx_path = out_dir.join("triangle.fbx");
    println!("Writing FBX to {}...", fbx_path.display());
    let mut writer = FbxWriter::new();
    writer.add_mesh(&mesh, Some("Triangle"))?;
    writer.write(&fbx_path)?;
    let size_uncompressed = std::fs::metadata(&fbx_path)?.len();
    println!("  File size: {} bytes", size_uncompressed);

    // Write with compression (if feature enabled)
    #[cfg(feature = "compression")]
    {
        let fbx_compressed_path = out_dir.join("triangle_compressed.fbx");
        println!(
            "Writing compressed FBX to {}...",
            fbx_compressed_path.display()
        );
        let mut writer = FbxWriter::new()
            .with_compression(true)
            .with_compression_threshold(0);
        writer.add_mesh(&mesh, Some("TriangleCompressed"))?;
        writer.write(&fbx_compressed_path)?;
        let size_compressed = std::fs::metadata(&fbx_compressed_path)?.len();
        println!("  File size: {} bytes (compressed)", size_compressed);
    }

    // Read it back
    println!("Reading back using FbxReader...");
    let mut reader = FbxReader::open(fbx_path.to_str().unwrap())?;
    let meshes = reader.read_meshes()?;
    println!("  Found {} mesh(es)", meshes.len());
    if let Some(m) = meshes.first() {
        println!(
            "  Read mesh: vertices={}, faces={}",
            m.num_points(),
            m.num_faces()
        );
    }

    // Read compressed file back (if feature enabled)
    #[cfg(feature = "compression")]
    {
        let fbx_compressed_path = out_dir.join("triangle_compressed.fbx");
        println!("Reading compressed FBX back...");
        let mut reader = FbxReader::open(fbx_compressed_path.to_str().unwrap())?;
        let meshes = reader.read_meshes()?;
        println!("  Found {} mesh(es) from compressed file", meshes.len());
        if let Some(m) = meshes.first() {
            println!(
                "  Read mesh: vertices={}, faces={}",
                m.num_points(),
                m.num_faces()
            );
        }
    }

    println!("✓ FBX write & read successful");
    Ok(())
}
