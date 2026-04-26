use crate::decoder_buffer::DecoderBuffer;
use crate::prediction_scheme::{PredictionSchemeDecodingTransform, PredictionSchemeTransformType};
use crate::prediction_scheme_normal_octahedron_canonicalized_transform_base::PredictionSchemeNormalOctahedronCanonicalizedTransformBase;

pub struct PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform {
    base: PredictionSchemeNormalOctahedronCanonicalizedTransformBase,
    num_components: usize,
}

impl Default for PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform {
    fn default() -> Self {
        Self::new()
    }
}

impl PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform {
    pub fn new() -> Self {
        Self {
            base: PredictionSchemeNormalOctahedronCanonicalizedTransformBase::new(0),
            num_components: 0,
        }
    }

    pub fn max_quantized_value(&self) -> i32 {
        self.base.base().max_quantized_value()
    }

    pub fn quantization_bits(&self) -> i32 {
        self.base.base().quantization_bits()
    }
}

impl PredictionSchemeDecodingTransform<i32, i32>
    for PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform
{
    fn init(&mut self, num_components: usize) {
        self.num_components = num_components;
    }

    fn decode_transform_data(&mut self, buffer: &mut DecoderBuffer) -> bool {
        let max_quantized_value: i32;
        let _center_value: i32;

        if let Ok(val) = buffer.decode::<i32>() {
            max_quantized_value = val;
        } else {
            return false;
        }

        if let Ok(val) = buffer.decode::<i32>() {
            _center_value = val;
        } else {
            return false;
        }

        if !self
            .base
            .base_mut()
            .set_max_quantized_value(max_quantized_value)
        {
            return false;
        }
        // Account for wrong values (e.g., due to stream mismatch/fuzzing).
        // C++ requires quantization bits in [2, 30].
        let q = self.base.base().quantization_bits();
        if !(2..=30).contains(&q) {
            return false;
        }
        true
    }

    fn compute_original_value(
        &self,
        pred_vals: &[i32],
        corr_vals: &[i32],
        out_orig_vals: &mut [i32],
    ) {
        let center = self.base.base().center_value();

        let mut pred = [pred_vals[0] - center, pred_vals[1] - center];
        let corr = [corr_vals[0], corr_vals[1]];

        let pred_is_in_diamond = self.base.base().is_in_diamond(pred[0], pred[1]);
        if !pred_is_in_diamond {
            let (s, t) = pred.split_at_mut(1);
            self.base.base().invert_diamond(&mut s[0], &mut t[0]);
        }

        let pred_is_in_bottom_left = self.base.is_in_bottom_left(&pred);
        let rotation_count = self.base.get_rotation_count(&pred);

        if !pred_is_in_bottom_left {
            pred = self.base.rotate_point(&pred, rotation_count);
        }

        let mut orig = [0; 2];
        orig[0] = self.base.base().mod_max(pred[0].wrapping_add(corr[0]));
        orig[1] = self.base.base().mod_max(pred[1].wrapping_add(corr[1]));

        if !pred_is_in_bottom_left {
            let reverse_rotation_count = (4 - rotation_count) % 4;
            orig = self.base.rotate_point(&orig, reverse_rotation_count);
        }

        if !pred_is_in_diamond {
            let (s, t) = orig.split_at_mut(1);
            self.base.base().invert_diamond(&mut s[0], &mut t[0]);
        }

        out_orig_vals[0] = orig[0] + center;
        out_orig_vals[1] = orig[1] + center;
    }

    fn get_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::NormalOctahedronCanonicalized
    }

    fn are_corrections_positive(&self) -> bool {
        // Corrections from octahedron transforms are always in [0, max_quantized_value]
        true
    }
}
