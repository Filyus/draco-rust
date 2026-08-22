//! Byte parity on meshes whose attributes do not share the position's
//! connectivity.
//!
//! Every other parity case gives each attribute one value per point, which is
//! the case where a mesh has a single connectivity and the encoder never builds
//! a `MeshAttributeCornerTable`. A vertex carrying two different UVs -- what a
//! UV island boundary is, and what any unwrapped model out of Blender has --
//! takes the other path entirely: the position attribute has fewer unique
//! values than the mesh has points, both attributes carry explicit point maps,
//! and the encoder has to split the attribute's connectivity from the
//! position's and encode the seams.
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{AttributeValueIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;

/// Every speed is asserted. Kept as a named constant so that narrowing the
/// guarantee again is a visible edit rather than a quiet one.
const ASSERTED_FROM_SPEED: i32 = 0;

const POSITION_BITS: i32 = 14;
const UV_BITS: i32 = 12;

/// A mesh with a UV seam: `position_map` collapses several points onto the same
/// position value, while the UVs stay distinct.
struct SeamSample {
    name: &'static str,
    positions: Vec<f32>,
    position_map: Vec<u32>,
    uvs: Vec<f32>,
    uv_map: Vec<u32>,
    faces: Vec<u32>,
}

/// Two triangles sharing an edge, with the shared vertices carrying a different
/// UV on each side -- the smallest thing that is a seam.
fn two_triangle_seam() -> SeamSample {
    SeamSample {
        name: "two triangles across a seam",
        positions: vec![
            0.0, 0.0, 0.0, // 0
            1.0, 0.0, 0.0, // 1
            0.0, 1.0, 0.0, // 2
            1.0, 1.0, 0.0, // 3
        ],
        position_map: vec![0, 1, 2, 1, 3, 2],
        uvs: vec![
            0.0, 0.0, // p0
            1.0, 0.0, // p1
            0.0, 1.0, // p2
            0.2, 0.0, // p3, same vertex as p1
            1.0, 1.0, // p4
            0.2, 1.0, // p5, same vertex as p2
        ],
        uv_map: vec![0, 1, 2, 3, 4, 5],
        faces: vec![0, 1, 2, 3, 4, 5],
    }
}

/// A strip cut down the middle: every vertex on the cut carries two UVs, so the
/// seam runs the length of the mesh rather than sitting on one edge.
fn split_strip(quads: usize) -> SeamSample {
    let mut positions = Vec::new();
    for i in 0..=quads {
        let x = i as f32;
        positions.extend([x, 0.0, 0.0]);
        positions.extend([x, 1.0, ((i % 3) as f32) * 0.25]);
        positions.extend([x, 2.0, 0.0]);
    }
    let vertex = |i: usize, row: usize| (i * 3 + row) as u32;

    // Points: the middle row is duplicated, once for each side of the cut.
    let mut position_map = Vec::new();
    let mut uvs = Vec::new();
    let mut uv_map = Vec::new();
    let mut point_of = Vec::new();
    for i in 0..=quads {
        let mut column = [0u32; 4];
        for (slot, (row, v)) in [(0usize, 0.0f32), (1, 0.5), (1, 0.55), (2, 1.0)]
            .into_iter()
            .enumerate()
        {
            column[slot] = position_map.len() as u32;
            position_map.push(vertex(i, row));
            uv_map.push(uvs.len() as u32 / 2);
            uvs.extend([i as f32 / quads as f32, v]);
        }
        point_of.push(column);
    }

    let mut faces = Vec::new();
    for i in 0..quads {
        let a = point_of[i];
        let b = point_of[i + 1];
        // Lower band, below the cut.
        faces.extend([a[0], b[0], a[1]]);
        faces.extend([b[0], b[1], a[1]]);
        // Upper band, above it.
        faces.extend([a[2], b[2], a[3]]);
        faces.extend([b[2], b[3], a[3]]);
    }

    SeamSample {
        name: "strip split along a seam",
        positions,
        position_map,
        uvs,
        uv_map,
        faces,
    }
}

fn build_rust_mesh(sample: &SeamSample) -> Mesh {
    let num_points = sample.position_map.len();
    let mut mesh = Mesh::new();
    mesh.set_num_points(num_points);

    let mut positions = PointAttribute::new();
    positions.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        sample.positions.len() / 3,
    );
    for (i, value) in sample.positions.iter().enumerate() {
        positions
            .buffer_mut()
            .update(&value.to_le_bytes(), Some(i * 4));
    }
    positions.set_explicit_mapping(num_points);
    for (point, &entry) in sample.position_map.iter().enumerate() {
        positions.set_point_map_entry(PointIndex(point as u32), AttributeValueIndex(entry));
    }
    mesh.add_attribute(positions);

    let mut texcoords = PointAttribute::new();
    texcoords.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Float32,
        false,
        sample.uvs.len() / 2,
    );
    for (i, value) in sample.uvs.iter().enumerate() {
        texcoords
            .buffer_mut()
            .update(&value.to_le_bytes(), Some(i * 4));
    }
    texcoords.set_explicit_mapping(num_points);
    for (point, &entry) in sample.uv_map.iter().enumerate() {
        texcoords.set_point_map_entry(PointIndex(point as u32), AttributeValueIndex(entry));
    }
    mesh.add_attribute(texcoords);

    for face in sample.faces.as_chunks::<3>().0 {
        mesh.add_face([
            PointIndex(face[0]),
            PointIndex(face[1]),
            PointIndex(face[2]),
        ]);
    }
    mesh
}

