//! Choosing a point-cloud attribute's prediction scheme by estimating both ways.
//!
//! The automatic choice for a point-cloud attribute is always `Difference`,
//! which costs more than it saves whenever consecutive values do not correlate.
//! `EncoderOptions::set_prediction_search` lets the encoder find that out from
//! the values, rather than from a rule that cannot know.
//!
//! What these pin, in the order that matters: that the option changes nothing
//! when it is off, that it changes nothing when the search finds nothing, that
//! it does shrink the case it exists for, that what it produces still decodes
//! to the same values, and that it leaves an explicit choice alone.

#![cfg(all(feature = "encoder", feature = "decoder"))]

use draco_core::{
    DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType, PointAttribute,
    PointCloud, PointCloudDecoder, PointCloudEncoder,
};

const NUM_POINTS: usize = 2048;

/// How an attribute's values move from one point to the next.
#[derive(Clone, Copy)]
enum Shape {
    /// Consecutive values differ by a little. Differencing wins by a lot.
    Smooth,
    /// Values concentrated around a centre, with outliers setting the
    /// quantization range -- the shape of a spherical-harmonic coefficient,
    /// and the one differencing makes worse. Two such values differ by more
    /// than either strays from the centre, so the differences occupy more
    /// levels than the values do.
    ///
    /// Uniform noise does *not* work here, which is worth recording: it
    /// already fills its range, so differencing cannot widen it and the search
    /// correctly finds nothing. The first version of this test used it and
    /// measured no difference at all.
    Concentrated,
}

fn attribute(
    kind: GeometryAttributeType,
    components: u8,
    seed: u32,
    shape: Shape,
) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(kind, components, DataType::Float32, false, NUM_POINTS);
    let buffer = attribute.buffer_mut();
    let mut state = seed.wrapping_mul(2654435761).wrapping_add(1);
    for point in 0..NUM_POINTS {
        for component in 0..components as usize {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let mut unit = || {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 8) as f32 / (1 << 24) as f32
            };
            let value = match shape {
                Shape::Smooth => point as f32 * 0.01 + component as f32 * 0.25,
                // A bell from four uniforms, plus rare outliers that stretch
                // the range without filling it.
                Shape::Concentrated => {
                    let bell: f32 = (0..4).map(|_| unit()).sum::<f32>() - 2.0;
                    if unit() < 0.002 {
                        bell * 8.0
                    } else {
                        bell
                    }
                }
            };
            let offset = (point * components as usize + component) * 4;
            buffer.write(offset, &value.to_le_bytes());
        }
    }
    attribute
}

/// A position plus one generic attribute of the given shape.
fn cloud(shape: Shape) -> PointCloud {
    let mut cloud = PointCloud::new();
    cloud.set_num_points(NUM_POINTS);
    cloud.add_attribute(attribute(
        GeometryAttributeType::Position,
        3,
        1,
        Shape::Smooth,
    ));
    cloud.add_attribute(attribute(GeometryAttributeType::Generic, 1, 7, shape));
    cloud
}

fn encode(cloud: &PointCloud, configure: impl Fn(&mut EncoderOptions)) -> Vec<u8> {
    let mut options = EncoderOptions::new();
    options.set_encoding_method(0); // sequential
    for id in 0..cloud.num_attributes() {
        options.set_attribute_int(id, "quantization_bits", 8);
    }
    configure(&mut options);
    let mut encoder = PointCloudEncoder::new();
    encoder.set_point_cloud(cloud.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).expect("encode");
    buffer.data().to_vec()
}

fn values(cloud: &PointCloud, att_id: i32) -> Vec<u8> {
    cloud.attribute(att_id).buffer().data().to_vec()
}

#[test]
fn the_search_is_off_unless_it_is_asked_for() {
    let mut options = EncoderOptions::new();
    assert!(
        !options.prediction_search(),
        "a fresh EncoderOptions searches"
    );
    options.set_prediction_search(true);
    assert!(options.prediction_search());
    options.set_prediction_search(false);
    assert!(!options.prediction_search());
}

#[test]
fn leaving_the_search_off_changes_nothing() {
    let cloud = cloud(Shape::Concentrated);
    let untouched = encode(&cloud, |_| {});
    let explicitly_off = encode(&cloud, |options| options.set_prediction_search(false));
    assert_eq!(
        untouched, explicitly_off,
        "the option changes the stream while switched off"
    );
}

#[test]
fn a_search_that_finds_nothing_leaves_the_stream_alone() {
    // The case this guards is not the choice but the plumbing: a searched
    // attribute is encoded into its own buffer and appended, and an ordinary
    // one is written straight to the output. Those two paths have to produce
    // the same bytes, and only a cloud where the default wins can show it.
    let cloud = cloud(Shape::Smooth);
    let unsearched = encode(&cloud, |_| {});
    let searched = encode(&cloud, |options| options.set_prediction_search(true));
    assert_eq!(
        unsearched, searched,
        "searching rewrote a stream it did not improve"
    );
}

#[test]
fn the_search_shrinks_what_prediction_was_making_worse() {
    let cloud = cloud(Shape::Concentrated);
    let unsearched = encode(&cloud, |options| options.set_prediction_search(false));
    let searched = encode(&cloud, |options| options.set_prediction_search(true));
    println!(
        "concentrated attribute: {} bytes searched, {} unsearched, {:.1}% off",
        searched.len(),
        unsearched.len(),
        (1.0 - searched.len() as f64 / unsearched.len() as f64) * 100.0
    );
    assert!(
        searched.len() < unsearched.len(),
        "searching found nothing on data differencing hurts: {} vs {}",
        searched.len(),
        unsearched.len()
    );

    // Smaller is necessary but not sufficient: it says the output changed,
    // not that it changed into the candidate. Encoding the attribute with
    // PREDICTION_NONE named outright must land on the same bytes.
    let forced = encode(&cloud, |options| {
        options.set_attribute_int(1, "prediction_scheme", -2)
    });
    assert_eq!(
        searched, forced,
        "the search produced something other than the candidate it chose"
    );
}

#[test]
fn what_the_search_produces_still_decodes_to_the_same_values() {
    let source = cloud(Shape::Concentrated);
    let unsearched = encode(&source, |options| options.set_prediction_search(false));
    let searched = encode(&source, |options| options.set_prediction_search(true));
    assert_ne!(
        unsearched, searched,
        "nothing was searched; test is vacuous"
    );

    let decode = |bytes: &[u8]| {
        let mut decoded = PointCloud::new();
        PointCloudDecoder::new()
            .decode(&mut DecoderBuffer::new(bytes), &mut decoded)
            .expect("decode");
        decoded
    };
    let plain = decode(&unsearched);
    let found = decode(&searched);

    assert_eq!(found.num_points(), source.num_points());
    assert_eq!(found.num_attributes(), source.num_attributes());
    for id in 0..source.num_attributes() {
        assert_eq!(
            values(&plain, id),
            values(&found, id),
            "attribute {id} decodes differently after the search"
        );
    }
}

#[test]
fn an_attribute_told_which_scheme_to_use_is_not_searched() {
    let cloud = cloud(Shape::Concentrated);
    // Difference, named explicitly on the attribute the search would have
    // changed. A caller who names a scheme has already made this decision.
    let forced =
        |options: &mut EncoderOptions| options.set_attribute_int(1, "prediction_scheme", 0);
    let unsearched = encode(&cloud, |options| forced(options));
    let searched = encode(&cloud, |options| {
        forced(options);
        options.set_prediction_search(true);
    });
    assert_eq!(
        unsearched, searched,
        "the search overrode a scheme the caller named"
    );
}
