//! The mesh writers, given geometry a decoder can legitimately produce.
//!
//! A `.drc` declares its own attribute data types and component counts, so
//! `MeshDecoder` hands back positions that are Uint8x3 or normals that are
//! Int16x3 as readily as Float32x3 ones. The writers read attribute values by
//! slicing a fixed number of bytes at `point * byte_stride`, which holds only
//! while the attribute is the type the reader assumes - so the decode-then-
//! write pipeline this crate exists for could panic inside `DataBuffer::read`.

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::mesh::Mesh;
use draco_io::traits::Writer;
use draco_io::{ObjWriter, PlyWriter, StlWriter};

/// A quad whose position attribute has the given scalar type, optionally with
/// no position attribute at all.
fn quad(position_type: Option<DataType>) -> Mesh {
    let mut mesh = Mesh::new();
    let num_points = 4;
    if let Some(position_type) = position_type {
        let mut position = PointAttribute::new();
        position.init(
            GeometryAttributeType::Position,
            3,
            position_type,
            false,
            num_points,
        );
        mesh.add_attribute(position);
    }
    let mut normal = PointAttribute::new();
    normal.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Int16,
        false,
        num_points,
    );
    mesh.add_attribute(normal);
    mesh.set_num_points(num_points);
    mesh.set_num_faces(2);
    mesh.set_face_from_indices(0, [0, 1, 2]);
    mesh.set_face_from_indices(1, [1, 3, 2]);
    mesh
}

#[test]
fn writers_do_not_panic_on_narrow_position_types() {
    for position_type in [
        DataType::Uint8,
        DataType::Int8,
        DataType::Int16,
        DataType::Uint16,
        DataType::Int32,
        DataType::Float32,
    ] {
        // Every writer either handles the type or refuses the mesh; none may
        // read past the attribute buffer.
        let mut stl = <StlWriter as Writer>::new();
        let _ = stl.add_mesh(&quad(Some(position_type)), None);

        let mut ply = <PlyWriter as Writer>::new();
        let _ = ply.add_mesh(&quad(Some(position_type)), None);

        let mut obj = <ObjWriter as Writer>::new();
        let _ = obj.add_mesh(&quad(Some(position_type)), None);
    }
}

#[test]
fn the_stl_writer_refuses_a_position_type_it_cannot_read() {
    let mut stl = <StlWriter as Writer>::new();
    let error = stl
        .add_mesh(&quad(Some(DataType::Uint8)), None)
        .expect_err("Uint8x3 positions must be refused, as the OBJ writer refuses them");
    assert!(
        error.to_string().contains("Float32x3"),
        "unexpected error: {error}"
    );
}

/// A quad with float32 positions, optionally carrying normals.
fn quad_with_normals(base: f32, normals: bool) -> Mesh {
    let mut mesh = Mesh::new();
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        4,
    );
    for i in 0..4 {
        for c in 0..3 {
            let value = base + i as f32 + c as f32 * 0.25;
            position
                .buffer_mut()
                .write(i * 12 + c * 4, &value.to_le_bytes());
        }
    }
    mesh.set_num_points(4);
    mesh.add_attribute(position);
    if normals {
        let mut normal = PointAttribute::new();
        normal.init(
            GeometryAttributeType::Normal,
            3,
            DataType::Float32,
            false,
            4,
        );
        for i in 0..4 {
            normal.buffer_mut().write(i * 12 + 8, &1.0f32.to_le_bytes());
        }
        mesh.add_attribute(normal);
    }
    mesh.set_num_faces(2);
    mesh.set_face_from_indices(0, [0, 1, 2]);
    mesh.set_face_from_indices(1, [1, 3, 2]);
    mesh
}

fn ply_of(meshes: &[Mesh]) -> String {
    let mut writer = <PlyWriter as Writer>::new();
    for mesh in meshes {
        writer.add_mesh(mesh, None).expect("add_mesh");
    }
    String::from_utf8_lossy(&PlyWriter::write_to_vec(&writer).expect("write")).into_owned()
}

#[test]
fn adding_a_mesh_without_normals_keeps_the_normals_of_the_one_before_it() {
    // The writer flattens every mesh into shared per-vertex lists and emits a
    // property only when its list is as long as the position list. The lists
    // were padded before a mesh's values were appended but not after, so a mesh
    // that did not carry the attribute left the list short and the property was
    // dropped for every mesh - silently, and only in this order.
    let with_then_without = ply_of(&[quad_with_normals(0.0, true), quad_with_normals(50.0, false)]);
    let without_then_with = ply_of(&[quad_with_normals(0.0, false), quad_with_normals(50.0, true)]);

    for (order, ply) in [
        ("normals first", &with_then_without),
        ("normals second", &without_then_with),
    ] {
        assert!(
            ply.contains("property float nx"),
            "{order}: the normals of one of the two meshes were dropped"
        );
        assert!(
            ply.contains("element vertex 8"),
            "{order}: expected both meshes' vertices"
        );
    }
    assert_eq!(
        with_then_without.len(),
        without_then_with.len(),
        "the same two meshes wrote different files depending on the order"
    );
}

#[test]
fn the_ply_writer_handles_a_mesh_with_no_position_attribute() {
    // The normal/color/texcoord padding measured how far behind the position
    // list they were, which underflows when nothing was appended to it.
    let mut ply = <PlyWriter as Writer>::new();
    let _ = ply.add_mesh(&quad(None), None);
}

/// A mesh whose attribute holds fewer values than it has points is refused,
/// not read past the end of.
///
/// Every writer here reads attribute data as `point * byte_stride` through the
/// panicking `DataBuffer::read`, which is sound only while the attribute is at
/// least as long as the point count. Nothing between a decoder and a writer
/// re-checks that: the counts come from a `.drc` header.
#[test]
fn writers_refuse_an_attribute_shorter_than_the_point_count() {
    // Four points, a position attribute holding two of them.
    let mut mesh = Mesh::new();
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        2,
    );
    mesh.add_attribute(position);
    mesh.set_num_points(4);
    mesh.set_num_faces(2);
    mesh.set_face_from_indices(0, [0, 1, 2]);
    mesh.set_face_from_indices(1, [1, 3, 2]);

    let obj = ObjWriter::new().add_mesh(&mesh, None).err();
    let ply = PlyWriter::new().add_mesh(&mesh, None).err();
    let stl = StlWriter::new().add_mesh(&mesh, None).err();
    for (format, error) in [("OBJ", obj), ("PLY", ply), ("STL", stl)] {
        let error = error.unwrap_or_else(|| panic!("{format} should refuse the short attribute"));
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{format}");
        assert!(
            error.to_string().contains("2 values for 4 points"),
            "{format}: {error}"
        );
    }
}