fn encode_rust(sample: &SeamSample, speed: i32) -> Vec<u8> {
    let mesh = build_rust_mesh(sample);
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);
    options.set_attribute_int(0, "quantization_bits", POSITION_BITS);
    options.set_attribute_int(1, "quantization_bits", UV_BITS);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    let _ = encoder.encode(&options, &mut buffer);
    buffer.data().to_vec()
}

fn first_difference(left: &[u8], right: &[u8]) -> Option<usize> {
    let shared = left.len().min(right.len());
    (0..shared)
        .find(|&i| left[i] != right[i])
        .or(if left.len() == right.len() {
            None
        } else {
            Some(shared)
        })
}

#[test]
fn encoder_output_matches_cpp_across_seams() {
    if !draco_cpp_test_bridge::is_available() {
        println!("SKIP seam parity: no C++ bridge");
        return;
    }

    let samples = vec![two_triangle_seam(), split_strip(12), split_strip(40)];
    let mut compared = 0;
    let mut mismatches = Vec::new();
    let mut known_open = Vec::new();

    for sample in &samples {
        for speed in 0..=10 {
            let rust = encode_rust(sample, speed);
            let Some(cpp) = draco_cpp_test_bridge::encode_cpp_mesh_seamed(
                &sample.positions,
                &sample.position_map,
                &sample.uvs,
                &sample.uv_map,
                &sample.faces,
                speed,
                speed,
                POSITION_BITS,
                UV_BITS,
            ) else {
                mismatches.push(format!("{} speed {speed}: C++ encode failed", sample.name));
                continue;
            };
            compared += 1;
            let Some(offset) = first_difference(&rust, &cpp) else {
                continue;
            };
            let note = format!(
                "{} speed {speed}: C++ {} bytes, Rust {} bytes, first difference at {offset}",
                sample.name,
                cpp.len(),
                rust.len(),
            );
            if speed < ASSERTED_FROM_SPEED {
                known_open.push(note);
            } else {
                mismatches.push(note);
            }
        }
    }

    println!(
        "compared {compared} seamed encodes across {} samples",
        samples.len()
    );
    if !known_open.is_empty() {
        println!(
            "known open, speeds below {ASSERTED_FROM_SPEED} ({} cases):
{}",
            known_open.len(),
            known_open.join(
                "
"
            )
        );
    }
    assert!(
        mismatches.is_empty(),
        "encoder output differs from C++ Draco on meshes with attribute seams:\n{}",
        mismatches.join("\n")
    );
}

/// The C++ decoder, on a Rust-encoded seamed stream.
///
/// Byte parity implies this, but it is the assertion that says what actually
/// went wrong when the traversal was missing: values came back attached to the
/// wrong points, which a byte comparison reports only as a size difference.
#[test]
fn cpp_decoder_reads_rust_seamed_uvs() {
    if !draco_cpp_test_bridge::is_available() {
        println!("SKIP seam interop: no C++ bridge");
        return;
    }

    let samples = vec![two_triangle_seam(), split_strip(12), split_strip(40)];
    let mut mismatches = Vec::new();

    for sample in &samples {
        for speed in 0..=10 {
            let payload = encode_rust(sample, speed);
            let (Some(positions), Some(uvs)) = (
                draco_cpp_test_bridge::decode_cpp_attribute_values(
                    &payload,
                    draco_cpp_test_bridge::cpp_attribute::POSITION,
                ),
                draco_cpp_test_bridge::decode_cpp_attribute_values(
                    &payload,
                    draco_cpp_test_bridge::cpp_attribute::TEX_COORD,
                ),
            ) else {
                mismatches.push(format!(
                    "{} speed {speed}: C++ refused the Rust stream",
                    sample.name
                ));
                continue;
            };

            // Every decoded point must reproduce one of the input's
            // (position, uv) pairings. A wrong traversal keeps both sets intact
            // and only mispairs them, so checking either alone sees nothing.
            let wrong = positions
                .as_chunks::<3>()
                .0
                .iter()
                .zip(uvs.as_chunks::<2>().0)
                .filter(|(p, u)| !is_input_pairing(sample, *p, *u))
                .count();
            if wrong > 0 {
                mismatches.push(format!(
                    "{} speed {speed}: {wrong} of {} points carry a pairing not in the input",
                    sample.name,
                    positions.len() / 3
                ));
            }
        }
    }

    assert!(
        mismatches.is_empty(),
        "the C++ decoder does not read back the pairings the Rust encoder was given:\n{}",
        mismatches.join("\n")
    );
}

/// Whether a decoded (position, uv) pair is one the sample was built from.
/// Positions are quantized to 14 bits and UVs to 12 over a unit range, so the
/// tolerance is far above the quantization step and far below the gap between
/// two distinct input values.
fn is_input_pairing(sample: &SeamSample, position: &[f32], uv: &[f32]) -> bool {
    (0..sample.position_map.len()).any(|point| {
        let v = sample.position_map[point] as usize;
        let w = sample.uv_map[point] as usize;
        (0..3).all(|c| (sample.positions[v * 3 + c] - position[c]).abs() < 0.01)
            && (0..2).all(|c| (sample.uvs[w * 2 + c] - uv[c]).abs() < 0.01)
    })
}
