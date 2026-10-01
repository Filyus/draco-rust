//! The point order option, through the public encoder and decoder: the stream
//! it writes must read back as the same points, must be smaller than the one
//! without it where the input order is bad, must be the very stream without it
//! where the input order is already good, and must not depend on how many
//! threads wrote it.

#![cfg(all(feature = "encoder", feature = "decoder"))]

use draco_core::{
    DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointAttribute,
    PointCloud, PointCloudDecoder, PointCloudEncoder, PointIndex,
};

const SEQUENTIAL: i32 = 0;

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// A position and `extras` single-float attributes, per point.
struct Points {
    positions: Vec<[f32; 3]>,
    extras: Vec<Vec<f32>>,
}

/// Points on a helix, their attributes smooth along it: an input order as
/// coherent as a scan's.
fn helix(points: usize) -> Points {
    let positions = (0..points)
        .map(|i| {
            let t = i as f32 / points as f32 * 60.0;
            [t.cos() * 10.0, t.sin() * 10.0, t * 0.2]
        })
        .collect();
    let extras = (0..8)
        .map(|k| {
            (0..points)
                .map(|i| ((i as f32 / points as f32) * (3.0 + k as f32)).sin())
                .collect()
        })
        .collect();
    Points { positions, extras }
}

/// The same points in an order that means nothing.
fn shuffled(points: &Points, seed: u64) -> Points {
    let mut rng = Xorshift(seed);
    let mut order: Vec<usize> = (0..points.positions.len()).collect();
    for i in (1..order.len()).rev() {
        order.swap(i, rng.next() as usize % (i + 1));
    }
    Points {
        positions: order.iter().map(|&i| points.positions[i]).collect(),
        extras: points
            .extras
            .iter()
            .map(|e| order.iter().map(|&i| e[i]).collect())
            .collect(),
    }
}

fn cloud(points: &Points) -> PointCloud {
    let count = points.positions.len();
    let mut cloud = PointCloud::new();
    cloud.set_num_points(count);
    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        count,
    );
    for (p, xyz) in points.positions.iter().enumerate() {
        for (axis, value) in xyz.iter().enumerate() {
            position
                .buffer_mut()
                .write((p * 3 + axis) * 4, &value.to_le_bytes());
        }
    }
    cloud.add_attribute(position);
    for values in &points.extras {
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Generic,
            1,
            DataType::Float32,
            false,
            count,
        );
        for (p, value) in values.iter().enumerate() {
            attribute.buffer_mut().write(p * 4, &value.to_le_bytes());
        }
        cloud.add_attribute(attribute);
    }
    cloud
}

fn options(extras: usize) -> EncoderOptions {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(SEQUENTIAL);
    options.set_attribute_int(0, "quantization_bits", 14);
    for id in 1..=extras as i32 {
        options.set_attribute_int(id, "quantization_bits", 8);
    }
    options
}

fn encode(cloud: &PointCloud, options: &EncoderOptions) -> Vec<u8> {
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(options, &mut buffer).expect("encodes");
    buffer.data().to_vec()
}

/// One hash a point over every decoded attribute, sorted: the points as a set,
/// the order taken out.
fn decoded_set(bytes: &[u8]) -> Vec<u64> {
    let mut decoded = PointCloud::new();
    PointCloudDecoder::new()
        .decode(&mut DecoderBuffer::new(bytes), &mut decoded)
        .expect("what the encoder wrote, the decoder reads");
    let mut hashes = vec![0xcbf2_9ce4_8422_2325u64; decoded.num_points()];
    for id in 0..decoded.num_attributes() {
        let attribute = decoded.attribute(id);
        let stride = attribute.byte_stride() as usize;
        for (point, hash) in hashes.iter_mut().enumerate() {
            let index = attribute.mapped_index(PointIndex(point as u32)).0 as usize;
            for component in 0..attribute.num_components() as usize {
                let mut bytes = [0u8; 4];
                attribute
                    .buffer()
                    .read(index * stride + component * 4, &mut bytes);
                *hash =
                    (*hash ^ u64::from(u32::from_le_bytes(bytes))).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    hashes.sort_unstable();
    hashes
}

#[test]
fn a_searched_order_decodes_to_the_same_points_in_a_smaller_stream() {
    let points = shuffled(&helix(60_000), 1);
    let cloud = cloud(&points);
    let plain = options(8);
    let mut spatial = options(8);
    spatial.set_spatial_point_order(true);
    let mut searched = options(8);
    searched.set_point_order_search(true);

    let plain_bytes = encode(&cloud, &plain);
    let spatial_bytes = encode(&cloud, &spatial);
    let searched_bytes = encode(&cloud, &searched);

    let expected = decoded_set(&plain_bytes);
    assert_eq!(
        decoded_set(&spatial_bytes),
        expected,
        "the curve changed the points"
    );
    assert_eq!(
        decoded_set(&searched_bytes),
        expected,
        "the search changed the points"
    );

    assert!(
        spatial_bytes.len() < plain_bytes.len(),
        "the curve: {} against {}",
        spatial_bytes.len(),
        plain_bytes.len()
    );
    assert!(
        searched_bytes.len() < spatial_bytes.len(),
        "the search: {} against the curve's {}",
        searched_bytes.len(),
        spatial_bytes.len()
    );
}

#[test]
fn an_input_order_that_is_already_good_is_written_as_it_was() {
    let cloud = cloud(&helix(60_000));
    let plain = options(8);
    let mut searched = options(8);
    searched.set_point_order_search(true);
    assert_eq!(
        encode(&cloud, &searched),
        encode(&cloud, &plain),
        "the search declined, so the stream must be the one without it"
    );
}

#[test]
fn the_stream_does_not_depend_on_the_number_of_threads() {
    let points = shuffled(&helix(80_000), 2);
    let cloud = cloud(&points);
    let mut searched = options(8);
    searched.set_point_order_search(true);
    searched.set_threads(1);
    let single = encode(&cloud, &searched);
    for threads in [2, 7, 16, 0] {
        searched.set_threads(threads);
        assert_eq!(encode(&cloud, &searched), single, "{threads} threads");
    }
}

#[test]
fn every_encoding_speed_writes_a_stream_that_reads_back_as_the_same_points() {
    let points = shuffled(&helix(30_000), 3);
    let cloud = cloud(&points);
    let expected = decoded_set(&encode(&cloud, &options(8)));
    let mut searched = options(8);
    searched.set_point_order_search(true);
    for speed in 0..=10 {
        searched.set_global_int("encoding_speed", speed);
        assert_eq!(
            decoded_set(&encode(&cloud, &searched)),
            expected,
            "speed {speed}"
        );
    }
}
