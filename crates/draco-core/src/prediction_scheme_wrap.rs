//! Wrap prediction-scheme transform.
//!
//! Maps prediction corrections into the attribute's value range with modular
//! wrap-around, so residuals stay small even when a prediction overshoots the
//! min/max. The standard residual transform for quantized integer attributes.
//! Port of Draco's `prediction_scheme_wrap_*_transform`.

use crate::prediction_scheme::PredictionSchemeTransformType;
use std::marker::PhantomData;

#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "decoder")]
use crate::prediction_scheme::PredictionSchemeDecodingTransform;

#[cfg(feature = "encoder")]
use crate::prediction_scheme::PredictionSchemeEncodingTransform;
#[cfg(feature = "decoder")]
use crate::status::DracoError;
use crate::status::Status;

#[cfg(feature = "encoder")]
pub struct PredictionSchemeWrapEncodingTransform<DataType> {
    num_components: usize,
    min_value: DataType,
    max_value: DataType,
    max_dif: DataType,
    min_correction: DataType,
    max_correction: DataType,
    _marker: PhantomData<DataType>,
}

#[cfg(feature = "encoder")]
impl<DataType> Default for PredictionSchemeWrapEncodingTransform<DataType>
where
    DataType: Copy + Ord + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "encoder")]
impl<DataType> PredictionSchemeWrapEncodingTransform<DataType>
where
    DataType: Copy + Ord + Default,
{
    pub fn new() -> Self {
        Self {
            num_components: 0,
            min_value: DataType::default(),
            max_value: DataType::default(),
            max_dif: DataType::default(),
            min_correction: DataType::default(),
            max_correction: DataType::default(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "encoder")]
impl PredictionSchemeEncodingTransform<i32, i32> for PredictionSchemeWrapEncodingTransform<i32> {
    fn get_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::Wrap
    }

    fn init(&mut self, orig_data: &[i32], size: usize, num_components: usize) {
        self.num_components = num_components;

        if size == 0 {
            return;
        }

        let mut min_val = orig_data[0];
        let mut max_val = orig_data[0];

        for i in 1..size {
            let val = orig_data[i];
            if val < min_val {
                min_val = val;
            }
            if val > max_val {
                max_val = val;
            }
        }

        self.min_value = min_val;
        self.max_value = max_val;

        // InitCorrectionBounds
        let dif = (max_val as i64) - (min_val as i64);

        self.max_dif = (1 + dif) as i32;
        self.max_correction = self.max_dif / 2;
        self.min_correction = -self.max_correction;
        if (self.max_dif & 1) == 0 {
            self.max_correction -= 1;
        }
    }

    fn compute_correction(
        &self,
        original_vals: &[i32],
        predicted_vals: &[i32],
        out_corr_vals: &mut [i32],
    ) {
        for i in 0..self.num_components {
            // Clamp predicted value
            let mut pred = predicted_vals[i];
            if pred > self.max_value {
                pred = self.max_value;
            } else if pred < self.min_value {
                pred = self.min_value;
            }

            let mut corr_val = original_vals[i].wrapping_sub(pred);

            // Wrap around
            if corr_val < self.min_correction {
                corr_val = corr_val.wrapping_add(self.max_dif);
            } else if corr_val > self.max_correction {
                corr_val = corr_val.wrapping_sub(self.max_dif);
            }
            out_corr_vals[i] = corr_val;
        }
    }

    /// The same clamp and wrap as `compute_correction`, which apply to every
    /// component alike, over the whole run as one loop of selects: nothing in
    /// it depends on an entry boundary, and it vectorizes where the per-entry
    /// form carried a slice and a bound to check for every value.
    fn compute_corrections(
        &self,
        original_vals: &[i32],
        predicted_vals: &[i32],
        out_corr_vals: &mut [i32],
        _num_components: usize,
    ) {
        let (min_value, max_value) = (self.min_value, self.max_value);
        let (min_correction, max_correction) = (self.min_correction, self.max_correction);
        let max_dif = self.max_dif;
        for ((corr, &original), &predicted) in out_corr_vals
            .iter_mut()
            .zip(original_vals)
            .zip(predicted_vals)
        {
            // `min_value <= max_value` once `init` has run, so this is the
            // two-sided clamp above.
            let predicted = predicted.max(min_value).min(max_value);
            let value = original.wrapping_sub(predicted);
            *corr = if value < min_correction {
                value.wrapping_add(max_dif)
            } else if value > max_correction {
                value.wrapping_sub(max_dif)
            } else {
                value
            };
        }
    }

    fn encode_transform_data(&mut self, buffer: &mut Vec<u8>) -> Status {
        buffer.extend_from_slice(&self.min_value.to_le_bytes());
        buffer.extend_from_slice(&self.max_value.to_le_bytes());
        Ok(())
    }
}

#[cfg(feature = "decoder")]
pub struct PredictionSchemeWrapDecodingTransform<DataType> {
    num_components: usize,
    min_value: DataType,
    max_value: DataType,
    max_dif: DataType,
    _marker: PhantomData<DataType>,
}

#[cfg(feature = "decoder")]
impl<DataType> Default for PredictionSchemeWrapDecodingTransform<DataType>
where
    DataType: Copy + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "decoder")]
