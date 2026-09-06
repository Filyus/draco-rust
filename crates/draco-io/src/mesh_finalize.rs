//! Reader-level regression tests for [`draco_core::mesh::Mesh::finalize`].
//!
//! The pass itself lives in `draco-core`, which owns the three steps it
//! composes and the reason their order is load-bearing. What it buys, though,
//! is only observable through a reader that builds a mesh from scratch: a
//! merge that does not happen shows up as a face pair sharing a corner instead
//! of an edge, and as a wider quantization range. So the tests that pin it
//! sit next to the readers rather than next to the operation.

mod tests {
    #[cfg(feature = "obj-reader")]
    use draco_core::geometry_indices::FaceIndex;
    #[cfg(feature = "ply-reader")]
    use draco_core::geometry_indices::PointIndex;

    /// Two `v` lines carrying the same coordinates are one vertex once the
    /// values merge, and the triangles around them then share an edge rather
    /// than a single corner. Measured against C++ Draco 1.5.7 on this exact
    /// geometry: it encodes to the same 75 bytes, and before the merge this
    /// crate wrote 77 and decoded six points instead of four.
    #[cfg(feature = "obj-reader")]
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
    #[cfg(feature = "obj-reader")]
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

    #[cfg(feature = "ply-reader")]
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
