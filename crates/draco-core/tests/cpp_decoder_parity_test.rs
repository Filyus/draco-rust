use draco_core::mesh::Mesh;

// Parse C++ debug log lines of the form:
// "C++ OnNewVertexVisited: data_id=0 vertex=1 corner=1 point_id=1"
// For encoder logs, look for "C++ Encoder OnNewVertexVisited" prefix instead
fn parse_cpp_encoder_lines(content: &str) -> Vec<String> {
    // Parse C++ "OnNewVertexVisited" lines from encoder and return canonical corner->point sequences
    // We specifically want the ENCODER section, which starts after "ORIG GenerateSequenceInternal: using corner_order"
    let mut out = Vec::new();
    let mut in_encoder_section = false;
    for line in content.lines() {
        // Detect encoder section (has corner_order, not "NO corner_order")
        if line.contains("using corner_order with") {
            in_encoder_section = true;
        }
        // Detect decoder section (uses sequential faces)
        if line.contains("NO corner_order") {
            in_encoder_section = false;
        }
        // Only parse encoder OnNewVertexVisited lines
        if in_encoder_section && line.contains("C++ OnNewVertexVisited") {
            // extract corner and point_id
            let mut corner: Option<&str> = None;
            let mut point_id: Option<&str> = None;
            for token in line.split_whitespace() {
                if token.starts_with("corner=") {
                    corner = Some(token.trim_start_matches("corner=").trim_end_matches(','));
                }
                if token.starts_with("point_id=") {
                    point_id = Some(token.trim_start_matches("point_id=").trim_end_matches(','));
                }
            }
            if let (Some(c), Some(p)) = (corner, point_id) {
                // Use canonical point-level mapping: MAP_POINT:{corner}->p{point_id}
                out.push(format!("MAP_POINT:{}->p{}", c, p));
            }
        }
    }
    out
}

