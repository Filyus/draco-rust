//! Scalar quantization helpers.
//!
//! [`Quantizer`] and its inverse map floating-point values to and from a
//! fixed-point integer grid defined by an origin and range — the primitive used
//! by the quantization attribute transform. Port of Draco's
//! `quantization_utils.h`.

#[derive(Debug, Default, Clone, Copy)]
pub struct Quantizer {
    inverse_delta: f32,
}

impl Quantizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self, range: f32, max_quantized_value: i32) {
        if range > 0.0 {
            self.inverse_delta = max_quantized_value as f32 / range;
        } else {
            self.inverse_delta = 0.0;
        }
    }

    pub fn init_with_delta(&mut self, delta: f32) {
        if delta > 0.0 {
            self.inverse_delta = 1.0 / delta;
        } else {
            self.inverse_delta = 0.0;
        }
    }

    pub fn quantize_float(&self, val: f32) -> i32 {
        let val = val * self.inverse_delta;
        // Use explicit f32 literal to avoid accidental promotion to f64 and
        // to match C++'s float-floor(val + 0.5f) behavior exactly.
        let scaled = val + 0.5f32;

        // `f32::floor` is a call into libm on a baseline x86-64 target -- the
        // instruction that would inline it is SSE4.1, which the default target
        // does not assume -- and this runs once per component of every
        // quantized value. The cast truncates toward zero, so it already is
        // the floor for anything non-negative, and one comparison covers the
        // rest; `saturating_sub` keeps the correction from wrapping at the
        // bottom of the range, where the cast itself saturates.
        //
        // Exact for every finite input: above `2^24` an `f32` is already an
        // integer, so truncation and floor agree there and the round trip
        // through `as f32` is exact. `quantize_float_matches_floor` pins it
        // against `f32::floor` across the range.
        let truncated = scaled as i32;
        if (truncated as f32) > scaled {
            truncated.saturating_sub(1)
        } else {
            truncated
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Dequantizer {
    delta: f32,
}

impl Dequantizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self, range: f32, max_quantized_value: i32) -> bool {
        if max_quantized_value > 0 {
            self.delta = range / max_quantized_value as f32;
            true
        } else {
            false
        }
    }

    pub fn init_with_delta(&mut self, delta: f32) -> bool {
        if delta >= 0.0 {
            self.delta = delta;
            true
        } else {
            false
        }
    }

    #[inline(always)]
    pub fn dequantize_float(&self, val: i32) -> f32 {
        val as f32 * self.delta
    }
}

#[cfg(test)]
mod tests {
    use super::Quantizer;

    /// The floor inside `quantize_float` is open-coded to keep libm out of a
    /// per-component path. This pins it against `f32::floor` over the range
    /// the transform actually produces and past the point where an `f32` is
    /// integral, including the negatives a caller can reach through the
    /// public API and the saturating ends.
    #[test]
    fn quantize_float_matches_floor() {
        let mut quantizer = Quantizer::new();
        quantizer.init_with_delta(1.0);

        let mut cases: Vec<f32> = vec![
            0.0,
            -0.0,
            0.49999997,
            0.5,
            -0.5,
            -0.50000006,
            1.0,
            -1.0,
            16_777_215.0,
            16_777_216.0,
            -16_777_216.0,
            f32::MAX,
            f32::MIN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ];
        let mut x = -4.0f32;
        while x < 4.0 {
            cases.push(x);
            x += 0.0625;
        }

        for value in cases {
            let expected = (value + 0.5f32).floor();
            let expected = if expected >= i32::MAX as f32 {
                i32::MAX
            } else if expected <= i32::MIN as f32 {
                i32::MIN
            } else {
                expected as i32
            };
            assert_eq!(
                quantizer.quantize_float(value),
                expected,
                "quantizing {value}"
            );
        }
    }
}
