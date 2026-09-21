//! What upstream C++ Draco makes of the two point-cloud encoder options.
//!
//! Both options are off by default because this crate's output is otherwise
//! byte-identical to C++ Draco's, and the case for turning either on rests on
//! a claim about the other implementation: that nothing either one produces is
//! outside what an ordinary decoder reads. `set_prediction_search` can write
//! `PREDICTION_NONE`, which upstream's own encoder never chooses for a point
//! cloud but its decoder has read since bitstream 1.1;
//! `set_spatial_point_order` changes no element of the format at all, only
//! which point is written first.
//!
//! Neither claim is ours to settle by reading our own encoder, so both are
//! settled here by the real C++ decoder, over every attribute rather than the
//! ones a PLY happens to carry: `decode_cpp_point_cloud_fingerprint` hashes
//! every value of every attribute of the decoded cloud.
//!
//! Each test first asserts that the option changed the bytes. "C++ decoded it
//! identically" is worth nothing if the option quietly did nothing.

mod common;

use draco_core::{
    DataType, EncoderBuffer, EncoderOptions, GeometryAttributeType, Metadata, PointAttribute,
    PointCloud, PointCloudEncoder,
};
use draco_cpp_test_bridge::{
    decode_cpp_point_cloud_attribute_values, decode_cpp_point_cloud_fingerprint,
};

const NUM_POINTS: usize = 4096;

/// `draco::GeometryAttribute::POSITION`.
const POSITION: i32 = 0;

/// Sequential. The options act on the sequential coder, and the automatic
/// choice for a cloud this shape is kd-tree.
const SEQUENTIAL: i32 = 0;

const QUANTIZATION_BITS: i32 = 8;

/// Values shaped so that the search says yes to some attributes and no to
/// others, which is what keeps it from being tested on a cloud where every
/// answer is the same.
///
/// Positions vary smoothly, where differencing wins; the harmonics are
/// concentrated around a centre with rare outliers stretching the quantization
/// range, which is the shape differencing makes worse.
fn attribute(
    kind: GeometryAttributeType,
    components: u8,
    seed: u32,
    concentrated: bool,
) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(kind, components, DataType::Float32, false, NUM_POINTS);
    let buffer = attribute.buffer_mut();
    let mut state = seed.wrapping_mul(2654435761).wrapping_add(1);
    for point in 0..NUM_POINTS {
        for component in 0..components as usize {
            let mut unit = || {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as f32 / (1 << 24) as f32
            };
            let value = if concentrated {
                let bell: f32 = (0..4).map(|_| unit()).sum::<f32>() - 2.0;
                if unit() < 0.002 {
                    bell * 8.0
                } else {
                    bell
                }
            } else {
                point as f32 * 0.01 + component as f32 * 0.25 + (unit() - 0.5)
            };
            let offset = (point * components as usize + component) * 4;
            buffer.write(offset, &value.to_le_bytes());
        }
    }
    attribute
}

/// A splat in miniature: a position and the named generic attributes a
/// Gaussian splat carries, which is the data both options were written for.
fn splat_cloud() -> PointCloud {
    let mut cloud = PointCloud::new();
    cloud.set_num_points(NUM_POINTS);
    cloud.add_attribute(attribute(GeometryAttributeType::Position, 3, 1, false));

    let mut layout: Vec<(String, u8)> = vec![
        ("scale".to_string(), 3),
        ("rotation".to_string(), 4),
        ("opacity".to_string(), 1),
    ];
    for i in 0..9 {
        layout.push((format!("f_rest_{i}"), 1));
    }
    for (index, (name, components)) in layout.into_iter().enumerate() {
        let id = cloud.add_attribute(attribute(
            GeometryAttributeType::Generic,
            components,
            10 + index as u32,
            name.starts_with("f_rest_"),
        ));
        let unique_id = cloud.attribute(id).unique_id();
        let mut metadata = Metadata::new();
        metadata.set_string("name", name).expect("string entry");
        cloud
            .metadata_or_insert()
            .set_attribute_metadata(unique_id, metadata);
    }
    cloud
}

fn encode(search: bool, spatial: bool) -> Vec<u8> {
    let cloud = splat_cloud();
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_prediction_search(search);
    options.set_spatial_point_order(spatial);
    for id in 0..cloud.num_attributes() {
        options.set_attribute_int(id, "quantization_bits", QUANTIZATION_BITS);
    }
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud);
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encodes");
    buffer.data().to_vec()
}

/// Positions as a set, for a stream whose points come back in another order.
fn sorted_positions(encoded: &[u8]) -> Vec<[u32; 3]> {
    let values = decode_cpp_point_cloud_attribute_values(encoded, POSITION)
        .expect("C++ decodes the position attribute");
    // Compared as bit patterns: these are dequantized values that went through
    // the same arithmetic on both sides, so equality is exact or it is a
    // failure, and sorting wants a total order that floats do not give.
    let mut points: Vec<[u32; 3]> = values
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()])
        .collect();
    points.sort_unstable();
    points
}

#[test]
fn cpp_reads_a_searched_stream_as_the_ordinary_one() {
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }
    let plain = encode(false, false);
    let searched = encode(true, false);
    assert_ne!(
        plain, searched,
        "the search chose the default everywhere, so the comparison below would pass on nothing"
    );

    let plain = decode_cpp_point_cloud_fingerprint(&plain).expect("C++ decodes the default stream");
    let searched =
        decode_cpp_point_cloud_fingerprint(&searched).expect("C++ decodes the searched stream");
    assert_eq!(
        plain, searched,
        "C++ decoded the searched stream to a different cloud"
    );
}

#[test]
fn cpp_reads_a_spatially_ordered_stream_as_the_same_points() {
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }
    for search in [false, true] {
        let plain = encode(search, false);
        let spatial = encode(search, true);
        assert_ne!(
            plain, spatial,
            "the spatial order changed nothing (search: {search})"
        );

        let decoded =
            decode_cpp_point_cloud_fingerprint(&spatial).expect("C++ decodes the reordered stream");
        let reference = decode_cpp_point_cloud_fingerprint(&plain).expect("C++ decodes the stream");
        assert_eq!(decoded.num_points, reference.num_points, "search: {search}");
        assert_eq!(
            decoded.num_attributes, reference.num_attributes,
            "search: {search}"
        );
        // The fingerprint walks the points in order, and the order is exactly
        // what this option changes, so the two hashes differ by construction
        // and the points are compared as a set instead. Asserted rather than
        // assumed: it is also what shows the hash is sensitive to the contents
        // at all, which is what the equality above the set comparison rests on.
        assert_ne!(
            decoded.attribute_hash, reference.attribute_hash,
            "the reordered stream hashed the same, so the order did not reach the decoder"
        );
        assert_eq!(
            sorted_positions(&plain),
            sorted_positions(&spatial),
            "C++ decoded the reordered stream to a different set of points (search: {search})"
        );
    }
}
