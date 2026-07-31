//! Byte parity with the C++ encoder on point clouds.
//!
//! The rest of the parity suite is meshes. Point clouds take a different
//! encoder entirely -- two of them, sequential and kd-tree -- and which one runs
//! is decided by a rule rather than by the caller: with no explicit method,
//! Draco picks kd-tree for any cloud whose attributes it can handle, at any
//! speed below 10.
//!
//! Every case runs at all eleven speeds and every one is asserted.
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::PointIndex;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;
use draco_cpp_test_bridge::CppPointCloudAttributes;

/// Every speed is asserted. Kept as a named constant so that narrowing the
/// guarantee again is a visible edit rather than a quiet one.
const ASSERTED_FROM_SPEED: i32 = 0;

const POSITION_BITS: i32 = 14;
const NORMAL_BITS: i32 = 10;
const COLOR_BITS: i32 = 8;

struct Sample {
    name: &'static str,
    positions: Vec<f32>,
    normals: Option<Vec<f32>>,
    colors: Option<Vec<u8>>,
}

/// A lattice with a little irregularity, so the kd-tree splits are not all
/// degenerate. `n` cubed points.
fn lattice(n: usize, with: (bool, bool)) -> Sample {
    let (want_normals, want_colors) = with;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let (fx, fy, fz) = (x as f32, y as f32, z as f32);
                positions.extend([fx, fy, fz + ((x * y) % 5) as f32 * 0.1]);
                let len = (fx * fx + fy * fy + 4.0).sqrt();
                normals.extend([fx / len, fy / len, 2.0 / len]);
                colors.extend([
                    (x * 9 % 256) as u8,
                    (y * 13 % 256) as u8,
                    (z * 7 % 256) as u8,
                ]);
            }
        }
    }
    Sample {
        name: if n * n * n > 64 {
            "lattice, over 64 points"
        } else {
            "lattice"
        },
        positions,
        normals: want_normals.then_some(normals),
        colors: want_colors.then_some(colors),
    }
}

fn build_rust_point_cloud(sample: &Sample) -> PointCloud {
    let num_points = sample.positions.len() / 3;
    let mut pc = PointCloud::new();
    pc.set_num_points(num_points);

    let add_floats = |pc: &mut PointCloud, kind, components: usize, values: &[f32]| {
        let mut attribute = PointAttribute::new();
        attribute.init(kind, components as u8, DataType::Float32, false, num_points);
        for (i, value) in values.iter().enumerate() {
            attribute
                .buffer_mut()
                .update(&value.to_le_bytes(), Some(i * 4));
        }
        pc.add_attribute(attribute);
    };

    add_floats(
        &mut pc,
        GeometryAttributeType::Position,
        3,
        &sample.positions,
    );
    if let Some(normals) = &sample.normals {
        add_floats(&mut pc, GeometryAttributeType::Normal, 3, normals);
    }
    if let Some(colors) = &sample.colors {
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Color,
            3,
            DataType::Uint8,
            true,
            num_points,
        );
        for (i, value) in colors.iter().enumerate() {
            attribute.buffer_mut().update(&[*value], Some(i));
        }
        pc.add_attribute(attribute);
    }
    pc
}

fn encode_rust(sample: &Sample, method: Option<i32>, speed: i32) -> Vec<u8> {
    let pc = build_rust_point_cloud(sample);
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);
    if let Some(method) = method {
        options.set_encoding_method(method);
    }

    let mut attribute_id = 0;
    options.set_attribute_int(attribute_id, "quantization_bits", POSITION_BITS);
    if sample.normals.is_some() {
        attribute_id += 1;
        options.set_attribute_int(attribute_id, "quantization_bits", NORMAL_BITS);
    }
    if sample.colors.is_some() {
        attribute_id += 1;
        options.set_attribute_int(attribute_id, "quantization_bits", COLOR_BITS);
    }

    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);
    let mut buffer = EncoderBuffer::new();
    let _ = encoder.encode(&options, &mut buffer);
    buffer.data().to_vec()
}

