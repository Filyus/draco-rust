//! Example demonstrating the unified Reader trait interface.

use draco_io::{ObjReader, ObjWriter, PlyReader, PlyWriter, Reader, Writer};
use std::io;

fn main() -> io::Result<()> {
    // Create a test mesh and write it in multiple formats
    let mesh = create_test_mesh();

    println!("=== Writing test files ===\n");

    let mut obj_writer = ObjWriter::new();
    obj_writer.add_mesh(&mesh, Some("TestMesh"))?;
    obj_writer.write("test_unified.obj")?;
    println!("✓ Wrote test_unified.obj");

    let mut ply_writer = PlyWriter::new();
    ply_writer.add_mesh(&mesh, None)?;
    ply_writer.write("test_unified.ply")?;
    println!("✓ Wrote test_unified.ply\n");

    println!("=== Reading with unified Reader trait ===\n");

    // Generic function works with any Reader
    load_and_display::<ObjReader>("test_unified.obj", "OBJ")?;
    load_and_display::<PlyReader>("test_unified.ply", "PLY")?;

    println!("\n=== Point cloud reading ===\n");

    // Using PointCloudReader trait
    use draco_io::PointCloudReader;

    let mut obj_reader = ObjReader::open("test_unified.obj")?;
    let points = obj_reader.read_points()?;
    println!("OBJ: Read {} points", points.len());

    let mut ply_reader = PlyReader::open("test_unified.ply")?;
    let points = ply_reader.read_points()?;
    println!("PLY: Read {} points", points.len());

    println!("\n✓ All readers work consistently!");
    Ok(())
}

/// Generic function that works with any Reader implementation
fn load_and_display<R: Reader>(path: &str, format_name: &str) -> io::Result<()> {
    let mut reader = R::open(path)?;
    let mesh = reader.read_mesh()?;

    println!("Format: {}", format_name);
    println!("  Points: {}", mesh.num_points());
    println!("  Attributes: {}", mesh.num_attributes());

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

    mesh.set_num_faces(2);
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);

    mesh
}
