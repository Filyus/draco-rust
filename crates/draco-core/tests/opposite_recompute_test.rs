use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_indices::{CornerIndex, VertexIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;

fn create_grid_mesh(width: u32, height: u32) -> Mesh {
    let mut mesh = Mesh::new();
    let num_points = width * height;
    mesh.set_num_points(num_points as usize);

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
fn test_encoder_ct_equals_recomputed_ct() {
    for &size in &[4u32, 5u32, 6u32, 8u32, 10u32] {
        eprintln!(
            "\n--- Encoder vs recomputed CT test for grid {}x{} ---",
            size, size
        );
        let mesh = create_grid_mesh(size, size);

        let mut options = EncoderOptions::default();
        options.set_global_int("encoding_method", 1);
        options.set_global_int("encoding_speed", 5);

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

        // Recompute opposites by building faces vector and calling CornerTable::init
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
        assert!(
            recomputed.init(&faces),
            "Recomputed CornerTable::init failed for grid {}x{}",
            size,
            size
        );

        // Compare opposites
        for i in 0..ct.num_corners() {
            let c = CornerIndex(i as u32);
            let a = ct.opposite(c);
            let b = recomputed.opposite(c);
            if a != b {
                eprintln!(
                    "Corner {} opposite mismatch: enc={} recomputed={}",
                    i, a.0, b.0
                );
                // provide local context
                eprintln!(
                    " enc endpoints = ({},{})",
                    ct.vertex(ct.next(c)).0,
                    ct.vertex(ct.previous(c)).0
                );
                eprintln!(
                    " rec endpoints = ({},{})",
                    recomputed.vertex(recomputed.next(b)).0,
                    recomputed.vertex(recomputed.previous(b)).0
                );
                panic!(
                    "Encoder CT does not match recomputed CT for grid {}x{} at corner {}",
                    size, size, i
                );
            }
        }
        println!(
            "Encoder CT matches recomputed CT for grid {}x{}",
            size, size
        );
    }
}

#[test]
fn test_decoder_ct_equals_recomputed_ct() {
    for &size in &[4u32, 5u32, 6u32, 8u32, 10u32] {
        eprintln!(
            "\n--- Decoder vs recomputed CT test for grid {}x{} ---",
            size, size
        );
        let mesh = create_grid_mesh(size, size);

        let mut options = EncoderOptions::default();
        options.set_global_int("encoding_method", 1);
        options.set_global_int("encoding_speed", 5);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut buffer = EncoderBuffer::new();
        encoder
            .encode(&options, &mut buffer)
            .expect("Encode failed");

        // Full decode and get decoder CT
        let mut decoder = MeshDecoder::new();
        let mut decoded_mesh = Mesh::new();
        let mut dec_buffer = DecoderBuffer::new(buffer.data());
        decoder
            .decode(&mut dec_buffer, &mut decoded_mesh)
            .expect("Decode failed");
        let dec_ct = decoder
            .get_corner_table_ref()
            .expect("Decoder did not produce corner table")
            .clone();

        use draco_core::corner_table::CornerTable;
        let num_faces = dec_ct.num_faces();
        let mut faces: Vec<[VertexIndex; 3]> = Vec::with_capacity(num_faces);
        for fi in 0..num_faces {
            let c0 = CornerIndex((fi * 3) as u32);
            let v0 = dec_ct.vertex(c0);
            let v1 = dec_ct.vertex(dec_ct.next(c0));
            let v2 = dec_ct.vertex(dec_ct.previous(c0));
            faces.push([v0, v1, v2]);
        }
        let mut recomputed = CornerTable::new(0);
        assert!(
            recomputed.init(&faces),
            "Recomputed CornerTable::init failed for decoder grid {}x{}",
            size,
            size
        );

        for i in 0..dec_ct.num_corners() {
            let c = CornerIndex(i as u32);
            let a = dec_ct.opposite(c);
            let b = recomputed.opposite(c);
            assert_eq!(
                a, b,
                "Decoder CT opposite differs from recomputed for grid {}x{} at corner {}",
                size, size, i
            );
        }
        println!(
            "Decoder CT matches recomputed CT for grid {}x{}",
            size, size
        );
    }
}
