//! An encode on threads writes the stream an encode on one does, byte for
//! byte: attributes encoded side by side and appended in order, and a large
//! attribute's own passes -- quantization, the gather, the prediction, the
//! symbol plan -- run in pieces whose results are combined in order.

#![cfg(feature = "encoder")]

use draco_core::{
    DataType, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointAttribute, PointCloud,
    PointCloudEncoder,
};

const SEQUENTIAL: i32 = 0;

/// Enough points that the position's three components pass the size at which
/// one attribute's passes are cut into pieces (`parallel::PASS_MIN_VALUES`,
/// 2^20 values), so the inner passes run on threads as well as the attributes.
const POINTS: usize = 360_000;

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn float_attribute(kind: GeometryAttributeType, components: u8, values: &[f32]) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(kind, components, DataType::Float32, false, POINTS);
    for (i, value) in values.iter().enumerate() {
        attribute.buffer_mut().write(i * 4, &value.to_le_bytes());
    }
    attribute
}

/// A position, a normal, a generic float and an 8-bit colour, so every
/// sequential attribute encoder takes part.
fn cloud() -> PointCloud {
    let mut rng = Xorshift(0x9e37_79b9_7f4a_7c15);
    let positions: Vec<f32> = (0..POINTS * 3)
        .map(|i| (i / 3) as f32 * 0.001 + rng.unit())
        .collect();
    let normals: Vec<f32> = (0..POINTS)
        .flat_map(|_| {
            let (x, y) = (rng.unit() - 0.5, rng.unit() - 0.5);
            let z = (1.0 - x * x - y * y).max(0.0).sqrt();
            [x, y, z]
        })
        .collect();
    let generic: Vec<f32> = (0..POINTS).map(|_| rng.unit() * 4.0 - 2.0).collect();

    let mut cloud = PointCloud::new();
    cloud.set_num_points(POINTS);
    cloud.add_attribute(float_attribute(
        GeometryAttributeType::Position,
        3,
        &positions,
    ));
    cloud.add_attribute(float_attribute(GeometryAttributeType::Normal, 3, &normals));
    cloud.add_attribute(float_attribute(GeometryAttributeType::Generic, 1, &generic));
    let mut colour = PointAttribute::new();
    colour.init(
        GeometryAttributeType::Color,
        3,
        DataType::Uint8,
        true,
        POINTS,
    );
    for i in 0..POINTS * 3 {
        colour.buffer_mut().write(i, &[(rng.next() >> 56) as u8]);
    }
    cloud.add_attribute(colour);
    cloud
}

fn encode(cloud: &PointCloud, options: &EncoderOptions) -> Vec<u8> {
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(options, &mut buffer).expect("encodes");
    buffer.data().to_vec()
}

#[test]
fn the_stream_does_not_depend_on_the_number_of_threads() {
    let cloud = cloud();
    for (prediction_search, spatial) in [(false, false), (true, true)] {
        let mut options = EncoderOptions::new();
        options.set_encoding_method(SEQUENTIAL);
        options.set_attribute_int(0, "quantization_bits", 16);
        options.set_attribute_int(1, "quantization_bits", 10);
        options.set_attribute_int(2, "quantization_bits", 12);
        options.set_prediction_search(prediction_search);
        options.set_spatial_point_order(spatial);
        options.set_threads(1);
        let single = encode(&cloud, &options);
        for threads in [2, 7, 16, 0] {
            options.set_threads(threads);
            assert!(
                encode(&cloud, &options) == single,
                "{threads} threads (prediction search {prediction_search}, spatial {spatial})"
            );
        }
    }
}

/// The passes in pieces on the attribute shapes the cloud above leaves out:
/// one component, two of a 16-bit integer, four of a byte, and a normal,
/// whose two octahedral coordinates are what its passes cut. One point past
/// sixteen pieces, so the last piece holds one value. The float column holds
/// both zeros, each first met in a different piece, where a fold that took
/// whichever came first would write a different minimum.
#[test]
fn every_attribute_shape_cut_into_pieces_writes_the_same_stream() {
    const POINTS: usize = 16 * (1 << 16) + 1;
    let mut rng = Xorshift(0x2545_f491_4f6c_dd1d);
    let attribute = |kind, components: u8, data_type, bytes: Vec<u8>| {
        let mut attribute = PointAttribute::new();
        attribute.init(kind, components, data_type, false, POINTS);
        attribute.buffer_mut().write(0, &bytes);
        attribute
    };
    let floats =
        |values: Vec<f32>| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };

    let zeros_apart: Vec<f32> = (0..POINTS)
        .map(|i| match i {
            100 => 0.0,
            327_687 => -0.0,
            _ => 1.0 + (i % 977) as f32 * 0.01,
        })
        .collect();
    let normals: Vec<f32> = (0..POINTS * 3).map(|_| rng.unit() * 2.0 - 1.0).collect();
    let pairs: Vec<u8> = (0..POINTS * 2)
        .flat_map(|i| (((i / 2) as i16).wrapping_mul(31) ^ (rng.next() >> 60) as i16).to_le_bytes())
        .collect();
    let quads: Vec<u8> = (0..POINTS * 4).map(|_| (rng.next() >> 59) as u8).collect();

    let mut cloud = PointCloud::new();
    cloud.set_num_points(POINTS);
    cloud.add_attribute(attribute(
        GeometryAttributeType::Generic,
        1,
        DataType::Float32,
        floats(zeros_apart),
    ));
    cloud.add_attribute(attribute(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        floats(normals),
    ));
    cloud.add_attribute(attribute(
        GeometryAttributeType::Generic,
        2,
        DataType::Int16,
        pairs,
    ));
    cloud.add_attribute(attribute(
        GeometryAttributeType::Color,
        4,
        DataType::Uint8,
        quads,
    ));

    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_attribute_int(0, "quantization_bits", 14);
    options.set_attribute_int(1, "quantization_bits", 10);
    options.set_threads(1);
    let single = encode(&cloud, &options);
    options.set_threads(16);
    assert!(encode(&cloud, &options) == single);
}
