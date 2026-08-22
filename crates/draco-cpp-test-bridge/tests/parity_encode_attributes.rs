//! Byte parity with the C++ encoder on meshes that carry attributes.
//!
//! `parity_encode_bytes` compares positions only, which is the part with the
//! least room to diverge: the interesting prediction schemes belong to normals
//! (octahedral transform) and texture coordinates, and neither is reachable
//! through a bridge that takes positions and faces alone.
//!
//! Every case runs at all eleven speed settings and every one of them is
//! asserted: a difference fails the test and names the byte it happened at.
//! That includes speeds 0 to 3, where the two schemes reserved for them --
//! geometric normal for normals, portable tex coords for texture coordinates --
//! decide the bytes.
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use draco_cpp_test_bridge::CppMeshAttributes;

/// Every speed is asserted. Kept as a named constant so that narrowing the
/// guarantee again is a visible edit rather than a quiet one.
const ASSERTED_FROM_SPEED: i32 = 0;

const POSITION_BITS: i32 = 14;
const NORMAL_BITS: i32 = 10;
const UV_BITS: i32 = 12;
const COLOR_BITS: i32 = 8;

struct Sample {
    name: &'static str,
    positions: Vec<f32>,
    faces: Vec<u32>,
    normals: Option<Vec<f32>>,
    uvs: Option<Vec<f32>>,
    colors: Option<Vec<u8>>,
}

/// A grid, whose interior vertices all reach maximum valence.
fn grid(n: usize, with: (bool, bool, bool)) -> Sample {
    let (want_normals, want_uvs, want_colors) = with;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut colors = Vec::new();
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32, y as f32);
            positions.extend([fx, fy, ((x * y) % 7) as f32 * 0.1]);
            // Not axis aligned, so the octahedral transform has real work.
            let len = (fx * fx + fy * fy + 4.0).sqrt();
            normals.extend([fx / len, fy / len, 2.0 / len]);
            uvs.extend([fx / n as f32, fy / n as f32]);
            colors.extend([
                (x * 9 % 256) as u8,
                (y * 13 % 256) as u8,
                ((x + y) * 7 % 256) as u8,
                255,
            ]);
        }
    }
    let mut faces = Vec::new();
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let i = (y * n + x) as u32;
            let n32 = n as u32;
            faces.extend([i, i + 1, i + n32]);
            faces.extend([i + 1, i + n32 + 1, i + n32]);
        }
    }
    Sample {
        name: "grid",
        positions,
        faces,
        normals: want_normals.then_some(normals),
        uvs: want_uvs.then_some(uvs),
        colors: want_colors.then_some(colors),
    }
}

/// A fan, whose valence is anything but uniform.
fn fan(n: usize) -> Sample {
    let mut positions = vec![0.0, 0.0, 0.0];
    let mut normals = vec![0.0, 0.0, 1.0];
    let mut uvs = vec![0.5, 0.5];
    let mut colors = vec![10u8, 20, 30, 255];
    for i in 0..n {
        let a = i as f32 * 0.37;
        positions.extend([a.cos(), a.sin(), (i % 3) as f32 * 0.25]);
        let len = (1.0 + (i % 5) as f32).sqrt();
        normals.extend([a.sin() / len, a.cos() / len, 1.0 / len]);
        uvs.extend([a.cos() * 0.5 + 0.5, a.sin() * 0.5 + 0.5]);
        colors.extend([(i * 3 % 256) as u8, (i * 5 % 256) as u8, 128, 200]);
    }
    let mut faces = Vec::new();
    for i in 1..n as u32 {
        faces.extend([0u32, i, i + 1]);
    }
    Sample {
        name: "fan",
        positions,
        faces,
        normals: Some(normals),
        uvs: Some(uvs),
        colors: Some(colors),
    }
}

fn add_float_attribute(
    mesh: &mut Mesh,
    kind: GeometryAttributeType,
    components: u8,
    values: &[f32],
    num_points: usize,
) {
    let mut attribute = PointAttribute::new();
    attribute.init(kind, components, DataType::Float32, false, num_points);
    let stride = components as usize;
    for point in 0..num_points {
        for component in 0..stride {
            let offset = (point * stride + component) * 4;
            attribute.buffer_mut().update(
                &values[point * stride + component].to_le_bytes(),
                Some(offset),
            );
        }
    }
    mesh.add_attribute(attribute);
}