fn encode_cpp(sample: &Sample, method: Option<i32>, speed: i32) -> Option<Vec<u8>> {
    draco_cpp_test_bridge::encode_cpp_point_cloud(
        &sample.positions,
        CppPointCloudAttributes {
            normals: sample.normals.as_deref(),
            colors: sample.colors.as_deref(),
            normal_bits: NORMAL_BITS,
            color_bits: COLOR_BITS,
        },
        method,
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

fn label(sample: &Sample, method: Option<i32>) -> String {
    let mut carried = Vec::new();
    if sample.normals.is_some() {
        carried.push("normals");
    }
    if sample.colors.is_some() {
        carried.push("colors");
    }
    let attributes = if carried.is_empty() {
        "positions only".to_string()
    } else {
        carried.join(", ")
    };
    let method = match method {
        None => "method chosen by the rule",
        Some(0) => "sequential",
        Some(1) => "kd-tree",
        Some(other) => return format!("{} + {attributes}, method {other}", sample.name),
    };
    format!("{} + {attributes}, {method}", sample.name)
}

/// Every attribute value in the stream, canonicalized so that two clouds
/// holding the same points compare equal whatever order they are stored in.
/// `None` if the stream does not decode.
fn decoded_points(bytes: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut buffer = DecoderBuffer::new(bytes);
    let mut pc = PointCloud::new();
    PointCloudDecoder::new().decode(&mut buffer, &mut pc).ok()?;

    let mut points: Vec<Vec<u8>> = (0..pc.num_points())
        .map(|point| {
            let mut row = Vec::new();
            for id in 0..pc.num_attributes() {
                let attribute = pc.attribute(id);
                let stride = attribute.byte_stride() as usize;
                let index = attribute.mapped_index(PointIndex(point as u32)).0 as usize;
                row.extend_from_slice(&attribute.buffer().data()[index * stride..][..stride]);
            }
            row
        })
        .collect();
    points.sort_unstable();
    Some(points)
}

/// The method byte a Draco header carries, at offset 8.
fn encoding_method_byte(bytes: &[u8]) -> Option<u8> {
    (bytes.len() > 8 && &bytes[0..5] == b"DRACO" && bytes[7] == 0).then(|| bytes[8])
}

#[test]
fn encoder_output_matches_cpp_for_point_clouds() {
    if !draco_cpp_test_bridge::is_available() {
        println!("SKIP point cloud parity: no C++ bridge");
        return;
    }

    let samples = vec![
        lattice(3, (false, false)),
        lattice(3, (true, false)),
        lattice(3, (false, true)),
        lattice(5, (true, true)),
    ];

    let mut compared = 0;
    let mut mismatches = Vec::new();
    let mut known_open = Vec::new();
    let mut methods_seen = std::collections::BTreeSet::new();

    for sample in &samples {
        // `None` exercises the selection rule itself, which is the part most
        // likely to differ and the part no caller sets explicitly.
        for method in [None, Some(0), Some(1)] {
            for speed in 0..=10 {
                let rust = encode_rust(sample, method, speed);
                let Some(cpp) = encode_cpp(sample, method, speed) else {
                    mismatches.push(format!(
                        "{} speed {speed}: C++ encode failed",
                        label(sample, method)
                    ));
                    continue;
                };
                compared += 1;
                if let Some(byte) = encoding_method_byte(&cpp) {
                    methods_seen.insert(byte);
                }

                // Bytes are the guarantee, but a stream C++ cannot read at all
                // is a different and worse failure than one that differs, and
                // on the KD-tree path a difference is just as likely to be a
                // reordering as corruption. Decoding every case keeps the two
                // apart instead of leaving a byte offset to interpret.
                let rust_read_by_cpp =
                    draco_cpp_test_bridge::decode_cpp_point_cloud_fingerprint(&rust);
                let cpp_read_by_cpp =
                    draco_cpp_test_bridge::decode_cpp_point_cloud_fingerprint(&cpp);
                match (&rust_read_by_cpp, &cpp_read_by_cpp) {
                    (Some(from_rust), Some(from_cpp)) => {
                        if from_rust.num_points != from_cpp.num_points
                            || from_rust.num_attributes != from_cpp.num_attributes
                        {
                            mismatches.push(format!(
                                "{} speed {speed}: C++ decodes the Rust stream as {} points \
                                 and {} attributes, its own as {} and {}",
                                label(sample, method),
                                from_rust.num_points,
                                from_rust.num_attributes,
                                from_cpp.num_points,
                                from_cpp.num_attributes,
                            ));
                        }
                    }
                    (None, _) => mismatches.push(format!(
                        "{} speed {speed}: C++ cannot decode the Rust stream",
                        label(sample, method)
                    )),
                    (_, None) => mismatches.push(format!(
                        "{} speed {speed}: C++ cannot decode its own stream",
                        label(sample, method)
                    )),
                }

                let Some(offset) = first_difference(&rust, &cpp) else {
                    continue;
                };
                // The KD-tree coder is free to emit points in any order, so two
                // streams can differ byte for byte and still carry the same
                // cloud. Saying which of the two happened turns a byte offset
                // into a diagnosis. The decode fingerprint cannot answer it --
                // it hashes in point order -- so compare the points as sets.
                let note = format!(
                    "{} speed {speed}: C++ {} bytes, Rust {} bytes, first difference at \
                     {offset}, {}",
                    label(sample, method),
                    cpp.len(),
                    rust.len(),
                    match (decoded_points(&rust), decoded_points(&cpp)) {
                        (Some(from_rust), Some(from_cpp)) if from_rust == from_cpp =>
                            "same points in a different order",
                        (Some(_), Some(_)) => "different points",
                        _ => "and one of them does not decode",
                    },
                );
                if speed < ASSERTED_FROM_SPEED {
                    known_open.push(note);
                } else {
                    mismatches.push(note);
                }
            }
        }
    }

    println!(
        "compared {compared} point-cloud encodes across {} samples",
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
        "encoder output differs from C++ Draco on point clouds:\n{}",
        mismatches.join("\n")
    );

    // A suite that silently only ever exercised one encoder would pass while
    // proving half of what it claims.
    assert!(
        methods_seen.contains(&0) && methods_seen.contains(&1),
        "expected both point-cloud encoders to be reached, saw methods {methods_seen:?}"
    );
}
