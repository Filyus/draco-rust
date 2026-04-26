//! Example showing polymorphic usage of the Writer trait.
//!
//! This demonstrates how the Writer trait allows you to write
//! format-agnostic code that works with any supported output format.

use draco_io::{FbxWriter, GltfWriter, ObjWriter, PlyWriter, Writer};
use std::io;

fn main() -> io::Result<()> {
    let mesh = create_test_mesh();

    println!("=== Generic Function (Compile-time Polymorphism) ===\n");

    // Using generic function - resolved at compile time
    write_format(ObjWriter::new(), &mesh, "generic.obj", "OBJ")?;
    write_format(PlyWriter::new(), &mesh, "generic.ply", "PLY")?;
    write_format(FbxWriter::new(), &mesh, "generic.fbx", "FBX")?;
    write_format(GltfWriter::new(), &mesh, "generic.glb", "GLB")?;

    println!("\n=== Runtime Format Selection ===\n");

    // Choose format at runtime based on user input
    let format = "obj"; // Could come from args, config, etc.
    write_dynamic_format(&mesh, format)?;

    println!("\n✓ All formats written successfully!");
    Ok(())
}

// ============================================================================
// Generic Function - Compile-time Polymorphism
// ============================================================================

/// Write mesh using any Writer implementation (resolved at compile time)
fn write_format<W: Writer>(
    mut writer: W,
    mesh: &draco_core::mesh::Mesh,
    filename: &str,
    format_name: &str,
) -> io::Result<()> {
    writer.add_mesh(mesh, Some("Model"))?;
    println!("Format: {}", format_name);
    println!("  Vertices: {}", writer.vertex_count());
    println!("  Faces: {}", writer.face_count());
    writer.write(filename)?;
    println!("  ✓ Written to {}", filename);
    Ok(())
}

// ============================================================================
// Runtime Format Selection
// ============================================================================

/// Choose writer at runtime based on format string
fn write_dynamic_format(mesh: &draco_core::mesh::Mesh, format: &str) -> io::Result<()> {
    match format.to_lowercase().as_str() {
        "obj" => {
            let mut w = ObjWriter::new();
            w.add_mesh(mesh, Some("RuntimeModel"))?;
            w.write("dynamic.obj")?;
            println!("✓ Wrote OBJ dynamically");
        }
        "ply" => {
            let mut w = PlyWriter::new();
            w.add_mesh(mesh, Some("RuntimeModel"))?;
            w.write("dynamic.ply")?;
            println!("✓ Wrote PLY dynamically");
        }
        "fbx" => {
            let mut w = FbxWriter::new();
            w.add_mesh(mesh, Some("RuntimeModel"))?;
            w.write("dynamic.fbx")?;
            println!("✓ Wrote FBX dynamically");
        }
        "glb" | "gltf" => {
            let mut w = GltfWriter::new();
            w.add_mesh(mesh, Some("RuntimeModel"))?;
            w.write("dynamic.glb")?;
            println!("✓ Wrote GLB dynamically");
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unknown format",
            ))
        }
    }
    Ok(())
}

fn create_test_mesh() -> draco_core::mesh::Mesh {
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
    use draco_core::geometry_indices::{FaceIndex, PointIndex};
    use draco_core::mesh::Mesh;

    let mut mesh = Mesh::new();
    let mut pos_att = PointAttribute::new();

    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        4,
    );
    let buffer = pos_att.buffer_mut();

    // Create a quad
    let positions: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];

    for (i, pos) in positions.iter().enumerate() {
        let bytes: Vec<u8> = pos.iter().flat_map(|v| v.to_le_bytes()).collect();
        buffer.write(i * 12, &bytes);
    }
    mesh.add_attribute(pos_att);

    // Two triangles forming a quad
    mesh.set_num_faces(2);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);

    mesh
}
