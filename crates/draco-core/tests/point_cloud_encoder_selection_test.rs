//! How the point-cloud encoder picks a method and an encoder per attribute.
//!
//! Each test here pins one input that the port used to get wrong in its own
//! way, rather than a byte difference: the byte-level agreement with C++ Draco
//! is measured in `draco-cpp-test-bridge/tests/parity_point_clouds.rs`.

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::PointIndex;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;

const POSITIONS: [f32; 12] = [
    0.0, 0.0, 0.0, //
    1.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, //
    1.0, 1.0, 1.0, //
];

fn cloud_with_positions() -> PointCloud {
    let num_points = POSITIONS.len() / 3;
    let mut pc = PointCloud::new();
    pc.set_num_points(num_points);
    let mut att = PointAttribute::new();
    att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );
    for (i, value) in POSITIONS.iter().enumerate() {
        att.buffer_mut().write(i * 4, &value.to_le_bytes());
    }
    pc.add_attribute(att);
    pc
}

fn encode(pc: PointCloud, options: &EncoderOptions) -> Result<Vec<u8>, String> {
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(pc);
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(options, &mut buffer)
        .map_err(|e| format!("{e:?}"))?;
    Ok(buffer.data().to_vec())
}

fn decode(bytes: &[u8]) -> PointCloud {
    let mut buffer = DecoderBuffer::new(bytes);
    let mut out = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut buffer, &mut out)
        .expect("decode");
    out
}

/// The method byte a Draco point-cloud header carries, at offset 8.
fn method_byte(bytes: &[u8]) -> u8 {
    assert_eq!(&bytes[0..5], b"DRACO", "not a Draco stream");
    assert_eq!(bytes[7], 0, "not a point cloud");
    bytes[8]
}

/// Upstream picks the KD-tree encoder for anything it can handle below speed
/// 10, so an unset `encoding_method` is not the same as asking for sequential.
#[test]
fn an_unset_method_selects_kd_tree() {
    let mut options = EncoderOptions::new();
    options.set_attribute_int(0, "quantization_bits", 14);

    let bytes = encode(cloud_with_positions(), &options).expect("encode");
    assert_eq!(method_byte(&bytes), 1, "expected KD-tree");
}

/// ...but not at speed 10, which upstream reads as "do the cheapest thing".
#[test]
fn an_unset_method_at_speed_10_selects_sequential() {
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", 10);
    options.set_global_int("decoding_speed", 10);
    options.set_attribute_int(0, "quantization_bits", 14);

    let bytes = encode(cloud_with_positions(), &options).expect("encode");
    assert_eq!(method_byte(&bytes), 0, "expected sequential");
}

/// An attribute the KD-tree coder cannot take -- here an unquantized float --
/// drops the whole cloud back to sequential rather than failing.
#[test]
fn an_unquantized_attribute_falls_back_to_sequential() {
    let options = EncoderOptions::new();
    let bytes = encode(cloud_with_positions(), &options).expect("encode");
    assert_eq!(method_byte(&bytes), 0, "expected sequential");
}

/// The same input with the KD-tree asked for explicitly is upstream's one
/// hard error on this path.
#[test]
fn an_unquantized_attribute_rejects_an_explicit_kd_tree() {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(1);

    let error = encode(cloud_with_positions(), &options).expect_err("must not encode");
    assert!(
        error.contains("Invalid encoding method"),
        "unexpected error: {error}"
    );
}

/// Zero attributes passes every eligibility check vacuously, so the KD-tree
/// encoder is what upstream selects -- and it used to index attribute 0 and
/// panic.
#[test]
fn kd_tree_survives_a_cloud_with_no_attributes() {
    let mut pc = PointCloud::new();
    pc.set_num_points(4);
    let mut options = EncoderOptions::new();
    options.set_encoding_method(1);

    let bytes = encode(pc, &options).expect("encode");
    assert_eq!(method_byte(&bytes), 1, "expected KD-tree");
    assert_eq!(decode(&bytes).num_attributes(), 0);
}

/// Quantization is only ever applied to floats, so asking for it on an integer
/// attribute must not change how that attribute is written. The port used to
/// announce the quantizing decoder and then write raw bytes, which desyncs
/// every attribute after it.
#[test]
fn quantization_requested_on_an_integer_attribute_is_ignored() {
    let mut pc = cloud_with_positions();
    let colors: [u8; 12] = [0, 17, 250, 255, 3, 128, 64, 200, 9, 90, 180, 7];
    let mut att = PointAttribute::new();
    att.init(GeometryAttributeType::Color, 3, DataType::Uint8, true, 4);
    for (i, value) in colors.iter().enumerate() {
        att.buffer_mut().write(i, &[*value]);
    }
    pc.add_attribute(att);

    let mut options = EncoderOptions::new();
    // Sequential, so points keep their order and the comparison below can be
    // index by index.
    options.set_encoding_method(0);
    options.set_attribute_int(0, "quantization_bits", 14);
    options.set_attribute_int(1, "quantization_bits", 8);

    let decoded = decode(&encode(pc, &options).expect("encode"));
    assert_eq!(decoded.num_attributes(), 2);

    let att = decoded.attribute(1);
    assert_eq!(att.data_type(), DataType::Uint8);
    let mut round_tripped = [0u8; 12];
    for point in 0..4 {
        let index = att.mapped_index(PointIndex(point as u32)).0 as usize;
        att.buffer()
            .read(index * 3, &mut round_tripped[point * 3..point * 3 + 3]);
    }
    // Exactly, not approximately: nothing quantized these.
    assert_eq!(round_tripped, colors);
}

/// A normal attribute nobody asked to quantize has no octahedral form to be
/// written in. Upstream writes it as raw floats; the port used to select the
/// octahedral encoder on the attribute type alone and fail to initialize it.
#[test]
fn an_unquantized_normal_encodes_as_raw_floats() {
    let normals: [f32; 12] = [
        1.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, //
        0.0, 0.0, 1.0, //
        0.57735, 0.57735, 0.57735,
    ];
    let mut pc = PointCloud::new();
    pc.set_num_points(4);
    let mut att = PointAttribute::new();
    att.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        4,
    );
    for (i, value) in normals.iter().enumerate() {
        att.buffer_mut().write(i * 4, &value.to_le_bytes());
    }
    pc.add_attribute(att);

    let options = EncoderOptions::new();
    let decoded = decode(&encode(pc, &options).expect("encode"));

    let att = decoded.attribute(0);
    assert_eq!(att.attribute_type(), GeometryAttributeType::Normal);
    for (i, expected) in normals.iter().enumerate() {
        let mut bytes = [0u8; 4];
        att.buffer().read(i * 4, &mut bytes);
        // Bit exact: an untransformed attribute is copied, not approximated.
        assert_eq!(
            f32::from_le_bytes(bytes),
            *expected,
            "component {i} changed value"
        );
    }
}