impl<DataType> PredictionSchemeWrapDecodingTransform<DataType>
where
    DataType: Copy + Default,
{
    pub fn new() -> Self {
        Self {
            num_components: 0,
            min_value: DataType::default(),
            max_value: DataType::default(),
            max_dif: DataType::default(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "decoder")]
impl PredictionSchemeDecodingTransform<i32> for PredictionSchemeWrapDecodingTransform<i32> {
    fn get_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::Wrap
    }

    #[inline]
    fn init(&mut self, num_components: usize) -> Status {
        self.num_components = num_components;
        Ok(())
    }

    #[inline(always)]
    fn compute_original_value(&self, predicted_vals: &[i32], data: &mut [i32]) {
        // Left branching on purpose. Both tests are thresholds on decoded data
        // rather than on a pattern, which is the shape where folding a branch
        // into arithmetic usually pays -- but there is no branch here to fold:
        // LLVM already lowers each of these pairs to two `cmov`s and an add,
        // with no jump. Spelling the fold out by hand
        // (`val + (under - over) * max_dif`) replaces those `cmov`s with two
        // `setcc`, a subtract and an `imul` on the dependency chain: 10
        // instructions against 8, and 0.9% slower on a Bunny decode.
        //
        // `decode_transform_data` refuses `min > max`, so at most one test in
        // each pair can hold; that is what makes the two forms equivalent at
        // all, and a unit test pins them against each other.
        for i in 0..self.num_components {
            let mut pred = predicted_vals[i];
            if pred < self.min_value {
                pred = self.min_value;
            } else if pred > self.max_value {
                pred = self.max_value;
            }

            // The add is exact: when the `i32` sum would overflow, it is taken
            // in `i64`, where the value sits at most half a span outside
            // `[min, max]` -- the correction was wrapped into
            // `min_correction..=max_correction` on the way in -- so the single
            // wrap lands on the value the encoder coded. C++ performs this
            // addition in `uint32` -- its own guard against signed overflow --
            // and where the `uint32` sum wraps, its single wrap cannot reach:
            // the reconstruction lands a whole span away, and every later
            // prediction reads the aliased number. See the wrap transform
            // section in COMPATIBILITY.md. Both arms wrap exactly once; the
            // unit test below states the rule as arithmetic.
            let val = match pred.checked_add(data[i]) {
                Some(sum) => {
                    if sum < self.min_value {
                        sum.wrapping_add(self.max_dif)
                    } else if sum > self.max_value {
                        sum.wrapping_sub(self.max_dif)
                    } else {
                        sum
                    }
                }
                None => {
                    let sum = pred as i64 + data[i] as i64;
                    if sum > self.max_value as i64 {
                        (sum - self.max_dif as i64) as i32
                    } else {
                        (sum + self.max_dif as i64) as i32
                    }
                }
            };

            data[i] = val;
        }
    }

    /// The run as a sum, where the stream allows it.
    ///
    /// Each value waits on the one before it, so the run costs what one step's
    /// chain costs: a clamp, an add and a wrap, about 1.2 ns a value. That
    /// chain shortens to the add alone when the run is a modular sum:
    ///
    /// * every correction is under the span in size -- what an encoder writes,
    ///   having wrapped each into half of it -- and the first entry is in
    ///   range. Then the clamp is the identity on every step: a prediction in
    ///   `[min, max]` plus a correction in `(-max_dif, max_dif)` sums into
    ///   `(min - max_dif, max + max_dif)`, where one wrap lands back in
    ///   `[min, max]`, so the next prediction is in range too; and that one
    ///   wrap is the reduction modulo the span, so a value is
    ///   `min + ((v0 - min + the corrections since) mod max_dif)`.
    /// * the span is a power of two, which makes the reduction the low bits of
    ///   a wrapping sum. A quantized attribute is stretched over its whole
    ///   range, `0..=2^bits - 1`, so its span is one.
    ///
    /// Anything else takes the general step, which is what C++ computes. The
    /// same conditions with the wrap kept as two selects on the chain measured
    /// no faster than the general step, so there is no middle path.
    fn compute_original_run(&self, data: &mut [i32], num_components: usize) {
        let (min, max, dif) = (self.min_value, self.max_value, self.max_dif);
        let summed = data.len() > num_components
            && (dif as u32).is_power_of_two()
            && data[..num_components].iter().all(|v| (min..=max).contains(v))
            // A fold rather than `all`, so the pass has no early exit and
            // vectorizes.
            && data[num_components..]
                .iter()
                .fold(true, |ok, &c| ok & (c > -dif) & (c < dif));
        if !summed {
            for i in (num_components..data.len()).step_by(num_components) {
                let (decoded, rest) = data.split_at_mut(i);
                self.compute_original_value(
                    &decoded[i - num_components..],
                    &mut rest[..num_components],
                );
            }
            return;
        }
        // `min + (sum & mask)` is in `[min, max]`, so it does not overflow.
        let mask = dif - 1;
        if num_components == 1 {
            let mut sum = data[0] - min;
            for value in &mut data[1..] {
                sum = sum.wrapping_add(*value);
                *value = min + (sum & mask);
            }
        } else {
            let mut sums: Vec<i32> = data[..num_components].iter().map(|v| v - min).collect();
            for entry in data[num_components..].chunks_exact_mut(num_components) {
                for (sum, value) in sums.iter_mut().zip(entry) {
                    *sum = sum.wrapping_add(*value);
                    *value = min + (*sum & mask);
                }
            }
        }
    }

    fn decode_transform_data(&mut self, buffer: &mut DecoderBuffer) -> Status {
        let truncated = |bound: &str| {
            DracoError::buffer(format!(
                "Stream ends before the wrap transform's {bound} value"
            ))
        };
        let min_value = buffer.decode::<i32>().map_err(|_| truncated("minimum"))?;
        let max_value = buffer.decode::<i32>().map_err(|_| truncated("maximum"))?;

        // Both bounds are read straight off the wire, and everything below
        // assumes the range is non-empty and that its span is representable.
        // Upstream refuses exactly these two cases before accepting the
        // transform; this port did not, so a crafted stream was accepted where
        // C++ rejects it. Without the first check the two range tests in
        // `compute_original_value` stop being mutually exclusive and the port
        // silently disagrees with C++ about which one wins; without the second,
        // `1 + dif` wraps and `max_dif` comes out wrong rather than refused --
        // `min = i32::MIN, max = i32::MAX` yields 0.
        let dif = (max_value as i64) - (min_value as i64);
        if dif < 0 {
            return Err(DracoError::general(format!(
                "Wrap transform's range is empty: minimum {min_value} is above maximum {max_value}"
            )));
        }
        if dif >= i32::MAX as i64 {
            return Err(DracoError::general(format!(
                "Wrap transform's range {min_value}..={max_value} is too wide for its span to be represented"
            )));
        }

        self.min_value = min_value;
        self.max_value = max_value;
        self.max_dif = 1 + dif as i32;

        Ok(())
    }
}

#[cfg(test)]
#[cfg(feature = "decoder")]
mod tests {
    use super::*;
    use crate::prediction_scheme::PredictionSchemeDecodingTransform;

    fn bounds_stream(min_value: i32, max_value: i32) -> Vec<u8> {
        let mut bytes = min_value.to_le_bytes().to_vec();
        bytes.extend_from_slice(&max_value.to_le_bytes());
        bytes
    }

    /// Upstream refuses an empty range before accepting the transform. Without
    /// this, the two range tests in `compute_original_value` can both hold for
    /// one value, and which of them wins is then a silent difference between
    /// this port and C++ rather than something either of them decided.
    #[test]
    fn a_range_whose_minimum_is_above_its_maximum_is_refused() {
        let bytes = bounds_stream(10, 5);
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut transform = PredictionSchemeWrapDecodingTransform::<i32>::new();

        let err = transform
            .decode_transform_data(&mut buffer)
            .expect_err("an empty range is not decodable");
        assert!(
            err.to_string().contains("range is empty"),
            "unexpected error: {err}"
        );
    }

    /// The span is stored as `1 + (max - min)` in an `i32`, so a range that
    /// covers the whole type has no representable span. Computing it anyway
    /// wrapped it to 0 and left the wrap doing nothing.
    #[test]
    fn a_range_too_wide_for_its_span_is_refused() {
        let bytes = bounds_stream(i32::MIN, i32::MAX);
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut transform = PredictionSchemeWrapDecodingTransform::<i32>::new();

        let err = transform
            .decode_transform_data(&mut buffer)
            .expect_err("a span that does not fit is not decodable");
        assert!(
            err.to_string().contains("too wide"),
            "unexpected error: {err}"
        );
    }

    /// Pins the transform against an independent restatement of the same rule,
    /// over the edge cases that reach it through `wrapping_add` -- `i32::MIN`
    /// and `i32::MAX` on both the prediction and the correction, and ranges
    /// that touch either end of the type.
    ///
    /// Written when a branchless rewrite of this loop was tried and measured
    /// slower (see the comment on `compute_original_value`), and kept because
    /// it is what such a rewrite has to satisfy: the arithmetic form and this
    /// one agree only while `min <= max`, which `decode_transform_data`
    /// enforces, and only if the wrapping is reproduced exactly.
    #[test]
    fn the_wrap_matches_an_independent_statement_of_the_same_rule() {
        // The rule, stated as arithmetic rather than as branches: clamp the
        // prediction into the range, add the correction exactly, wrap once.
        // The `i64` add is what makes the wrap exact -- the overflow path of
        // `compute_original_value` exists so this statement holds everywhere.
        fn branching(pred: i32, corr: i32, min_value: i32, max_value: i32, max_dif: i32) -> i32 {
            let pred = pred.clamp(min_value, max_value);
            let sum = pred as i64 + corr as i64;
            if sum > max_value as i64 {
                (sum - max_dif as i64) as i32
            } else if sum < min_value as i64 {
                (sum + max_dif as i64) as i32
            } else {
                sum as i32
            }
        }

        for &(min_value, max_value) in &[(0, 0), (0, 7), (-5, 5), (-100, -1), (i32::MIN, 0)] {
            let max_dif = 1 + ((max_value as i64) - (min_value as i64)) as i32;
            let mut transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
            transform.min_value = min_value;
            transform.max_value = max_value;
            transform.max_dif = max_dif;
            transform
                .init(1)
                .expect("the wrap transform accepts any component count");

            for pred in [i32::MIN, -7, -1, 0, 1, 7, i32::MAX] {
                for corr in [i32::MIN, -8, -1, 0, 1, 8, i32::MAX] {
                    let mut out = [corr];
                    transform.compute_original_value(&[pred], &mut out);
                    assert_eq!(
                        out[0],
                        branching(pred, corr, min_value, max_value, max_dif),
                        "min={min_value} max={max_value} pred={pred} corr={corr}"
                    );
                }
            }
        }
    }

    /// A run reconstructs to what entry-by-entry reconstruction gives, as a sum
    /// and not: corrections an encoder writes over a power-of-two span, which
    /// sum, including spans at the edges of `i32` and one of 2^30; and
    /// corrections of any size, or a span that is not a power of two, which do
    /// not. Both paths are counted, so neither passes on the other's code.
    #[test]
    fn a_run_is_what_entry_by_entry_reconstruction_gives() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut draw = |range: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % range
        };
        let mut summed_runs = 0;
        let mut general_runs = 0;
        for (min, max) in [
            (0i32, 63),
            (-100, 155),
            (-7, 8),
            (5, 5),
            (0, 99),
            (i32::MIN + 10, i32::MIN + 265),
            (i32::MAX - 265, i32::MAX - 10),
            (i32::MIN + 10, i32::MIN + 300),
            (-(1 << 29), (1 << 29) - 1),
            (-(1 << 29), 1 << 29),
        ] {
            let bytes = bounds_stream(min, max);
            let mut transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
            transform
                .decode_transform_data(&mut DecoderBuffer::new(&bytes))
                .expect("a valid range");
            let dif = i64::from(max) - i64::from(min) + 1;
            for num_components in [1usize, 2, 3, 4] {
                transform.init(num_components).unwrap();
                for encoder_like in [true, false] {
                    let mut data: Vec<i32> = (0..num_components * 500)
                        .map(|_| {
                            if encoder_like {
                                (draw(dif as u64) as i64 - dif / 2) as i32
                            } else {
                                draw(u64::from(u32::MAX)) as u32 as i32
                            }
                        })
                        .collect();
                    for value in &mut data[..num_components] {
                        *value = (i64::from(min) + draw(dif as u64) as i64) as i32;
                    }

                    let mut expected = data.clone();
                    for i in (num_components..expected.len()).step_by(num_components) {
                        let (decoded, rest) = expected.split_at_mut(i);
                        transform.compute_original_value(
                            &decoded[i - num_components..],
                            &mut rest[..num_components],
                        );
                    }
                    let mut run = data.clone();
                    transform.compute_original_run(&mut run, num_components);
                    assert_eq!(
                        run, expected,
                        "[{min}, {max}], {num_components} components, encoder-like {encoder_like}"
                    );

                    let summed = (dif as u64).is_power_of_two()
                        && data[num_components..]
                            .iter()
                            .all(|&c| i64::from(c).abs() < dif);
                    if summed {
                        summed_runs += 1;
                    } else {
                        general_runs += 1;
                    }
                }
            }
        }
        assert!(
            summed_runs >= 24 && general_runs >= 24,
            "{summed_runs} summed, {general_runs} general"
        );
    }
}

