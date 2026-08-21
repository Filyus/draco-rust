//! The monotonicity the encoder's position-bounds reporting rests on.
//!
//! `MeshEncoder::build_encoded_mesh_info` reports the bounds of the position
//! attribute *as a decoder will see it*, i.e. after a quantize/dequantize round
//! trip. It gets them by folding the original attribute for its min and max and
//! round-tripping those two scalars, rather than round-tripping every point and
//! folding the result -- which is only the same answer because the round trip
//! is monotonic non-decreasing per component.
//!
//! That is an algebraic claim about the arithmetic in
//! `AttributeQuantizationTransform`, not about the encoder, so it is tested
//! here directly: if a future change to quantization breaks the ordering, this
//! fails and names the reason, instead of the bounds quietly drifting.

use draco_core::attribute_quantization_transform::AttributeQuantizationTransform;

/// Deterministic spread of values across and beyond the quantization range,
/// including the exact endpoints and values that land between two steps.
fn sample_values(min: f32, range: f32) -> Vec<f32> {
    let mut values = vec![min, min + range];
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    for _ in 0..512 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
        values.push(min + range * unit as f32);
    }
    values
}

#[test]
fn round_trip_is_monotonic_so_extremes_map_to_extremes() {
    for &bits in &[1, 2, 8, 11, 16, 24, 30] {
        for &(min, range) in &[
            (0.0f32, 1.0f32),
            (-3.5, 7.0),
            (1e-4, 1e-3),
            (-1234.5, 4321.0),
        ] {
            let mut transform = AttributeQuantizationTransform::new();
            transform
                .set_parameters(bits, &[min, min, min], range)
                .expect("parameters");

            let values = sample_values(min, range);
            let round_tripped: Vec<f32> = values
                .iter()
                .map(|&v| {
                    transform
                        .round_trip_component(0, v)
                        .expect("round trip component")
                })
                .collect();

            // Monotonic: sorting the inputs sorts the outputs.
            let mut by_input: Vec<(f32, f32)> = values
                .iter()
                .copied()
                .zip(round_tripped.iter().copied())
                .collect();
            by_input.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("no NaN inputs"));
            for pair in by_input.windows(2) {
                assert!(
                    pair[0].1 <= pair[1].1,
                    "round trip not monotonic at bits={bits} min={min} range={range}: \
                     {} -> {} but {} -> {}",
                    pair[0].0,
                    pair[0].1,
                    pair[1].0,
                    pair[1].1
                );
            }

            // The consequence the encoder relies on, asserted bit-exactly:
            // round-tripping the extremes gives the extremes of the round trip.
            let fold_then_trip = (
                transform
                    .round_trip_component(0, by_input[0].0)
                    .expect("min"),
                transform
                    .round_trip_component(0, by_input[by_input.len() - 1].0)
                    .expect("max"),
            );
            let trip_then_fold = (
                round_tripped.iter().copied().fold(f32::INFINITY, f32::min),
                round_tripped
                    .iter()
                    .copied()
                    .fold(f32::NEG_INFINITY, f32::max),
            );
            assert_eq!(
                fold_then_trip.0.to_bits(),
                trip_then_fold.0.to_bits(),
                "min disagrees at bits={bits} min={min} range={range}"
            );
            assert_eq!(
                fold_then_trip.1.to_bits(),
                trip_then_fold.1.to_bits(),
                "max disagrees at bits={bits} min={min} range={range}"
            );
        }
    }
}
