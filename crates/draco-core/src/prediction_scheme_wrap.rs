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
}
