//! Example demonstrating glTF/GLB I/O with Draco compression.
//!
//! Run with: cargo run --example gltf_demo

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_io::gltf_reader::GltfReader;
use draco_io::gltf_writer::GltfWriter;

fn create_cube_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    let mut pos_att = PointAttribute::new();

    // 8 vertices of a cube
    let positions: [f32; 24] = [
        -1.0, -1.0, -1.0, // 0
        1.0, -1.0, -1.0, // 1
        1.0, 1.0, -1.0, // 2
        -1.0, 1.0, -1.0, // 3
        -1.0, -1.0, 1.0, // 4
        1.0, -1.0, 1.0, // 5
        1.0, 1.0, 1.0, // 6
        -1.0, 1.0, 1.0, // 7
    ];

    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        8,
    );

    let buffer = pos_att.buffer_mut();
    for i in 0..8 {
        let bytes = [
            positions[i * 3].to_le_bytes(),
            positions[i * 3 + 1].to_le_bytes(),
            positions[i * 3 + 2].to_le_bytes(),
        ]
        .concat();
        buffer.write(i * 12, &bytes);
    }

    mesh.add_attribute(pos_att);

    // 12 triangles (6 faces * 2 triangles each)
    let faces = [
        [0, 1, 2],
        [0, 2, 3], // Front
        [1, 5, 6],
        [1, 6, 2], // Right
        [5, 4, 7],
        [5, 7, 6], // Back
        [4, 0, 3],
        [4, 3, 7], // Left
        [3, 2, 6],
        [3, 6, 7], // Top
        [4, 5, 1],
        [4, 1, 0], // Bottom
    ];

    mesh.set_num_faces(12);
    for (i, face) in faces.iter().enumerate() {
        mesh.set_face(
            FaceIndex(i as u32),
            [
                PointIndex(face[0]),
                PointIndex(face[1]),
                PointIndex(face[2]),
            ],
        );
    }

    mesh
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating a cube mesh...");
    let mesh = create_cube_mesh();
    println!("  Vertices: {}", mesh.num_points());
    println!("  Faces: {}", mesh.num_faces());

    let mut writer = GltfWriter::new();
    // Use None for default quantization settings
    writer.add_draco_mesh(&mesh, Some("Cube"), None)?;

    // Write in all three formats to 'output' directory
    println!("\nWriting glTF files with Draco compression:");
    let out_dir = std::path::Path::new("output");
    std::fs::create_dir_all(out_dir)?;

    // 1. GLB (binary, most compact) - write to file
    let glb_path = out_dir.join("cube.glb");
    writer.write_glb(glb_path.to_str().unwrap())?;
    let glb_size = std::fs::metadata(&glb_path)?.len();
    println!(
        "  1. GLB format: {} ({} bytes) ✓",
        glb_path.display(),
        glb_size
    );

    // 2. glTF + .bin (separate files)
    let gltf_path = out_dir.join("cube.gltf");
    let bin_path = out_dir.join("cube.bin");
    writer.write_gltf(gltf_path.to_str().unwrap(), bin_path.to_str().unwrap())?;
    let json_size = std::fs::metadata(&gltf_path)?.len();
    let bin_size = std::fs::metadata(&bin_path)?.len();
    println!(
        "  2. glTF + .bin: {} ({} bytes) + {} ({} bytes) ✓",
        gltf_path.display(),
        json_size,
        bin_path.display(),
        bin_size
    );

    // 3. glTF embedded (pure text, no external files)
    let embedded_path = out_dir.join("cube_embedded.gltf");
    writer.write_gltf_embedded(embedded_path.to_str().unwrap())?;
    let embedded_size = std::fs::metadata(&embedded_path)?.len();
    println!(
        "  3. glTF embedded: {} ({} bytes, base64-encoded) ✓",
        embedded_path.display(),
        embedded_size
    );

    // Demonstrate reading back from written file
    println!("\nReading back {}...", glb_path.display());
    let reader = GltfReader::open(glb_path.to_str().unwrap())?;

    println!("  Has Draco extension: {}", reader.has_draco_extension());
    println!("  Number of meshes: {}", reader.num_meshes());

    let primitives = reader.draco_primitives();
    for (i, info) in primitives.iter().enumerate() {
        println!(
            "  Primitive {}: '{}'",
            i,
            info.mesh_name.as_deref().unwrap_or("<unnamed>")
        );
    }

    // Decode the mesh
    if let Some(info) = primitives.first() {
        let decoded = reader.decode_draco_mesh(info)?;
        println!("\nDecoded mesh:");
        println!("  Vertices: {}", decoded.num_points());
        println!("  Faces: {}", decoded.num_faces());
        println!("  Attributes: {}", decoded.num_attributes());
    }

    // Demonstrate embedded format from file
    println!(
        "\nTesting embedded glTF format at {}...",
        embedded_path.display()
    );
    let reader2 = GltfReader::open(embedded_path.to_str().unwrap())?;
    println!("  Successfully parsed {}", embedded_path.display());
    println!("  Can decode: {}", reader2.has_draco_extension());

    println!("\n✓ All formats work correctly!");
    println!("\nCreated files in {}:", out_dir.display()); // directory: output
    println!("  - {} (GLB with Draco embedded)", glb_path.display());
    println!(
        "  - {} + {} (glTF with external binary)",
        gltf_path.display(),
        bin_path.display()
    );
    println!("  - {} (pure text with base64)", embedded_path.display());

    Ok(())
}
