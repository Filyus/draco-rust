use draco_core::corner_table::CornerTable;
use draco_core::geometry_indices::{FaceIndex, PointIndex, VertexIndex, INVALID_CORNER_INDEX};
use draco_core::mesh::Mesh;

fn create_torus_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    let n = 5;
    let m = 5;
    mesh.set_num_points(n * m);
    mesh.set_num_faces(2 * n * m);

    let mut face_idx = 0;
    for i in 0..n {
        for j in 0..m {
            let v00 = i * m + j;
            let v10 = ((i + 1) % n) * m + j;
            let v01 = i * m + ((j + 1) % m);
            let v11 = ((i + 1) % n) * m + ((j + 1) % m);

            mesh.set_face(
                FaceIndex(face_idx),
                [
                    PointIndex(v00 as u32),
                    PointIndex(v10 as u32),
                    PointIndex(v01 as u32),
                ],
            );
            face_idx += 1;
            mesh.set_face(
                FaceIndex(face_idx),
                [
                    PointIndex(v10 as u32),
                    PointIndex(v11 as u32),
                    PointIndex(v01 as u32),
                ],
            );
            face_idx += 1;
        }
    }
    mesh
}

#[test]
fn test_corner_table_torus() {
    let mesh = create_torus_mesh();

    let faces: Vec<[VertexIndex; 3]> = (0..mesh.num_faces())
        .map(|i| {
            let face = mesh.face(FaceIndex(i as u32));
            [
                VertexIndex(face[0].0),
                VertexIndex(face[1].0),
                VertexIndex(face[2].0),
            ]
        })
        .collect();

    let mut ct = CornerTable::new(faces.len());
    ct.init(&faces);

    assert_eq!(ct.num_faces(), 50);
    assert_eq!(ct.num_vertices(), 25);
    assert_eq!(ct.num_corners(), 150);

    // Check for boundary edges
    let mut boundary_edges = 0;
    for c in 0..ct.num_corners() {
        let c_idx = draco_core::geometry_indices::CornerIndex(c as u32);
        if ct.opposite(c_idx) == INVALID_CORNER_INDEX {
            boundary_edges += 1;
        }
    }

    assert_eq!(boundary_edges, 0, "Torus should have no boundary edges");
}

#[test]
fn test_corner_table_invariants_and_determinism() {
    let mesh = create_torus_mesh();
    let faces: Vec<[VertexIndex; 3]> = (0..mesh.num_faces())
        .map(|i| {
            let face = mesh.face(FaceIndex(i as u32));
            [
                VertexIndex(face[0].0),
                VertexIndex(face[1].0),
                VertexIndex(face[2].0),
            ]
        })
        .collect();

    let mut ct1 = CornerTable::new(faces.len());
    assert!(ct1.init(&faces));
    // In debug builds, init() will have asserted invariants. Explicitly check here too.
    assert!(ct1.validate_invariants());

    // Re-init and ensure deterministic vertex_corners mapping
    let mut ct2 = CornerTable::new(faces.len());
    assert!(ct2.init(&faces));
    assert_eq!(
        ct1.vertex_corners, ct2.vertex_corners,
        "Vertex corner mapping should be deterministic and repeatable"
    );
}
