//! Emitting a point cloud's points in a spatial order instead of the caller's.
//!
//! A point cloud's point order carries no meaning, so an encoder may choose it,
//! and choosing it spatially is what lets the difference predictor predict from
//! a neighbour. `EncoderOptions::set_spatial_point_order` asks for that.
//!
//! The property that has to hold, and the only one that is not about size: a
//! point stays a point. Every attribute is permuted by the same order or the
//! file is silently corrupt, and a size test would never notice. So the cloud
//! here carries an exact integer tag alongside its position, and the test
//! follows each tag to the position it arrived with.

#![cfg(all(feature = "encoder", feature = "decoder"))]

use draco_core::{
    DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointAttribute,
    PointCloud, PointCloudDecoder, PointCloudEncoder,
};

const NUM_POINTS: usize = 4096;
/// Enough that the quantization step is far smaller than the distance between
/// any two of these points, so a tag can be matched to its position exactly.
const POSITION_BITS: i32 = 16;

fn lcg(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    (*state >> 8) as f32 / (1 << 24) as f32
}

/// Positions scattered through a cube, in an order that is not spatial.
fn positions() -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        NUM_POINTS,
    );
    let buffer = attribute.buffer_mut();
    let mut state = 12345u32;
    for point in 0..NUM_POINTS {
        for component in 0..3 {
            let value = lcg(&mut state) * 10.0;
            buffer.write((point * 3 + component) * 4, &value.to_le_bytes());
        }
    }
    attribute
}

/// Each point's index, as an integer so it survives untouched.
fn tags() -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(
        GeometryAttributeType::Generic,
        1,
        DataType::Uint32,
        false,
        NUM_POINTS,
    );
    let buffer = attribute.buffer_mut();
    for point in 0..NUM_POINTS {
        buffer.write(point * 4, &(point as u32).to_le_bytes());
    }
    attribute
}

fn cloud() -> PointCloud {
    let mut cloud = PointCloud::new();
    cloud.set_num_points(NUM_POINTS);
    cloud.add_attribute(positions());
    cloud.add_attribute(tags());
    cloud
}

/// An attribute that varies smoothly through space rather than along the
/// caller's ordering -- a colour, a scale, a harmonic. What a spatial order is
/// for.
fn spatially_coherent() -> PointAttribute {
    let positions = positions();
    let mut attribute = PointAttribute::new();
    attribute.init(
        GeometryAttributeType::Generic,
        3,
        DataType::Float32,
        false,
        NUM_POINTS,
    );
    let stride = positions.byte_stride() as usize;
    let read = |point: usize, component: usize| -> f32 {
        let mut bytes = [0u8; 4];
        positions
            .buffer()
            .read(point * stride + component * 4, &mut bytes);
        f32::from_le_bytes(bytes)
    };
    let buffer = attribute.buffer_mut();
    for point in 0..NUM_POINTS {
        let (x, y, z) = (read(point, 0), read(point, 1), read(point, 2));
        for (component, value) in [x + y, y - z, x * 0.5 + z].into_iter().enumerate() {
            buffer.write((point * 3 + component) * 4, &value.to_le_bytes());
        }
    }
    attribute
}

/// Positions plus one attribute that follows them.
fn coherent_cloud() -> PointCloud {
    let mut cloud = PointCloud::new();
    cloud.set_num_points(NUM_POINTS);
    cloud.add_attribute(positions());
    cloud.add_attribute(spatially_coherent());
    cloud
}

fn encode(cloud: &PointCloud, configure: impl Fn(&mut EncoderOptions)) -> Vec<u8> {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(0); // sequential
    options.set_attribute_int(0, "quantization_bits", POSITION_BITS);
    configure(&mut options);
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encode");
    buffer.data().to_vec()
}

fn decode(bytes: &[u8]) -> PointCloud {
    let mut decoded = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut DecoderBuffer::new(bytes), &mut decoded)
        .expect("decode");
    decoded
}

fn position_of(cloud: &PointCloud, point: usize) -> [f32; 3] {
    let attribute = cloud.attribute(0);
    let stride = attribute.byte_stride() as usize;
    let mut out = [0.0f32; 3];
    for (component, slot) in out.iter_mut().enumerate() {
        let mut bytes = [0u8; 4];
        attribute
            .buffer()
            .read(point * stride + component * 4, &mut bytes);
        *slot = f32::from_le_bytes(bytes);
    }
    out
}

fn tag_of(cloud: &PointCloud, point: usize) -> u32 {
    let attribute = cloud.attribute(1);
    let mut bytes = [0u8; 4];
    attribute
        .buffer()
        .read(point * attribute.byte_stride() as usize, &mut bytes);
    u32::from_le_bytes(bytes)
}

#[test]
fn the_order_is_the_callers_unless_it_is_asked_for() {
    let mut options = EncoderOptions::new();
    assert!(!options.spatial_point_order());
    options.set_spatial_point_order(true);
    assert!(options.spatial_point_order());

    let cloud = cloud();
    let untouched = encode(&cloud, |_| {});
    let explicitly_off = encode(&cloud, |options| options.set_spatial_point_order(false));
    assert_eq!(untouched, explicitly_off);
}

