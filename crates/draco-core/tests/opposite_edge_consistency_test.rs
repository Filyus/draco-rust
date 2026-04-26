use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_indices::{CornerIndex, VertexIndex, INVALID_CORNER_INDEX};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;

// Small grid generator used for deterministic layouts
fn create_grid_mesh(width: u32, height: u32) -> Mesh {
    let mut mesh = Mesh::new();
    let num_points = width * height;
    mesh.set_num_points(num_points as usize);

    // positions not required for this test, only topology
    let mut face_idx = 0u32;
    for y in 0..height - 1 {
        for x in 0..width - 1 {
            let p0 = y * width + x;
            let p1 = y * width + (x + 1);
            let p2 = (y + 1) * width + x;
            let p3 = (y + 1) * width + (x + 1);

            mesh.set_face(
                draco_core::geometry_indices::FaceIndex(face_idx),
                [
                    draco_core::geometry_indices::PointIndex(p0),
                    draco_core::geometry_indices::PointIndex(p1),
                    draco_core::geometry_indices::PointIndex(p2),
                ],
            );
            face_idx += 1;
            mesh.set_face(
                draco_core::geometry_indices::FaceIndex(face_idx),
                [
                    draco_core::geometry_indices::PointIndex(p1),
                    draco_core::geometry_indices::PointIndex(p3),
                    draco_core::geometry_indices::PointIndex(p2),
                ],
            );
            face_idx += 1;
        }
    }
    mesh.set_num_faces(face_idx as usize);
    mesh
}

#[test]
fn test_encoder_ct_opposite_edges_consistent() {
    for &size in &[4u32, 5u32, 6u32, 8u32, 10u32] {
        eprintln!(
            "\n--- Opposite-edge sanity test for grid {}x{} ---",
            size, size
        );
        let mesh = create_grid_mesh(size, size);

        let mut options = EncoderOptions::default();
        options.set_global_int("encoding_method", 1); // Edgebreaker
        options.set_global_int("encoding_speed", 5); // Parallelogram path

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut buffer = EncoderBuffer::new();
        encoder
            .encode(&options, &mut buffer)
            .expect("Encode failed");

        let ct = encoder
            .corner_table()
            .expect("Expected encoder to produce a corner table")
            .clone();

        // For every corner with an opposite, ensure the opposite corresponds to the same undirected edge
        let num_c = ct.num_corners();
        for i in 0..num_c {
            let c = CornerIndex(i as u32);
            let opp = ct.opposite(c);
            if opp == INVALID_CORNER_INDEX {
                continue;
            }

            // Edge endpoints for corner c are (next(c), previous(c)) as vertex indices
            let a = ct.vertex(ct.next(c)).0;
            let b = ct.vertex(ct.previous(c)).0;
            let oa = ct.vertex(ct.next(opp)).0;
            let ob = ct.vertex(ct.previous(opp)).0;

            // Check unordered equality
            let ok = (a == oa && b == ob) || (a == ob && b == oa);
            if !ok {
                eprintln!(
                    "Mismatch at corner {}: opposite set to {} but edge endpoints differ",
                    i, opp.0
                );
                eprintln!(" corner {} endpoints = ({},{})", i, a, b);
                eprintln!(" opp {} endpoints = ({},{})", opp.0, oa, ob); // Recompute opposites using a fresh CornerTable::init to see what the canonical opposite should be
                {
                    use draco_core::corner_table::CornerTable;
                    let num_faces = ct.num_faces();
                    let mut faces: Vec<[VertexIndex; 3]> = Vec::with_capacity(num_faces);
                    for fi in 0..num_faces {
                        let c0 = CornerIndex((fi * 3) as u32);
                        let v0 = ct.vertex(c0);
                        let v1 = ct.vertex(ct.next(c0));
                        let v2 = ct.vertex(ct.previous(c0));
                        faces.push([v0, v1, v2]);
                    }
                    let mut recomputed = CornerTable::new(0);
                    let ok_init = recomputed.init(&faces);
                    if ok_init {
                        let recomputed_opp = recomputed.opposite(CornerIndex(i as u32));
                        let roa = recomputed.vertex(recomputed.next(recomputed_opp)).0;
                        let rob = recomputed.vertex(recomputed.previous(recomputed_opp)).0;
                        eprintln!(
                            " recomputed opp for corner {} = {} endpoints = ({},{})",
                            i, recomputed_opp.0, roa, rob
                        );
                    } else {
                        eprintln!(" recomputed CornerTable::init failed");
                    }
                } // Also dump a small neighborhood for context
                let start = i.saturating_sub(5);
                let end = usize::min(num_c, i + 5);
                let enc_op: Vec<u32> = (start..end)
                    .map(|j| ct.opposite(CornerIndex(j as u32)).0)
                    .collect();
                eprintln!("enc_op ({}..{}) = {:?}", start, end, enc_op);
                panic!(
                    "Encoder CT opposite-edge consistency violated for grid {}x{} at corner {}",
                    size, size, i
                );
            }
        }
        println!(
            "Opposite-edge sanity passed for grid {}x{} ({} corners)",
            size, size, num_c
        );
    }
}
