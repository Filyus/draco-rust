use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use std::collections::BTreeSet;

// Create small grid mesh helper (same as other tests)
fn create_grid_mesh(width: u32, height: u32) -> Mesh {
    let mut mesh = Mesh::new();
    let num_points = width * height;
    mesh.set_num_points(num_points as usize);

    let mut pos_attr = draco_core::geometry_attribute::PointAttribute::new();
    pos_attr.init(
        draco_core::geometry_attribute::GeometryAttributeType::Position,
        3,
        draco_core::draco_types::DataType::Float32,
        false,
        num_points as usize,
    );

    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) as usize;
            let coords = [x as f32, y as f32, 0.0f32];
            let offset = i * 3 * 4;
            pos_attr
                .buffer_mut()
                .update(&coords[0].to_le_bytes(), Some(offset));
            pos_attr
                .buffer_mut()
                .update(&coords[1].to_le_bytes(), Some(offset + 4));
            pos_attr
                .buffer_mut()
                .update(&coords[2].to_le_bytes(), Some(offset + 8));
        }
    }
    mesh.add_attribute(pos_attr);

    let mut face_idx = 0;
    for y in 0..height - 1 {
        for x in 0..width - 1 {
            let p0 = y * width + x;
            let p1 = y * width + (x + 1);
            let p2 = (y + 1) * width + x;
            let p3 = (y + 1) * width + (x + 1);

            mesh.set_face(
                draco_core::FaceIndex(face_idx),
                [
                    draco_core::PointIndex(p0),
                    draco_core::PointIndex(p1),
                    draco_core::PointIndex(p2),
                ],
            );
            face_idx += 1;
            mesh.set_face(
                draco_core::FaceIndex(face_idx),
                [
                    draco_core::PointIndex(p1),
                    draco_core::PointIndex(p3),
                    draco_core::PointIndex(p2),
                ],
            );
            face_idx += 1;
        }
    }
    mesh.set_num_faces(face_idx as usize);
    mesh
}

#[test]
fn test_encoder_and_decoder_emit_complete_map_point_traversals_4x4() {
    // Initialize and clear the test event log
    draco_core::test_event_log::init();
    draco_core::test_event_log::clear();

    // Build mesh and run encoder path (which constructs the encoder corner
    // table and records traversal order used to simulate decoder-side
    // attribute sequencing)
    let mesh = create_grid_mesh(4, 4);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut buffer = EncoderBuffer::new();
    let mut options = EncoderOptions::default();
    options.set_global_int("encoding_method", 1);
    options.set_global_int("encoding_speed", 5);
    encoder
        .encode(&options, &mut buffer)
        .expect("Encode failed");

    let enc_events = draco_core::test_event_log::take_events();
    let enc_map_points: Vec<u32> = enc_events
        .iter()
        .filter_map(|event| event.strip_prefix("MAP_POINT:"))
        .filter_map(|payload| payload.split("->p").nth(1))
        .map(|point| point.parse::<u32>().expect("valid point id"))
        .collect();

    // Now clear and run decoder to capture its sequence
    draco_core::test_event_log::clear();

    let mut decoder = MeshDecoder::new();
    let mut decoded_mesh = draco_core::Mesh::new();
    let mut dec_buffer = DecoderBuffer::new(buffer.data());
    decoder
        .decode(&mut dec_buffer, &mut decoded_mesh)
        .expect("Decode failed");

    let dec_events = draco_core::test_event_log::take_events();
    let dec_map_points: Vec<u32> = dec_events
        .iter()
        .filter_map(|event| event.strip_prefix("MAP_POINT:"))
        .filter_map(|payload| payload.split("->p").nth(1))
        .map(|point| point.parse::<u32>().expect("valid point id"))
        .collect();

    // Encoder and decoder intentionally use different traversal seeds in C++:
    // the encoder sets corner_order to simulate decoder-side attribute
    // sequencing, while the decoder traverser runs face-sequentially without a
    // corner_order. Their MAP_POINT order therefore differs, but each side
    // should still visit every point exactly once.
    let expected_points: BTreeSet<u32> = (0..mesh.num_points() as u32).collect();
    let enc_points_set: BTreeSet<u32> = enc_map_points.iter().copied().collect();
    let dec_points_set: BTreeSet<u32> = dec_map_points.iter().copied().collect();

    assert_eq!(
        enc_map_points.len(),
        mesh.num_points(),
        "Encoder should emit one MAP_POINT event per point"
    );
    assert_eq!(
        dec_map_points.len(),
        decoded_mesh.num_points(),
        "Decoder should emit one MAP_POINT event per point"
    );
    assert_eq!(
        enc_points_set.len(),
        mesh.num_points(),
        "Encoder MAP_POINT events should not contain duplicate point IDs"
    );
    assert_eq!(
        dec_points_set.len(),
        decoded_mesh.num_points(),
        "Decoder MAP_POINT events should not contain duplicate point IDs"
    );
    assert_eq!(
        enc_points_set, expected_points,
        "Encoder should visit every point exactly once"
    );
    assert_eq!(
        dec_points_set, expected_points,
        "Decoder should visit every point exactly once"
    );
}
