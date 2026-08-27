//! Canonicalized octahedral-normal decoding transform.
//!
//! Decode side of the canonicalized normal transform: reads quantization
//! parameters and inverts the canonical-octant rotation to reconstruct the
//! octahedral normal prediction. Port of Draco's
//! `prediction_scheme_normal_octahedron_canonicalized_decoding_transform.h`.

use crate::decoder_buffer::DecoderBuffer;
use crate::prediction_scheme::{PredictionSchemeDecodingTransform, PredictionSchemeTransformType};
use crate::prediction_scheme_normal_octahedron_canonicalized_transform_base::PredictionSchemeNormalOctahedronCanonicalizedTransformBase;
use crate::status::{DracoError, Status};

pub struct PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform {
    base: PredictionSchemeNormalOctahedronCanonicalizedTransformBase,
    num_components: usize,
    /// When false, behaves as the pre-canonicalized octahedron transform (id 2,
    /// Draco <= 0.9.1): it skips only the canonical-octant rotation while keeping
    /// the shared diamond inversion/correction steps. The two share enough math
    /// that a flag is cleaner than a separate type.
    is_canonicalized: bool,
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
            is_canonicalized: true,
        }
    }

    /// Selects the canonicalized (id 3) vs the legacy non-canonicalized (id 2)
    /// octahedron transform. Defaults to canonicalized.
    pub fn set_canonicalized(&mut self, canonicalized: bool) {
        self.is_canonicalized = canonicalized;
    }

    pub fn max_quantized_value(&self) -> i32 {
        self.base.base().max_quantized_value()
    }

    pub fn quantization_bits(&self) -> i32 {
        self.base.base().quantization_bits()
    }
}

impl PredictionSchemeDecodingTransform<i32>
    for PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform
{
    fn init(&mut self, num_components: usize) -> Status {
        // An octahedral value is a coordinate pair, and this reads and writes
        // both without asking. Whichever scheme drives it -- geometric normal,
        // delta, a parallelogram -- an attribute of any other width is one no
        // encoder here produces, and refusing it once, here, covers every
        // pairing the stream can name.
        if num_components != 2 {
            return Err(DracoError::invalid_parameter(format!(
                "Octahedral transform needs 2 components, got {num_components}"
            )));
        }
        self.num_components = num_components;
        Ok(())
    }

    fn decode_transform_data(&mut self, buffer: &mut DecoderBuffer) -> Status {
        let truncated = |what: &str| {
            DracoError::buffer(format!(
                "Stream ends before the octahedral transform's {what}"
            ))
        };
        let max_quantized_value = buffer
            .decode::<i32>()
            .map_err(|_| truncated("maximum quantized value"))?;
        let _center_value = buffer
            .decode::<i32>()
            .map_err(|_| truncated("center value"))?;

        if !self
            .base
            .base_mut()
            .set_max_quantized_value(max_quantized_value)
        {
            return Err(DracoError::invalid_parameter(format!(
                "Octahedral maximum quantized value {max_quantized_value} is not representable"
            )));
        }
        // Account for wrong values (e.g., due to stream mismatch/fuzzing).
        // C++ requires quantization bits in [2, 30].
        let q = self.base.base().quantization_bits();
        if !(2..=30).contains(&q) {
            return Err(DracoError::invalid_parameter(format!(
                "Octahedral quantization bits {q} outside the supported range 2..=30"
            )));
        }
        Ok(())
    }

    fn compute_original_value(&self, pred_vals: &[i32], data: &mut [i32]) {
        let center = self.base.base().center_value();

        let mut pred = [pred_vals[0] - center, pred_vals[1] - center];
        let corr = [data[0], data[1]];

        let pred_is_in_diamond = self.base.base().is_in_diamond(pred[0], pred[1]);
        if !pred_is_in_diamond {
            let (s, t) = pred.split_at_mut(1);
            self.base.base().invert_diamond(&mut s[0], &mut t[0]);
        }

        // The canonical-octant rotation is only part of the canonicalized (id 3)
        // transform; the legacy id 2 transform applies the shared invert-in,
        // correction, and trailing diamond inversion.
        let mut pred_is_in_bottom_left = false;
        let mut rotation_count = 0;
        if self.is_canonicalized {
            pred_is_in_bottom_left = self.base.is_in_bottom_left(&pred);
            rotation_count = self.base.get_rotation_count(&pred);
            if !pred_is_in_bottom_left {
                pred = self.base.rotate_point(&pred, rotation_count);
            }
        }

        let mut orig = [0; 2];
        orig[0] = self.base.base().mod_max(pred[0].wrapping_add(corr[0]));
        orig[1] = self.base.base().mod_max(pred[1].wrapping_add(corr[1]));

        // Only the canonical-octant rotation is canonicalized-specific; the
        // trailing diamond inversion is part of both the old (id 2) and the
        // canonicalized (id 3) transform.
        if self.is_canonicalized && !pred_is_in_bottom_left {
            let reverse_rotation_count = (4 - rotation_count) % 4;
            orig = self.base.rotate_point(&orig, reverse_rotation_count);
        }
        if !pred_is_in_diamond {
            let (s, t) = orig.split_at_mut(1);
            self.base.base().invert_diamond(&mut s[0], &mut t[0]);
        }

        data[0] = orig[0] + center;
        data[1] = orig[1] + center;
    }

    fn get_type(&self) -> PredictionSchemeTransformType {
        if self.is_canonicalized {
            PredictionSchemeTransformType::NormalOctahedronCanonicalized
        } else {
            PredictionSchemeTransformType::NormalOctahedron
        }
    }

    fn are_corrections_positive(&self) -> bool {
        // Corrections from octahedron transforms are always in [0, max_quantized_value]
        true
    }
}
