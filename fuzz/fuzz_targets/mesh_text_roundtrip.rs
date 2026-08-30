#![no_main]

//! The OBJ, PLY and STL *writers*, fed meshes their own readers produced.
//!
//! `mesh_text_readers` covers the parsers; nothing covered the writers, and the
//! fuzz build did not even compile them in. That is the gap this closes, and it
//! is not a hypothetical one: an audit of the ASCII STL writer found a NaN
//! coordinate spelled as `2.696539702293474e+308` -- a number every reader
//! accepts as a real position.
//!
//! Meshes come from the readers rather than from a builder, for the reason
//! `fbx_roundtrip` gives: a hand-built mesh only covers shapes someone thought
//! to write down, while a parsed one carries whatever survived a mutation --
//! empty attribute sets, degenerate faces, coordinates at the ends of the
//! range.
//!
//! Three oracles, and none of them is byte identity against the input. A text
//! writer is not obliged to reproduce its source: OBJ and PLY spell six decimal
//! places, so a value is rounded on the way out, and STL stores a facet normal
//! the writer recomputes.
//!
//! 1. **What we write, we can read.** A writer may refuse a mesh it cannot
//!    represent, but bytes it does emit must satisfy our own reader.
//! 2. **Writing settles.** The first write may lose what the container has no
//!    place for; it may not keep losing. Once a mesh has been through the
//!    format, writing it again must give the same bytes every time -- which is
//!    what catches formatting that depends on anything but the values:
//!    uninitialised state, iteration order, a buffer reused across calls.
//! 3. **A non-number stays a non-number.** If the mesh held a coordinate that
//!    is not a finite value, the file may refuse it or spell it, but must not
//!    turn it into an ordinary number. Idempotence alone does not see this: a
//!    NaN written as a huge finite decimal reads back as that decimal and
//!    writes again unchanged, which is exactly how the audited bug would have
//!    survived a round-trip test.

use std::io;

use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_io::obj_reader::ObjReader;
use draco_io::obj_writer::ObjWriter;
use draco_io::ply_format::PlyFormat;
use draco_io::ply_reader::PlyReader;
use draco_io::ply_writer::PlyWriter;
use draco_io::stl_reader::StlReader;
use draco_io::stl_writer::{StlFormat, StlWriter};
use draco_io::traits::Writer;
use draco_io::WriteToBytes;
use libfuzzer_sys::fuzz_target;

/// One writer plus the container it writes, since a format's text and binary
/// spellings are different code.
#[derive(Debug, Clone, Copy)]
enum Spelling {
    Obj,
    Ply(PlyFormat),
    Stl(StlFormat),
}

const SPELLINGS: &[Spelling] = &[
    Spelling::Obj,
    Spelling::Ply(PlyFormat::Ascii),
    Spelling::Ply(PlyFormat::BinaryLittleEndian),
    Spelling::Ply(PlyFormat::BinaryBigEndian),
    Spelling::Stl(StlFormat::Ascii),
    Spelling::Stl(StlFormat::Binary),
];

fn write(spelling: Spelling, mesh: &Mesh) -> io::Result<Vec<u8>> {
    match spelling {
        Spelling::Obj => {
            let mut writer = ObjWriter::new();
            writer.add_mesh(mesh, Some("m"))?;
            writer.write_to_vec()
        }
        Spelling::Ply(format) => {
            let mut writer = PlyWriter::new().with_format(format);
            writer.add_mesh(mesh, Some("m"))?;
            writer.write_to_vec()
        }
        Spelling::Stl(format) => {
            let mut writer = StlWriter::new().with_format(format);
            writer.add_mesh(mesh, Some("m"))?;
            writer.write_to_vec()
        }
    }
}

fn read(spelling: Spelling, bytes: &[u8]) -> io::Result<Mesh> {
    match spelling {
        Spelling::Obj => ObjReader::read_from_bytes(bytes),
        Spelling::Ply(_) => PlyReader::read_from_bytes(bytes),
        Spelling::Stl(_) => StlReader::read_from_bytes(bytes),
    }
}