#[test]
fn a_spatial_order_is_smaller_when_the_values_follow_the_geometry() {
    let cloud = coherent_cloud();
    let as_given = encode(&cloud, |options| {
        options.set_attribute_int(1, "quantization_bits", 12)
    });
    let sorted = encode(&cloud, |options| {
        options.set_attribute_int(1, "quantization_bits", 12);
        options.set_spatial_point_order(true);
    });
    println!(
        "coherent: {} bytes as given, {} sorted, {:.1}% off",
        as_given.len(),
        sorted.len(),
        (1.0 - sorted.len() as f64 / as_given.len() as f64) * 100.0
    );
    assert!(
        sorted.len() < as_given.len(),
        "sorting did not help data that follows the geometry: {} vs {}",
        sorted.len(),
        as_given.len()
    );
}

/// The trade, stated as a measurement rather than as a warning in prose.
///
/// Reordering helps an attribute that varies through space and hurts one that
/// varies along the order it came in. The tag here is the extreme of the
/// second: consecutive integers, which difference-predict to a constant and
/// cost almost nothing until they are scrambled. Real data rarely looks like
/// this, but "rarely" is why it belongs in a test -- a caller whose attribute
/// does track its input ordering gets a bigger file, and that is a property of
/// the option and not a defect in it.
#[test]
fn a_spatial_order_costs_more_when_an_attribute_tracks_the_input_order() {
    let cloud = cloud();
    let as_given = encode(&cloud, |_| {});
    let sorted = encode(&cloud, |options| options.set_spatial_point_order(true));
    println!(
        "order-tracking: {} bytes as given, {} sorted, {:+.1}%",
        as_given.len(),
        sorted.len(),
        (sorted.len() as f64 / as_given.len() as f64 - 1.0) * 100.0
    );
    assert!(
        sorted.len() > as_given.len(),
        "the known regression stopped happening, which is good news that this          test cannot express: {} vs {}",
        sorted.len(),
        as_given.len()
    );
}

#[test]
fn every_point_keeps_the_values_it_arrived_with() {
    let source = cloud();
    let sorted = decode(&encode(&source, |options| {
        options.set_spatial_point_order(true)
    }));
    assert_eq!(sorted.num_points(), NUM_POINTS);

    // The tags must be a permutation: every one present, exactly once.
    let mut seen = vec![false; NUM_POINTS];
    for point in 0..NUM_POINTS {
        let tag = tag_of(&sorted, point) as usize;
        assert!(tag < NUM_POINTS, "tag {tag} is not one of ours");
        assert!(!seen[tag], "tag {tag} arrived twice");
        seen[tag] = true;
    }

    // And each one must have travelled with its own position. Quantization
    // moves a coordinate by under a step; anything else means two attributes
    // were permuted differently, which is the corruption this exists to catch.
    let step = 10.0 / ((1u32 << POSITION_BITS) - 1) as f32;
    let tolerance = step * 2.0;
    let mut worst = 0.0f32;
    for point in 0..NUM_POINTS {
        let tag = tag_of(&sorted, point) as usize;
        let decoded = position_of(&sorted, point);
        let original = position_of(&source, tag);
        for axis in 0..3 {
            worst = worst.max((decoded[axis] - original[axis]).abs());
        }
    }
    println!("worst coordinate drift {worst:.6}, tolerance {tolerance:.6}");
    assert!(
        worst < tolerance,
        "a point did not keep its position: drift {worst} against a {step} step"
    );
}

#[test]
fn the_reordered_stream_holds_the_same_geometry() {
    let source = cloud();
    let plain = decode(&encode(&source, |_| {}));
    let sorted = decode(&encode(&source, |options| {
        options.set_spatial_point_order(true)
    }));

    // Same points, in a different order: compare the two as multisets of
    // quantized coordinates rather than point by point.
    let bag = |cloud: &PointCloud| {
        let mut values: Vec<[u32; 3]> = (0..NUM_POINTS)
            .map(|point| position_of(cloud, point).map(f32::to_bits))
            .collect();
        values.sort_unstable();
        values
    };
    assert_eq!(
        bag(&plain),
        bag(&sorted),
        "the reorder changed the geometry"
    );
}

#[test]
fn a_cloud_with_no_positions_is_left_alone() {
    // Nothing to derive an order from, so the caller's order stands rather
    // than some arbitrary one.
    let mut cloud = PointCloud::new();
    cloud.set_num_points(NUM_POINTS);
    cloud.add_attribute(tags());

    let as_given = encode(&cloud, |_| {});
    let asked = encode(&cloud, |options| options.set_spatial_point_order(true));
    assert_eq!(
        as_given, asked,
        "a cloud with no position attribute was reordered anyway"
    );
}

#[test]
fn the_kd_tree_coder_keeps_its_own_order() {
    // It chooses a point order itself, and this option is the sequential
    // coder's. Asking for one must not change what it writes.
    let cloud = cloud();
    let with_kd_tree = |options: &mut EncoderOptions| {
        options.set_encoding_method(1);
        options.set_attribute_int(1, "quantization_bits", 16);
    };
    let as_given = encode(&cloud, |options| with_kd_tree(options));
    let asked = encode(&cloud, |options| {
        with_kd_tree(options);
        options.set_spatial_point_order(true);
    });
    assert_eq!(as_given, asked, "the kd-tree stream changed");
}