#[cfg(all(test, feature = "encoder"))]
mod encoder_tests {
    use super::*;
    use crate::prediction_scheme::PredictionSchemeEncodingTransform;

    /// The flat run is the per-entry correction, entry for entry: predictions
    /// either side of the range the clamp pulls in, and differences either side
    /// of the bounds the wrap folds back, over even and odd spans.
    #[test]
    fn a_run_of_corrections_is_each_entry_corrected_alone() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut draw = |range: i64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % range as u64) as i64
        };
        for (min, max) in [(-100i32, 155), (0, 1000), (-7, 8)] {
            for num_components in [1usize, 3] {
                let span = i64::from(max) - i64::from(min) + 1;
                let mut values: Vec<i32> = (0..300)
                    .map(|_| (i64::from(min) + draw(span)) as i32)
                    .collect();
                values[0] = min;
                values[1] = max;
                let mut transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                transform.init(&values, values.len(), num_components);
                let predicted: Vec<i32> = (0..values.len())
                    .map(|_| (i64::from(min) - span / 2 + draw(2 * span)) as i32)
                    .collect();

                let mut expected = vec![0; values.len()];
                for ((original, predicted), corr) in values
                    .chunks_exact(num_components)
                    .zip(predicted.chunks_exact(num_components))
                    .zip(expected.chunks_exact_mut(num_components))
                {
                    transform.compute_correction(original, predicted, corr);
                }
                let mut run = vec![0; values.len()];
                transform.compute_corrections(&values, &predicted, &mut run, num_components);
                assert_eq!(run, expected, "[{min}, {max}], {num_components} components");
            }
        }
    }
}