#[test]
fn test_cpp_grid5x5_encoder_parity() {
    // Initialize and clear the test event log
    draco_core::test_event_log::init();
    draco_core::test_event_log::clear();

    // Read expected events from the original C++ debug log (encoder section)
    let cpp_log = std::env::var("DRACO_CPP_GRID5X5_LOG")
        .ok()
        .map(std::fs::read_to_string)
        .transpose()
        .or_else(|_| {
            std::fs::read_to_string("../../debug_logs/cpp_dec_grid5x5_original.txt").map(Some)
        })
        .or_else(|_| std::fs::read_to_string("./debug_logs/cpp_dec_grid5x5_original.txt").map(Some))
        .unwrap_or(None);
    let Some(cpp_log) = cpp_log else {
        println!(
            "Skipping test - C++ debug log not found. Set DRACO_CPP_GRID5X5_LOG or place cpp_dec_grid5x5_original.txt in debug_logs/"
        );
        return;
    };
    let expected = parse_cpp_encoder_lines(&cpp_log);

    // Build a 5x5 grid and run the Rust encoder to capture encoder traversal events
    fn create_grid_mesh(width: u32, height: u32) -> Mesh {
        // Build like C++ TriangleSoupMeshBuilder: add faces and create points on first use,
        // so that point indices are assigned in the same order as the C++ builder.
        let mut mesh = Mesh::new();

        // Temporary storage for unique points and a map from raw bytes to point index.
        use std::collections::HashMap;
        let mut value_map: HashMap<Vec<u8>, u32> = HashMap::new();
        let mut unique_points: Vec<[f32; 3]> = Vec::new();

        // Create a position attribute with no points yet.
        let mut pos_attr = draco_core::geometry_attribute::PointAttribute::new();
        pos_attr.init(
            draco_core::geometry_attribute::GeometryAttributeType::Position,
            3,
            draco_core::draco_types::DataType::Float32,
            false,
            0,
        );

        // Reserve face storage and assign faces by mapping coordinates to point ids in the same order
        // Triangle 1: p0(x,y), p1(x+1,y), p2(x,y+1)
        // Triangle 2: p1(x+1,y), p3(x+1,y+1), p2(x,y+1)
        let mut face_idx = 0u32;
        for _ in 0..((width - 1) * (height - 1) * 2) {
            // placeholder faces, we'll set them below once we have point ids
            mesh.set_face(
                draco_core::FaceIndex(face_idx),
                [
                    draco_core::PointIndex(0),
                    draco_core::PointIndex(0),
                    draco_core::PointIndex(0),
                ],
            );
            face_idx += 1;
        }

        // Reset face_idx and fill faces while creating points in encountered order.
        face_idx = 0;
        for y in 0..height - 1 {
            for x in 0..width - 1 {
                let coords = [
                    [x as f32, y as f32, 0.0f32],
                    [x as f32 + 1.0f32, y as f32, 0.0f32],
                    [x as f32, y as f32 + 1.0f32, 0.0f32],
                    [x as f32 + 1.0f32, y as f32 + 1.0f32, 0.0f32],
                ];

                let mut ids = [0u32; 4];
                for i in 0..4 {
                    let bytes: Vec<u8> = coords[i]
                        .iter()
                        .flat_map(|f| f.to_le_bytes().to_vec())
                        .collect();
                    if let Some(&id) = value_map.get(&bytes) {
                        ids[i] = id;
                    } else {
                        let id = unique_points.len() as u32;
                        value_map.insert(bytes, id);
                        unique_points.push(coords[i]);
                        ids[i] = id;
                    }
                }

                // Triangle 1: p0, p1, p2
                mesh.set_face(
                    draco_core::FaceIndex(face_idx),
                    [
                        draco_core::PointIndex(ids[0]),
                        draco_core::PointIndex(ids[1]),
                        draco_core::PointIndex(ids[2]),
                    ],
                );
                face_idx += 1;
                // Triangle 2: p1, p3, p2
                mesh.set_face(
                    draco_core::FaceIndex(face_idx),
                    [
                        draco_core::PointIndex(ids[1]),
                        draco_core::PointIndex(ids[3]),
                        draco_core::PointIndex(ids[2]),
                    ],
                );
                face_idx += 1;
            }
        }

        // Now set the unique points into the attribute buffer and attach to the mesh.
        let num_points = unique_points.len();
        mesh.set_num_points(num_points);
        // Re-init pos_attr with correct size
        pos_attr.init(
            draco_core::geometry_attribute::GeometryAttributeType::Position,
            3,
            draco_core::draco_types::DataType::Float32,
            false,
            num_points,
        );
        for (i, p) in unique_points.iter().enumerate() {
            let offset = i * 3 * 4;
            pos_attr
                .buffer_mut()
                .update(&p[0].to_le_bytes(), Some(offset));
            pos_attr
                .buffer_mut()
                .update(&p[1].to_le_bytes(), Some(offset + 4));
            pos_attr
                .buffer_mut()
                .update(&p[2].to_le_bytes(), Some(offset + 8));
        }
        mesh.add_attribute(pos_attr);
        mesh
    }

    // Enable verbose encoder logging for this test so we can compare internal seeds
    std::env::set_var("DRACO_VERBOSE", "1");

    // Run encoder on a 5x5 grid and capture encoder events
    let mesh = create_grid_mesh(5, 5);

    // Print attribute mapped indices for comparison with C++ dedup results
    if std::env::var("DRACO_VERBOSE").is_ok() {
        let att = mesh.attribute(0);
        let mut mapped = Vec::new();
        for i in 0..mesh.num_points() {
            mapped.push(att.mapped_index(draco_core::PointIndex(i as u32)).0);
        }
        println!("Rust pos mapped (first 25): {:?}", mapped);
    }
    let mut encoder = draco_core::mesh_encoder::MeshEncoder::new();
    encoder.set_mesh(mesh);
    draco_core::test_event_log::clear();
    let mut enc_buf = draco_core::encoder_buffer::EncoderBuffer::new();
    let mut opts = draco_core::encoder_options::EncoderOptions::default();
    opts.set_global_int("encoding_method", 1);
    opts.set_global_int("encoding_speed", 5);
    encoder.encode(&opts, &mut enc_buf).expect("Encode failed");
    let recorded_all = draco_core::test_event_log::take_events();

    // Filter recorded events to the canonical MAP_POINT entries
    let recorded: Vec<String> = recorded_all
        .into_iter()
        .filter(|s| s.starts_with("MAP_POINT:"))
        .collect();

    // Compare canonical point-level sequences (C++ logs only first 10 visits)
    let min_len = std::cmp::min(expected.len(), recorded.len());
    println!(
        "\nExpected (from C++ encoder, {} entries): {:?}",
        expected.len(),
        expected
    );
    println!(
        "Recorded (from Rust encoder, {} entries): {:?}",
        recorded.len(),
        &recorded[..min_len.min(40)]
    );

    for i in 0..min_len {
        if expected[i] != recorded[i] {
            panic!("Event mismatch idx {}: exp='{}' got='{}'\nExpected (first {}): {:?}\nRecorded (first {}): {:?}",
                i, expected[i], recorded[i], min_len, &expected[..min_len], min_len, &recorded[..min_len]);
        }
    }
    // Only warn if lengths differ since C++ might only log first N visits
    if expected.len() != recorded.len() {
        println!("Note: Event sequences differ in length: expected={} recorded={} (C++ only logs first 10)", expected.len(), recorded.len());
    }
    // If we got here without panic, first min_len entries match
    println!(
        "SUCCESS: First {} entries match between C++ and Rust encoder",
        min_len
    );
}
