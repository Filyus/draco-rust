//! Example demonstrating the unified Writer trait interface.
//!
//! This example shows how all writers implement a common trait,
//! allowing for polymorphic usage and consistent API.

use draco_io::{FbxWriter, GltfWriter, ObjWriter, PlyWriter, Writer};
use std::io;

fn main() -> io::Result<()> {
    // Create a simple test mesh
    let mesh = create_test_mesh();

    println!("=== Unified Writer Trait Demo ===\n");

    // All writers implement the same trait
    write_with_trait(ObjWriter::new(), &mesh, "output.obj", "OBJ")?;
    write_with_trait(PlyWriter::new(), &mesh, "output.ply", "PLY")?;
    write_with_trait(FbxWriter::new(), &mesh, "output.fbx", "FBX")?;
    write_with_trait(GltfWriter::new(), &mesh, "output.glb", "glTF/GLB")?;

    println!("\n=== Format-Specific Features ===\n");

    // OBJ with named groups - using trait
    let mut obj = ObjWriter::new();
    Writer::add_mesh(&mut obj, &mesh, Some("Triangle"))?;
    obj.write("named_output.obj")?;
    println!("✓ OBJ with named group written");

    // PLY with point cloud
    let mut ply = PlyWriter::new();
    ply.add_points(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
    ply.write("points.ply")?;
    println!("✓ PLY point cloud written");

    // FBX with compression - using trait
    #[cfg(feature = "compression")]
    {
        let mut fbx = FbxWriter::new().with_compression(true);
        Writer::add_mesh(&mut fbx, &mesh, Some("CompressedMesh"))?;
        fbx.write("compressed.fbx")?;
        println!("✓ FBX with compression written");
    }

    // glTF with default quantization (using simplified API)
    let mut gltf = GltfWriter::new();
    gltf.add_draco_mesh(&mesh, Some("HighQuality"), None) // None = use defaults
        .map_err(io::Error::other)?;
    gltf.write_glb("high_quality.glb")
        .map_err(io::Error::other)?;
    gltf.write_gltf_embedded("embedded.gltf")
        .map_err(io::Error::other)?;
    println!("✓ glTF with Draco compression written");

    println!("\n=== All formats successfully written! ===");
    Ok(())
}

/// Generic function that works with any Writer implementation.
fn write_with_trait<W: Writer>(
    mut writer: W,
    mesh: &draco_core::mesh::Mesh,
    filename: &str,
    format_name: &str,
) -> io::Result<()> {
    writer.add_mesh(mesh, Some("TestMesh"))?;
    println!("Format: {}", format_name);
    println!("  Vertices: {}", writer.vertex_count());
    println!("  Faces: {}", writer.face_count());
    writer.write(filename)?;
    println!("  ✓ Written to {}\n", filename);
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
        3,
    );
    let buffer = pos_att.buffer_mut();
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 1.0, 0.0]];
    for (i, pos) in positions.iter().enumerate() {
        let bytes: Vec<u8> = pos.iter().flat_map(|v| v.to_le_bytes()).collect();
        buffer.write(i * 12, &bytes);
    }
    mesh.add_attribute(pos_att);

    mesh.set_num_faces(1);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);

    mesh
}
