use crate::prediction_scheme::{PredictionSchemeEncodingTransform, PredictionSchemeTransformType};
use crate::prediction_scheme_normal_octahedron_canonicalized_transform_base::PredictionSchemeNormalOctahedronCanonicalizedTransformBase;

pub struct PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform {
    base: PredictionSchemeNormalOctahedronCanonicalizedTransformBase,
    num_components: usize,
}

impl PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform {
    pub fn new(max_quantized_value: i32) -> Self {
        Self {
            base: PredictionSchemeNormalOctahedronCanonicalizedTransformBase::new(
                max_quantized_value,
            ),
            num_components: 0,
        }
    }
}

impl PredictionSchemeEncodingTransform<i32, i32>
    for PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform
{
    fn init(&mut self, _orig_data: &[i32], _size: usize, num_components: usize) {
        self.num_components = num_components;
    }

    fn encode_transform_data(&mut self, buffer: &mut Vec<u8>) -> bool {
        buffer.extend_from_slice(&self.base.base().max_quantized_value().to_le_bytes());
        buffer.extend_from_slice(&self.base.base().center_value().to_le_bytes());
        true
    }

    fn compute_correction(&self, orig_vals: &[i32], pred_vals: &[i32], out_corr_vals: &mut [i32]) {
        let center = self.base.base().center_value();

        let mut orig = [orig_vals[0] - center, orig_vals[1] - center];
        let mut pred = [pred_vals[0] - center, pred_vals[1] - center];

        if !self.base.base().is_in_diamond(pred[0], pred[1]) {
            {
                let (s, t) = orig.split_at_mut(1);
                self.base.base().invert_diamond(&mut s[0], &mut t[0]);
            }
            {
                let (s, t) = pred.split_at_mut(1);
                self.base.base().invert_diamond(&mut s[0], &mut t[0]);
            }
        }

        // Match C++: Only rotate when pred is not in the bottom-left region.
        if !self.base.is_in_bottom_left(&pred) {
            let rotation_count = self.base.get_rotation_count(&pred);
            pred = self.base.rotate_point(&pred, rotation_count);
            orig = self.base.rotate_point(&orig, rotation_count);
        }

        out_corr_vals[0] = self.base.base().make_positive(orig[0] - pred[0]);
        out_corr_vals[1] = self.base.base().make_positive(orig[1] - pred[1]);
    }

    fn get_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::NormalOctahedronCanonicalized
    }

    fn are_corrections_positive(&self) -> bool {
        // Corrections from octahedron transforms are always in [0, max_quantized_value]
        // because make_positive() ensures they are non-negative
        true
    }
}
