//! Octahedral normal compression helpers.
//!
//! [`OctahedronToolBox`] converts unit normals to and from the quantized 2D
//! octahedral representation and provides the wrap/clamp math that the normal
//! transforms and predictors rely on. Port of Draco's
//! `normal_compression_utils.h`.

#[derive(Debug, Default, Clone, Copy)]
pub struct OctahedronToolBox {
    quantization_bits: i32,
    max_quantized_value: i32,
    max_value: i32,
    dequantization_scale: f32,
    center_value: i32,
}

impl OctahedronToolBox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_quantization_bits(&mut self, q: i32) -> bool {
        if !(2..=30).contains(&q) {
            return false;
        }
        self.quantization_bits = q;
        self.max_quantized_value = (1 << q) - 1;
        self.max_value = self.max_quantized_value - 1;
        self.dequantization_scale = 2.0 / self.max_value as f32;
        self.center_value = self.max_value / 2;
        true
    }

    pub fn is_initialized(&self) -> bool {
        self.quantization_bits != -1
    }

    pub fn quantization_bits(&self) -> i32 {
        self.quantization_bits
    }

    pub fn max_quantized_value(&self) -> i32 {
        self.max_quantized_value
    }

    pub fn max_value(&self) -> i32 {
        self.max_value
    }

    pub fn center_value(&self) -> i32 {
        self.center_value
    }

    /// Whether `(s, t)`, with the centre already at the origin, is inside the
    /// diamond.
    ///
    /// Upstream asserts `|s|, |t| <= center_value` here; this does not, and the
    /// reason is that the decode path cannot honour it. A correction value is
    /// read from the stream and added to the prediction, and `mod_max` folds
    /// the sum back by one period -- enough for a correction the encoder wrote,
    /// not for one an attacker chose. The arithmetic below is written to wrap
    /// rather than to trust the range, so the assertion would only turn a
    /// malformed file into a panic in any build with debug assertions on.
    pub fn is_in_diamond(&self, s: i32, t: i32) -> bool {
        let st = s.unsigned_abs().wrapping_add(t.unsigned_abs());
        st <= self.center_value as u32
    }

    /// Reflects `(s, t)` across the diamond's edge.
    ///
    /// Carries no range assertion, for the reason given on [`Self::is_in_diamond`].
    pub fn invert_diamond(&self, s: &mut i32, t: &mut i32) {
        // C++ code determines signs without modifying the values
        let sign_s: i32;
        let sign_t: i32;

        if *s >= 0 && *t >= 0 {
            sign_s = 1;
            sign_t = 1;
        } else if *s <= 0 && *t <= 0 {
            sign_s = -1;
            sign_t = -1;
        } else {
            sign_s = if *s > 0 { 1 } else { -1 };
            sign_t = if *t > 0 { 1 } else { -1 };
        }

        // Perform the addition and subtraction using unsigned integers to avoid
        // signed integer overflows for bad data. Note that the result will be
        // unchanged for non-overflowing cases.
        let corner_point_s = (sign_s * self.center_value) as u32;
        let corner_point_t = (sign_t * self.center_value) as u32;

        let mut us = *s as u32;
        let mut ut = *t as u32;

        us = us.wrapping_add(us).wrapping_sub(corner_point_s);
        ut = ut.wrapping_add(ut).wrapping_sub(corner_point_t);

        if sign_s * sign_t >= 0 {
            let temp = us;
            us = (-(ut as i32)) as u32;
            ut = (-(temp as i32)) as u32;
        } else {
            std::mem::swap(&mut us, &mut ut);
        }

        us = us.wrapping_add(corner_point_s);
        ut = ut.wrapping_add(corner_point_t);

        *s = us as i32;
        *t = ut as i32;
        *s /= 2;
        *t /= 2;
    }

    pub fn invert_direction(&self, s: &mut i32, t: &mut i32) {
        *s *= -1;
        *t *= -1;
        self.invert_diamond(s, t);
    }

    pub fn mod_max(&self, x: i32) -> i32 {
        // Branchless on purpose, unlike `CornerTable::next`'s wrap: the two
        // conditions are mutually exclusive (they would need
        // `center < x < -center`), and `x` is a decoded octahedral coordinate,
        // so which way it falls is data and not a pattern the predictor learns.
        let over = (x > self.center_value) as i32;
        let under = (x < -self.center_value) as i32;
        x + (under - over) * self.max_quantized_value
    }

    pub fn mod_max_positive(&self, x: i32) -> i32 {
        x & self.max_quantized_value
    }

    pub fn make_positive(&self, x: i32) -> i32 {
        debug_assert!(x <= self.center_value * 2);
        if x < 0 {
            return x + self.max_quantized_value;
        }
        x
    }

    pub fn canonicalize_octahedral_coords(&self, s: i32, t: i32) -> (i32, i32) {
        let mut s = s;
        let mut t = t;
        // Check if coordinates are at corners that need canonicalization
        let is_corner =
            (s == 0 && (t == 0 || t == self.max_value)) || (s == self.max_value && t == 0);
        if is_corner {
            s = self.max_value;
            t = self.max_value;
        } else if s == 0 && t > self.center_value {
            t = self.center_value - (t - self.center_value);
        } else if s == self.max_value && t < self.center_value {
            t = self.center_value + (self.center_value - t);
        } else if t == self.max_value && s < self.center_value {
            s = self.center_value + (self.center_value - s);
        } else if t == 0 && s > self.center_value {
            s = self.center_value - (s - self.center_value);
        }
        (s, t)
    }

    /// Scales an integer vector onto the octahedron's `|x| + |y| + |z| =
    /// center_value` diamond.
    ///
    /// Every absolute value is taken with `wrapping_abs`, because `i32::abs`
    /// has no answer at `i32::MIN` and panics for it wherever overflow checks
    /// are on. That is a reachable input rather than a hypothetical one: the
    /// geometric-normal predictor computes this vector from positions and
    /// clamps the result into `i32`, so a prediction past the range arrives
    /// here as exactly `i32::MIN`.
    ///
    /// Wrapping and not `unsigned_abs`, which would be the more sensible
    /// magnitude: at `i32::MIN` the two differ in sign, and release builds --
    /// upstream's included, where `std::abs` is undefined on that value -- have
    /// always carried the wrapped one into the sum below. Keeping it leaves
    /// every encode byte-identical and removes only the panic.
    pub fn canonicalize_integer_vector(&self, vec: &mut [i32; 3]) {
        let magnitude = |v: i32| i64::from(v.wrapping_abs());
        let abs_sum = magnitude(vec[0]) + magnitude(vec[1]) + magnitude(vec[2]);

        if abs_sum == 0 {
            vec[0] = self.center_value;
            vec[1] = 0;
            vec[2] = 0;
        } else {
            vec[0] = ((vec[0] as i64 * self.center_value as i64) / abs_sum) as i32;
            vec[1] = ((vec[1] as i64 * self.center_value as i64) / abs_sum) as i32;
            // `vec[0]` and `vec[1]` are now the scaled values, whose magnitudes
            // sum to at most `center_value`, so these stay in range.
            let remainder = self.center_value - vec[0].wrapping_abs() - vec[1].wrapping_abs();
            vec[2] = if vec[2] >= 0 {
                remainder
            } else {
                remainder.wrapping_neg()
            };
        }
    }

    pub fn integer_vector_to_quantized_octahedral_coords(&self, int_vec: &[i32; 3]) -> (i32, i32) {
        let abs_sum = int_vec[0].abs() + int_vec[1].abs() + int_vec[2].abs();
        debug_assert_eq!(abs_sum, self.center_value);

        let s;
        let t;
        if int_vec[0] >= 0 {
            // Right hemisphere.
            s = int_vec[1] + self.center_value;
            t = int_vec[2] + self.center_value;
        } else {
            // Left hemisphere.
            if int_vec[1] < 0 {
                s = int_vec[2].abs();
            } else {
                s = self.max_value - int_vec[2].abs();
            }
            if int_vec[2] < 0 {
                t = int_vec[1].abs();
            } else {
                t = self.max_value - int_vec[1].abs();
            }
        }
        self.canonicalize_octahedral_coords(s, t)
    }

    pub fn float_vector_to_quantized_octahedral_coords(&self, vector: &[f32; 3]) -> (i32, i32) {
        // Double, as upstream does it, and not for extra accuracy: the value
        // being floored lands exactly on `.5` for ordinary normals -- (0, k, k)
        // among them -- and single precision resolves that tie the other way.
        // The two encoders then pick neighbouring octahedral coordinates for
        // the same input.
        let abs_sum =
            (vector[0] as f64).abs() + (vector[1] as f64).abs() + (vector[2] as f64).abs();

        // Adjust values such that abs sum equals 1.
        let mut scaled_vector = [0.0f64; 3];
        if abs_sum > 1e-6 {
            let scale = 1.0 / abs_sum;
            scaled_vector[0] = vector[0] as f64 * scale;
            scaled_vector[1] = vector[1] as f64 * scale;
            scaled_vector[2] = vector[2] as f64 * scale;
        } else {
            scaled_vector[0] = 1.0;
            scaled_vector[1] = 0.0;
            scaled_vector[2] = 0.0;
        }

        // Scale vector such that the sum equals the center value.
        let mut int_vec = [0; 3];
        int_vec[0] = (scaled_vector[0] * self.center_value as f64 + 0.5).floor() as i32;
        int_vec[1] = (scaled_vector[1] * self.center_value as f64 + 0.5).floor() as i32;

        // Make sure the sum is exactly the center value.
        int_vec[2] = self.center_value - int_vec[0].abs() - int_vec[1].abs();
        if int_vec[2] < 0 {
            // If the sum of first two coordinates is too large, we need to decrease
            // the length of one of the coordinates.
            if int_vec[1] > 0 {
                int_vec[1] += int_vec[2];
            } else {
                int_vec[1] -= int_vec[2];
            }
            int_vec[2] = 0;
        }
        // Take care of the sign.
        if scaled_vector[2] < 0.0 {
            int_vec[2] *= -1;
        }

        self.integer_vector_to_quantized_octahedral_coords(&int_vec)
    }

    pub fn quantized_octahedral_coords_to_unit_vector(&self, s: i32, t: i32) -> [f32; 3] {
        // Scale s and t to [-1, 1] range
        let in_s_scaled = s as f32 * self.dequantization_scale - 1.0;
        let in_t_scaled = t as f32 * self.dequantization_scale - 1.0;

        // In the octahedral encoding:
        //   s corresponds to y component
        //   t corresponds to z component
        //   x is computed from the octahedron constraint
        let mut y = in_s_scaled;
        let mut z = in_t_scaled;

        // Compute x from the octahedron surface constraint
        let x = 1.0 - y.abs() - z.abs();

        // For points on the left hemisphere (x < 0), we need to unwrap them
        // by mirroring along the diagonal edges of the diamond
        if x < 0.0 {
            let x_offset = -x;
            y += if y < 0.0 { x_offset } else { -x_offset };
            z += if z < 0.0 { x_offset } else { -x_offset };
        }

        // Normalize the vector
        let norm_squared = x * x + y * y + z * z;
        if norm_squared < 1e-6 {
            [0.0, 0.0, 0.0]
        } else {
            let d = 1.0 / norm_squared.sqrt();
            [x * d, y * d, z * d]
        }
    }

    pub fn quantized_octahedral_coords_to_unit_vector_legacy(&self, s: i32, t: i32) -> [f32; 3] {
        let max_quantized_value = self.max_value as f32;
        let in_s = s as f32 / max_quantized_value;
        let in_t = t as f32 / max_quantized_value;

        let mut out_s = in_s;
        let mut out_t = in_t;
        let mut spt = out_s + out_t;
        let mut smt = out_s - out_t;
        let mut x_sign = 1.0;

        if !(0.5..=1.5).contains(&spt) || !(-0.5..=0.5).contains(&smt) {
            x_sign = -1.0;
            if spt <= 0.5 {
                out_s = 0.5 - in_t;
                out_t = 0.5 - in_s;
            } else if spt >= 1.5 {
                out_s = 1.5 - in_t;
                out_t = 1.5 - in_s;
            } else if smt <= -0.5 {
                out_s = in_t - 0.5;
                out_t = in_s + 0.5;
            } else {
                out_s = in_t + 0.5;
                out_t = in_s - 0.5;
            }
            spt = out_s + out_t;
            smt = out_s - out_t;
        }

        let y = 2.0 * out_s - 1.0;
        let z = 2.0 * out_t - 1.0;
        let x = (2.0 * spt - 1.0)
            .min(3.0 - 2.0 * spt)
            .min(2.0 * smt + 1.0)
            .min(1.0 - 2.0 * smt)
            * x_sign;

        let norm_squared = x * x + y * y + z * z;
        if norm_squared < 1e-6 {
            [0.0, 0.0, 0.0]
        } else {
            let d = 1.0 / norm_squared.sqrt();
            [x * d, y * d, z * d]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `i32::MIN` reaches this from the geometric-normal predictor, which
    /// clamps its prediction into `i32`, and `i32::abs` has no answer for it.
    ///
    /// Found by the `encode_drc` corpus replayed under overflow checks: a mesh
    /// with a `uint64` position and a normal predicted from it panicked here
    /// with "attempt to negate with overflow". The read that produced the
    /// extreme value is gone -- predictors now see the portable `int32` copy --
    /// but nothing stops the next caller, so the arithmetic is what is pinned.
    #[test]
    fn canonicalizing_a_vector_at_the_integer_minimum_does_not_panic() {
        let mut tool_box = OctahedronToolBox::new();
        assert!(tool_box.set_quantization_bits(8));

        for vec in [
            [i32::MIN, 0, 0],
            [0, i32::MIN, 0],
            [0, 0, i32::MIN],
            [i32::MIN, i32::MIN, i32::MIN],
        ] {
            let mut vec = vec;
            tool_box.canonicalize_integer_vector(&mut vec);
        }
    }
}