/// How many position components the mesh cannot express as a finite number,
/// counted over the points its faces reference.
///
/// Only those survive a round trip through any of these three formats. STL has
/// no vertex list at all -- it writes three corners per triangle -- and our OBJ
/// and PLY readers build the mesh from the faces, so a vertex nothing points at
/// is dropped on the way back in. Counting it would make the oracle demand that
/// a container keep what it has no way to name.
fn non_finite_components(mesh: &Mesh) -> usize {
    let id = mesh.named_attribute_id(GeometryAttributeType::Position);
    if id < 0 {
        return 0;
    }
    let attribute = mesh.attribute(id);
    let stride = attribute.byte_stride() as usize;
    let components = usize::from(attribute.num_components());

    let mut count = 0;
    for face in 0..mesh.num_faces() {
        for point in mesh.face(FaceIndex(face as u32)) {
            let point = point.0 as usize;
            // A face may name a point the attribute does not have; that is the
            // reader's business, not this oracle's.
            if point >= mesh.num_points() {
                continue;
            }
            // Through the point map. Several points share one value once a
            // reader has merged the values that repeat, so the point index is
            // not the value's address -- and reading as though it were walks
            // off the end of the buffer, which is this oracle crashing rather
            // than the code it is watching.
            let value = attribute.mapped_index(PointIndex(point as u32)).0 as usize;
            for component in 0..components {
                let mut bytes = [0u8; 4];
                if !attribute
                    .buffer()
                    .try_read(value * stride + component * 4, &mut bytes)
                {
                    continue;
                }
                if !f32::from_le_bytes(bytes).is_finite() {
                    count += 1;
                }
            }
        }
    }
    count
}

fn check(spelling: Spelling, mesh: &Mesh) {
    let Ok(first) = write(spelling, mesh) else {
        // Refusing a mesh the format cannot carry is the writer doing its job.
        return;
    };

    let reread = read(spelling, &first)
        .unwrap_or_else(|error| panic!("{spelling:?} wrote bytes its own reader rejects: {error}"));

    if non_finite_components(mesh) > 0 {
        assert!(
            non_finite_components(&reread) > 0,
            "{spelling:?} turned a non-finite coordinate into an ordinary number"
        );
    }

    // Idempotence is claimed from the second write on, not the first. The first
    // one is allowed to lose what the container has no place for -- an OBJ
    // normal belongs to a face corner, so a mesh with no faces writes `vn`
    // lines nothing can attach on the way back, and a PLY vertex no face
    // mentions is dropped by STL, which has no vertex list. What must hold is
    // that this happens once: everything the file could not say is already
    // gone by the second write, so the third has to match it byte for byte.
    let Some(second) = write_again(spelling, &reread) else {
        return;
    };
    let twice = read(spelling, &second)
        .unwrap_or_else(|error| panic!("{spelling:?} wrote bytes its own reader rejects: {error}"));
    let Some(third) = write_again(spelling, &twice) else {
        return;
    };

    assert!(
        second == third,
        "{spelling:?} kept changing what it wrote after the round trip settled"
    );
}

/// Writes a mesh that came back out of this format's own reader.
///
/// `None` for the one refusal that is not a defect: a document with no geometry
/// at all. `solid m` / `endsolid m` is a valid file, and the mesh it reads back
/// as carries no position attribute -- which the writer refuses, because it
/// cannot tell that mesh from one whose positions went missing.
fn write_again(spelling: Spelling, mesh: &Mesh) -> Option<Vec<u8>> {
    match write(spelling, mesh) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            assert!(
                mesh.num_faces() == 0,
                "{spelling:?} refused a mesh it had just written: {error}"
            );
            None
        }
    }
}

fuzz_target!(|data: &[u8]| {
    // Every reader sees every input, as in `mesh_text_readers`: whichever one
    // recognises it hands a mesh to all six spellings, so a PLY file exercises
    // the OBJ and STL writers too.
    for parsed in [
        ObjReader::read_from_bytes(data),
        PlyReader::read_from_bytes(data),
        StlReader::read_from_bytes(data),
    ] {
        let Ok(mesh) = parsed else {
            continue;
        };
        for spelling in SPELLINGS {
            check(*spelling, &mesh);
        }
    }
});