/// The Rust mesh, built to mirror what the bridge builds on the C++ side:
/// the same attributes, the same component types, added in the same order.
fn build_rust_mesh(sample: &Sample) -> Mesh {
    let num_points = sample.positions.len() / 3;
    let num_faces = sample.faces.len() / 3;

    let mut mesh = Mesh::new();
    mesh.set_num_points(num_points);
    mesh.set_num_faces(num_faces);

    add_float_attribute(
        &mut mesh,
        GeometryAttributeType::Position,
        3,
        &sample.positions,
        num_points,
    );
    if let Some(normals) = &sample.normals {
        add_float_attribute(
            &mut mesh,
            GeometryAttributeType::Normal,
            3,
            normals,
            num_points,
        );
    }
    if let Some(uvs) = &sample.uvs {
        add_float_attribute(
            &mut mesh,
            GeometryAttributeType::TexCoord,
            2,
            uvs,
            num_points,
        );
    }
    if let Some(colors) = &sample.colors {
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Color,
            4,
            DataType::Uint8,
            true,
            num_points,
        );
        for point in 0..num_points {
            attribute
                .buffer_mut()
                .update(&colors[point * 4..point * 4 + 4], Some(point * 4));
        }
        mesh.add_attribute(attribute);
    }

    for face in 0..num_faces {
        mesh.set_face(
            FaceIndex(face as u32),
            [
                PointIndex(sample.faces[face * 3]),
                PointIndex(sample.faces[face * 3 + 1]),
                PointIndex(sample.faces[face * 3 + 2]),
            ],
        );
    }
    mesh
}

fn encode_rust(sample: &Sample, speed: i32) -> Vec<u8> {
    encode_rust_at(sample, speed, speed)
}

fn encode_rust_at(sample: &Sample, encoding_speed: i32, decoding_speed: i32) -> Vec<u8> {
    let mesh = build_rust_mesh(sample);
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", encoding_speed);
    options.set_global_int("decoding_speed", decoding_speed);

    let mut attribute_id = 0;
    options.set_attribute_int(attribute_id, "quantization_bits", POSITION_BITS);
    if sample.normals.is_some() {
        attribute_id += 1;
        options.set_attribute_int(attribute_id, "quantization_bits", NORMAL_BITS);
    }
    if sample.uvs.is_some() {
        attribute_id += 1;
        options.set_attribute_int(attribute_id, "quantization_bits", UV_BITS);
    }
    if sample.colors.is_some() {
        attribute_id += 1;
        options.set_attribute_int(attribute_id, "quantization_bits", COLOR_BITS);
    }

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    let _ = encoder.encode(&options, &mut buffer);
    buffer.data().to_vec()
}

