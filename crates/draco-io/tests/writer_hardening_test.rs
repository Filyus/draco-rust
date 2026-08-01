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

#[test]
fn the_ply_writer_handles_a_mesh_with_no_position_attribute() {
    // The normal/color/texcoord padding measured how far behind the position
    // list they were, which underflows when nothing was appended to it.
    let mut ply = <PlyWriter as Writer>::new();
    let _ = ply.add_mesh(&quad(None), None);
}
