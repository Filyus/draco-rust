//! How a reader finishes a mesh before anything encodes it.
//!
//! The three readers that build a mesh from scratch -- OBJ, PLY and the glTF
//! geometry path -- all end the same way, and upstream ends the same way too:
//! `TriangleSoupMeshBuilder::Finalize` merges bit-identical attribute values,
//! then merges the points those values made identical. Its OBJ and PLY readers
//! call the same pair directly.
//!
//! It then goes one step further than upstream, which stops there: a point no
//! face names, and a value no point names, are dropped. Neither reaches a
//! decoder in either implementation -- both encoders write what the
//! connectivity reaches -- but upstream's quantization range is computed over
//! the values an attribute holds, so carrying them spends precision on
//! geometry that is not there. `COMPATIBILITY.md` records that departure.
//!
//! Doing this is not tidying. Two vertices carrying the same position arrive as
//! two values, and until they are merged the triangles around them share a
//! vertex rather than an edge -- so the encoder sees two connected components
//! where upstream sees one, and writes a larger stream that decodes to more
//! points than it was given.

use std::io;

use draco_core::mesh::Mesh;

/// Merges duplicate attribute values, then duplicate points, rewriting the
/// faces that named them.
///
/// The order is upstream's and is load-bearing: two vertices with equal bytes
/// hold distinct value indices until the values merge, so a point merge run
/// first would find nothing to do.
pub(crate) fn finalize_mesh(mesh: &mut Mesh) -> io::Result<()> {
    mesh.deduplicate_attribute_values()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    mesh.deduplicate_point_ids();
    // Past upstream, deliberately: it stops here and carries whatever the
    // vertex list held. A point no face names reaches no decoder either way --
    // both encoders write what the connectivity reaches -- but it does reach
    // the quantization range, so keeping it spends precision on geometry that
    // is not there. See the section on it in COMPATIBILITY.md.
    mesh.remove_points_unused_by_faces();
    Ok(())
}

#[cfg(test)]
mod tests {
    use draco_core::geometry_indices::{FaceIndex, PointIndex};

    /// Two `v` lines carrying the same coordinates are one vertex once the
    /// values merge, and the triangles around them then share an edge rather
    /// than a single corner. Measured against C++ Draco 1.5.7 on this exact
    /// geometry: it encodes to the same 75 bytes, and before the merge this
    /// crate wrote 77 and decoded six points instead of four.
    #[test]
    fn two_vertices_at_one_position_become_one_point() {
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\nf 4 5 3\n";
        let mesh = crate::ObjReader::read_from_bytes(obj.as_bytes()).expect("read");

        assert_eq!(
            mesh.num_points(),
            4,
            "the duplicated position did not merge"
        );
        assert_eq!(mesh.num_faces(), 2);

        // The shared edge is what the merge buys: the two faces have two
        // points in common, where without it they would have one.
        let face = |i: usize| mesh.face(FaceIndex(i as u32));
        let (a, b) = (face(0), face(1));
        let shared = a.iter().filter(|p| b.contains(p)).count();
        assert_eq!(shared, 2, "faces {a:?} and {b:?} do not share an edge");
    }

    /// The same file through the PLY reader, which reaches the values by a
    /// different route: a vertex list rather than face corners.
    /// A vertex no face refers to leaves before the encoder sees the mesh, and
    /// leaves the same way in every reader -- the three disagreed before this
    /// was one step: OBJ dropped it by interning corners, PLY and glTF kept it.
    #[test]
    fn a_vertex_no_face_uses_is_dropped() {
        let obj = "v 0 0 0
v 1 0 0
v 0 1 0
v 1000 1000 1000
f 1 2 3
";
        let mesh = crate::ObjReader::read_from_bytes(obj.as_bytes()).expect("read");
        assert_eq!(mesh.num_points(), 3);
        assert_eq!(mesh.num_faces(), 1);
        assert_eq!(
            mesh.attribute(0).size(),
            3,
            "the unused vertex still holds an attribute value, and would widen              the quantization range the encoder computes from it"
        );
    }

    #[test]
    fn the_ply_reader_merges_the_same_way() {
        let mut ply = b"ply\nformat binary_little_endian 1.0\nelement vertex 5\n\
            property float x\nproperty float y\nproperty float z\n\
            element face 2\nproperty list uchar uint vertex_index\nend_header\n"
            .to_vec();
        for v in [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
        ] {
            for c in v {
                ply.extend_from_slice(&c.to_le_bytes());
            }
        }
        for f in [[0u32, 1, 2], [3, 4, 2]] {
            ply.push(3);
            for i in f {
                ply.extend_from_slice(&i.to_le_bytes());
            }
        }

        let mesh = crate::PlyReader::read_from_bytes(&ply).expect("read");
        assert_eq!(
            mesh.num_points(),
            4,
            "the duplicated position did not merge"
        );
        assert_eq!(mesh.num_faces(), 2);
        assert!(
            (0..mesh.num_points()).all(|p| mesh
                .faces()
                .iter()
                .any(|f| f.contains(&PointIndex(p as u32)))),
            "a surviving point is named by no face"
        );
    }
}