fn encode_cpp(sample: &Sample, speed: i32) -> Option<Vec<u8>> {
    draco_cpp_test_bridge::encode_cpp_mesh_attributed(
        &sample.positions,
        &sample.faces,
        CppMeshAttributes {
            normals: sample.normals.as_deref(),
            uvs: sample.uvs.as_deref(),
            colors: sample.colors.as_deref(),
            normal_bits: NORMAL_BITS,
            uv_bits: UV_BITS,
            color_bits: COLOR_BITS,
        },
        speed,
        speed,
        POSITION_BITS,
    )
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

fn label(sample: &Sample) -> String {
    let mut carried = Vec::new();
    if sample.normals.is_some() {
        carried.push("normals");
    }
    if sample.uvs.is_some() {
        carried.push("uvs");
    }
    if sample.colors.is_some() {
        carried.push("colors");
    }
    if carried.is_empty() {
        format!("{} (positions only)", sample.name)
    } else {
        format!("{} + {}", sample.name, carried.join(", "))
    }
}

#[test]
fn encoder_output_matches_cpp_with_attributes() {
    if !draco_cpp_test_bridge::is_available() {
        println!("SKIP encoder attribute parity: no C++ bridge");
        return;
    }

    let samples = vec![
        grid(10, (true, false, false)),
        grid(10, (false, true, false)),
        grid(10, (false, false, true)),
        grid(12, (true, true, true)),
        fan(48),
        // 1682 faces, past the num_faces < 1000 cutoff upstream calls a tiny
        // mesh. Below speed 5 that switches the connectivity encoder from
        // MESH_EDGEBREAKER_STANDARD to MESH_EDGEBREAKER_VALENCE, which none of
        // the smaller samples reach.
        grid(30, (true, false, false)),
    ];

    let mut compared = 0;
    let mut mismatches = Vec::new();
    let mut known_open = Vec::new();

    for sample in &samples {
        for speed in 0..=10 {
            let rust = encode_rust(sample, speed);
            let Some(cpp) = encode_cpp(sample, speed) else {
                mismatches.push(format!(
                    "{} speed {speed}: C++ encode failed",
                    label(sample)
                ));
                continue;
            };
            compared += 1;
            let Some(offset) = first_difference(&rust, &cpp) else {
                continue;
            };
            let note = format!(
                "{} speed {speed}: C++ {} bytes, Rust {} bytes, first difference at {offset}",
                label(sample),
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
        "compared {compared} encodes across {} samples",
        samples.len()
    );
    if !known_open.is_empty() {
        println!(
            "known open, speeds below {ASSERTED_FROM_SPEED} ({} cases):\n{}",
            known_open.len(),
            known_open.join("\n")
        );
    }
    assert!(
        mismatches.is_empty(),
        "encoder output differs from C++ Draco:\n{}",
        mismatches.join("\n")
    );
}

/// The decoders, on the same payload.
///
/// Bytes are the encoder's business. This is the reader's: given one file, do
/// the two implementations return the same mesh?
///
/// Compared point for point, by index. An earlier version paired points by
/// position on the belief that the two decoders number them differently once
/// edgebreaker is in play; they do not, and that belief came from reading Rust
/// attributes by entry instead of through their point map. Position pairing is
/// the weaker test -- it cannot see an attribute attached to the wrong point,
/// which is exactly the defect this is here to catch.
#[test]
fn decoders_agree_on_the_same_payload() {
    if !draco_cpp_test_bridge::is_available() {
        println!("SKIP decoder parity: no C++ bridge");
        return;
    }

    let samples = vec![
        grid(10, (true, false, false)),
        grid(12, (true, true, true)),
        fan(48),
    ];

    let mut mismatches = Vec::new();

    for sample in &samples {
        for speed in 0..=10 {
            let Some(payload) = encode_cpp(sample, speed) else {
                continue;
            };

            let Some(rust_mesh) = decode_with_rust(&payload) else {
                mismatches.push(format!(
                    "{} speed {speed}: Rust decoder refused",
                    label(sample)
                ));
                continue;
            };
            let rust_positions = read_rust(&rust_mesh, GeometryAttributeType::Position);
            let rust_normals = read_rust(&rust_mesh, GeometryAttributeType::Normal);
            let (Some(cpp_positions), Some(cpp_normals)) = (
                draco_cpp_test_bridge::decode_cpp_attribute_values(
                    &payload,
                    draco_cpp_test_bridge::cpp_attribute::POSITION,
                ),
                draco_cpp_test_bridge::decode_cpp_attribute_values(
                    &payload,
                    draco_cpp_test_bridge::cpp_attribute::NORMAL,
                ),
            ) else {
                mismatches.push(format!(
                    "{} speed {speed}: C++ decoder refused",
                    label(sample)
                ));
                continue;
            };

            if rust_positions.len() != cpp_positions.len()
                || rust_normals.len() != cpp_normals.len()
            {
                mismatches.push(format!(
                    "{} speed {speed}: {} points vs {}",
                    label(sample),
                    rust_positions.len() / 3,
                    cpp_positions.len() / 3
                ));
                continue;
            }

            let worst_position = rust_positions
                .iter()
                .zip(&cpp_positions)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            if worst_position > 1e-6 {
                mismatches.push(format!(
                    "{} speed {speed}: point {} differs in position by up to {worst_position:.6}",
                    label(sample),
                    rust_positions
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .zip(cpp_positions.as_chunks::<3>().0)
                        .position(|(a, b)| (0..3).any(|c| (a[c] - b[c]).abs() > 1e-6))
                        .unwrap_or(0)
                ));
                continue;
            }

            let worst = rust_normals
                .iter()
                .zip(&cpp_normals)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            if worst > 1e-6 {
                mismatches.push(format!(
                    "{} speed {speed}: same point, normals differ by up to {worst:.6}",
                    label(sample)
                ));
            }
        }
    }

    assert!(
        mismatches.is_empty(),
        "the two decoders return different meshes for the same payload:\n{}",
        mismatches.join("\n")
    );
}

fn decode_with_rust(payload: &[u8]) -> Option<Mesh> {
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::mesh_decoder::MeshDecoder;
    let mut mesh = Mesh::new();
    let mut buffer = DecoderBuffer::new(payload);
    MeshDecoder::new().decode(&mut buffer, &mut mesh).ok()?;
    Some(mesh)
}

fn read_rust(mesh: &Mesh, kind: GeometryAttributeType) -> Vec<f32> {
    let id = mesh.named_attribute_id(kind);
    if id < 0 {
        return Vec::new();
    }
    let attribute = mesh.attribute(id);
    let components = attribute.num_components() as usize;
    let stride = attribute.byte_stride() as usize;
    let mut values = Vec::new();
    for point in 0..mesh.num_points() {
        // Through the attribute's own point map, never point * stride. Two
        // attributes of the same mesh do not share one point-to-entry
        // permutation: at speed 0 the position is walked by prediction degree
        // and everything else depth first. Reading by entry silently pairs one
        // point's position with another point's normal, which looks exactly
        // like a decoder bug and is not one.
        let entry = attribute.mapped_index(PointIndex(point as u32));
        let mut bytes = vec![0u8; stride];
        attribute
            .buffer()
            .read(entry.0 as usize * stride, &mut bytes);
        for component in 0..components {
            let start = component * 4;
            values.push(f32::from_le_bytes(
                bytes[start..start + 4].try_into().unwrap(),
            ));
        }
    }
    values
}

/// The C++ decoder, on a Rust-encoded stream.
///
/// Byte parity is the strong statement, but it only holds where both encoders
/// choose the same scheme. This asks the weaker, independent question: whatever
/// we wrote, does the reference implementation read our normals back? A
/// prediction scheme whose encoder and decoder disagree produces plausible bytes
/// and wrong vectors, which a byte comparison shows only as a size difference.
#[test]
fn cpp_decoder_reads_rust_normals() {
    if !draco_cpp_test_bridge::is_available() {
        println!("SKIP: no C++ bridge");
        return;
    }

    // The grid only. Points come back renumbered, so the input normal has to be
    // found by position, and that needs positions a lookup can tell apart -- the
    // fan winds through nearly three turns and stacks vertices almost on top of
    // each other, where the nearest match is a coin toss rather than a defect.
    let samples = vec![grid(10, (true, false, false)), grid(12, (true, true, true))];
    let mut failures = Vec::new();

    for sample in &samples {
        let normals = sample.normals.as_ref().expect("sample carries normals");
        for speed in 0..=10 {
            let payload = encode_rust(sample, speed);
            let (Some(positions), Some(decoded)) = (
                draco_cpp_test_bridge::decode_cpp_attribute_values(
                    &payload,
                    draco_cpp_test_bridge::cpp_attribute::POSITION,
                ),
                draco_cpp_test_bridge::decode_cpp_attribute_values(
                    &payload,
                    draco_cpp_test_bridge::cpp_attribute::NORMAL,
                ),
            ) else {
                failures.push(format!(
                    "{} speed {speed}: C++ refused the Rust stream",
                    label(sample)
                ));
                continue;
            };
            if decoded.len() != normals.len() {
                failures.push(format!(
                    "{} speed {speed}: {} normals back, {} in",
                    label(sample),
                    decoded.len() / 3,
                    normals.len() / 3
                ));
                continue;
            }

            // Points come back renumbered, so pair them by position against the
            // input, which the samples build on a lattice we can look up.
            let mut worst = 0.0f32;
            for (p, n) in positions
                .as_chunks::<3>()
                .0
                .iter()
                .zip(decoded.as_chunks::<3>().0)
            {
                let Some(want) = nearest_input_normal(sample, p) else {
                    continue;
                };
                for c in 0..3 {
                    worst = worst.max((n[c] - want[c]).abs());
                }
            }
            // Normals are quantized to 10 bits over the octahedron, so a step is
            // a few thousandths; anything past 0.05 is a different vector.
            if worst > 0.05 {
                failures.push(format!(
                    "{} speed {speed}: normals off by up to {worst:.4}",
                    label(sample)
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "the C++ decoder does not read back what the Rust encoder wrote:\n{}",
        failures.join("\n")
    );
}

/// The input normal of whichever sample point sits closest to `position`.
fn nearest_input_normal(sample: &Sample, position: &[f32]) -> Option<[f32; 3]> {
    let normals = sample.normals.as_ref()?;
    let mut best = None;
    let mut best_distance = f32::MAX;
    for (i, p) in sample.positions.as_chunks::<3>().0.iter().enumerate() {
        let d = (0..3).map(|c| (p[c] - position[c]).powi(2)).sum::<f32>();
        if d < best_distance {
            best_distance = d;
            best = Some([normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]]);
        }
    }
    best
}

/// A normal attribute that is already integral has no octahedral quantization,
/// so the geometric-normal scheme cannot predict it. Upstream reaches this
/// combination and asks a wrap transform for quantization bits -- a stub behind
/// a failed assertion. This must fall back to a delta and encode.
#[test]
fn integer_normals_encode_and_round_trip() {
    let num_points = 25usize;
    let n = 5usize;
    let mut mesh = Mesh::new();
    mesh.set_num_points(num_points);

    let mut positions = Vec::new();
    for y in 0..n {
        for x in 0..n {
            positions.extend([x as f32, y as f32, ((x * y) % 3) as f32 * 0.25]);
        }
    }
    add_float_attribute(
        &mut mesh,
        GeometryAttributeType::Position,
        3,
        &positions,
        num_points,
    );

    // Int32 normals, and deliberately no quantization_bits for them.
    let mut normal = PointAttribute::new();
    normal.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Int32,
        false,
        num_points,
    );
    for point in 0..num_points {
        for component in 0..3 {
            let value = ((point + component) % 7) as i32 - 3;
            normal
                .buffer_mut()
                .update(&value.to_le_bytes(), Some((point * 3 + component) * 4));
        }
    }
    mesh.add_attribute(normal);

    let mut faces = Vec::new();
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let i = (y * n + x) as u32;
            let n32 = n as u32;
            faces.extend([i, i + 1, i + n32]);
            faces.extend([i + 1, i + n32 + 1, i + n32]);
        }
    }
    mesh.set_num_faces(faces.len() / 3);
    for face in 0..faces.len() / 3 {
        mesh.set_face(
            FaceIndex(face as u32),
            [
                PointIndex(faces[face * 3]),
                PointIndex(faces[face * 3 + 1]),
                PointIndex(faces[face * 3 + 2]),
            ],
        );
    }

    // Speed 0 is where the normal attribute selects geometric normal.
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", 0);
    options.set_global_int("decoding_speed", 0);
    options.set_attribute_int(0, "quantization_bits", POSITION_BITS);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("an integer normal attribute must encode, not fail selection");

    let mut decoded = Mesh::new();
    let mut input = draco_core::decoder_buffer::DecoderBuffer::new(buffer.data());
    draco_core::mesh_decoder::MeshDecoder::new()
        .decode(&mut input, &mut decoded)
        .expect("and it must decode again");
    assert_eq!(decoded.num_points(), num_points);
}

/// The two speed knobs, set apart.
///
/// Draco takes the LARGER of the encoding and decoding speeds wherever it asks
/// "how fast", so a caller who wants slow encoding but fast decoding gets the
/// fast behaviour. Every other case in this file sets both to the same value,
/// which makes the distinction invisible -- and makes it easy to reach for the
/// encoding speed alone and never notice.
#[test]
fn asymmetric_speeds_match_cpp() {
    if !draco_cpp_test_bridge::is_available() {
        println!("SKIP asymmetric speeds: no C++ bridge");
        return;
    }

    let samples = vec![grid(10, (true, false, false)), grid(12, (true, true, true))];
    let pairs = [(0, 10), (10, 0), (0, 5), (5, 0), (2, 7), (7, 2)];

    let mut mismatches = Vec::new();
    let mut compared = 0;

    for sample in &samples {
        for (encoding_speed, decoding_speed) in pairs {
            let rust = encode_rust_at(sample, encoding_speed, decoding_speed);
            let Some(cpp) = draco_cpp_test_bridge::encode_cpp_mesh_attributed(
                &sample.positions,
                &sample.faces,
                CppMeshAttributes {
                    normals: sample.normals.as_deref(),
                    uvs: sample.uvs.as_deref(),
                    colors: sample.colors.as_deref(),
                    normal_bits: NORMAL_BITS,
                    uv_bits: UV_BITS,
                    color_bits: COLOR_BITS,
                },
                encoding_speed,
                decoding_speed,
                POSITION_BITS,
            ) else {
                mismatches.push(format!(
                    "{} enc {encoding_speed} dec {decoding_speed}: C++ encode failed",
                    label(sample)
                ));
                continue;
            };
            compared += 1;
            if let Some(offset) = first_difference(&rust, &cpp) {
                mismatches.push(format!(
                    "{} enc {encoding_speed} dec {decoding_speed}: C++ {} bytes, Rust {} bytes, first difference at {offset}",
                    label(sample),
                    cpp.len(),
                    rust.len(),
                ));
            }
        }
    }

    println!("compared {compared} encodes with the two speeds set apart");
    assert!(
        mismatches.is_empty(),
        "encoder output differs from C++ Draco when the speeds differ:
{}",
        mismatches.join(
            "
"
        )
    );
}
