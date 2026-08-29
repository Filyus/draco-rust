//! Prediction-scheme abstraction and method/transform enums.
//!
//! A prediction scheme predicts each attribute value from previously coded
//! values so that only a small residual is entropy-coded. Two enums identify
//! what a bitstream used — [`PredictionSchemeMethod`] (which predictor) and
//! `PredictionSchemeTransformType` (how prediction and correction combine) —
//! and [`PredictionScheme`] is the trait implemented by the concrete predictors
//! and transforms in the sibling `prediction_scheme_*` modules. Port of Draco's
//! `prediction_scheme.h` family.

use crate::geometry_attribute::GeometryAttributeType;
use crate::geometry_indices::PointIndex;
use crate::portable_attribute::PredictionParent;
use crate::status::Status;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictionSchemeMethod {
    // Special value indicating that no prediction scheme was used.
    // CRITICAL: These values must match C++ enum values exactly:
    //   C++: PREDICTION_NONE = -2, PREDICTION_UNDEFINED = -1
    None = -2,
    Undefined = -1,
    Difference = 0,
    MeshPredictionParallelogram = 1,
    MeshPredictionMultiParallelogram = 2,
    MeshPredictionTexCoordsDeprecated = 3,
    MeshPredictionConstrainedMultiParallelogram = 4,
    MeshPredictionTexCoordsPortable = 5,
    MeshPredictionGeometricNormal = 6,
}

impl TryFrom<u8> for PredictionSchemeMethod {
    type Error = ();
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(PredictionSchemeMethod::Difference),
            1 => Ok(PredictionSchemeMethod::MeshPredictionParallelogram),
            2 => Ok(PredictionSchemeMethod::MeshPredictionMultiParallelogram),
            3 => Ok(PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated),
            4 => Ok(PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram),
            5 => Ok(PredictionSchemeMethod::MeshPredictionTexCoordsPortable),
            6 => Ok(PredictionSchemeMethod::MeshPredictionGeometricNormal),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictionSchemeTransformType {
    None = -1,
    Delta = 0,
    Wrap = 1,
    NormalOctahedron = 2,
    NormalOctahedronCanonicalized = 3,
    Parallelogram = 4,
    TexCoordsPortable = 5,
    GeometricNormal = 6,
    MultiParallelogram = 7,
    ConstrainedMultiParallelogram = 8,
}

#[derive(Clone, Copy)]
pub enum EntryToPointIdMap<'a> {
    PointIndices(&'a [PointIndex]),
    U32(&'a [u32]),
    /// The identity of a given length: entry `i` is point `i`.
    ///
    /// A sequential decode has no permutation to speak of, and writing one out
    /// costs four bytes per point taken from a count the header supplies --
    /// which is the shape of every allocation this crate has had to bound. The
    /// other two variants are for an order that really is an order.
    Identity(usize),
}

impl<'a> EntryToPointIdMap<'a> {
    #[inline]
    pub fn from_point_indices(point_ids: &'a [PointIndex]) -> Self {
        Self::PointIndices(point_ids)
    }

    #[inline]
    pub fn from_u32_slice(point_ids: &'a [u32]) -> Self {
        Self::U32(point_ids)
    }

    /// The identity over `num_points` entries, without materializing it.
    #[inline]
    pub fn identity(num_points: usize) -> Self {
        Self::Identity(num_points)
    }

    #[inline]
    pub fn len(self) -> usize {
        match self {
            Self::PointIndices(point_ids) => point_ids.len(),
            Self::U32(point_ids) => point_ids.len(),
            Self::Identity(num_points) => num_points,
        }
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get(self, index: usize) -> Option<u32> {
        match self {
            Self::PointIndices(point_ids) => point_ids.get(index).map(|p| p.0),
            Self::U32(point_ids) => point_ids.get(index).copied(),
            Self::Identity(num_points) => {
                (index < num_points).then(|| u32::try_from(index).unwrap_or(u32::MAX))
            }
        }
    }
}

impl TryFrom<u8> for PredictionSchemeTransformType {
    type Error = ();
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(PredictionSchemeTransformType::Delta),
            1 => Ok(PredictionSchemeTransformType::Wrap),
            2 => Ok(PredictionSchemeTransformType::NormalOctahedron),
            3 => Ok(PredictionSchemeTransformType::NormalOctahedronCanonicalized),
            // Nothing above three. The bitstream's transform types end at
            // `NUM_PREDICTION_SCHEME_TRANSFORM_TYPES == 4`, and upstream's
            // decoder refuses a byte at or past it before it builds anything.
            // The variants this used to accept here name predictors, not
            // transforms; no encoder writes them, so admitting them only let a
            // stream through that C++ Draco rejects -- and it was let through
            // to the wrap transform anyway, since nothing decodes them.
            _ => Err(()),
        }
    }
}

/// A predictor of attribute values from previously coded ones.
///
/// Fallible methods across this family return [`Status`] and name what went
/// wrong. Through 1.x they returned a bare `bool`, so a scheme that refused the
/// parent attribute it was given, one that ran out of buffer, and one whose
/// corrections did not fit the output all reported the same `false`.
/// `is_initialized` and `are_corrections_positive` stay `bool`: they are
/// predicates, not outcomes.
pub trait PredictionScheme<'a> {
    fn get_prediction_method(&self) -> PredictionSchemeMethod;
    fn is_initialized(&self) -> bool;
    fn get_num_parent_attributes(&self) -> i32;
    fn get_parent_attribute_type(&self, i: i32) -> GeometryAttributeType;
    /// Binds the parent a scheme predicts from.
    ///
    /// A [`PredictionParent`], not a `&PointAttribute`: a scheme may not read
    /// attribute storage, only the portable values the parent hands out. See
    /// `portable_attribute`.
    fn set_parent_attribute(&mut self, parent: PredictionParent<'a>) -> Status;
    fn get_transform_type(&self) -> PredictionSchemeTransformType;

    /// Returns true if the correction values are always positive (non-negative).
    /// This is used to determine whether to apply ZigZag encoding to corrections.
    /// For normal octahedron transforms, corrections are already in [0, max_value],
    /// so no ZigZag encoding is needed.
    fn are_corrections_positive(&self) -> bool {
        false
    }
}

pub trait PredictionSchemeEncodingTransform<DataType, CorrType> {
    fn init(&mut self, orig_data: &[DataType], size: usize, num_components: usize);
    fn compute_correction(
        &self,
        original_vals: &[DataType],
        predicted_vals: &[DataType],
        out_corr_vals: &mut [CorrType],
    );
    fn encode_transform_data(&mut self, buffer: &mut Vec<u8>) -> Status;
    fn get_type(&self) -> PredictionSchemeTransformType;

    /// Returns true if the corrections produced by this transform are always positive.
    fn are_corrections_positive(&self) -> bool {
        false
    }
}

#[cfg(feature = "decoder")]
pub trait PredictionSchemeDecodingTransform<DataType> {
    /// Announces the component count the scheme will hand to
    /// `compute_original_value`, and is the transform's one chance to refuse
    /// it: the octahedral transforms read and write a coordinate pair and have
    /// no meaning at any other width.
    fn init(&mut self, num_components: usize) -> Status;
    /// Reconstructs one entry in place: `data` holds the correction on entry
    /// and the original value on return. The single-buffer contract mirrors
    /// upstream, whose decoders pass the same pointer as both `in_corr` and
    /// `out_data`; it is what lets the sequential decoder run prediction on
    /// the correction buffer itself instead of allocating a second one.
    fn compute_original_value(&self, predicted_vals: &[DataType], data: &mut [DataType]);
    fn decode_transform_data(
        &mut self,
        buffer: &mut crate::decoder_buffer::DecoderBuffer,
    ) -> Status;
    fn get_type(&self) -> PredictionSchemeTransformType;

    /// Returns true if the corrections are always positive (no ZigZag encoding needed).
    fn are_corrections_positive(&self) -> bool {
        false
    }
}

pub trait PredictionSchemeEncoder<'a, DataType, CorrType>: PredictionScheme<'a> {
    fn compute_correction_values(
        &mut self,
        in_data: &[DataType],
        out_corr: &mut [CorrType],
        size: usize,
        num_components: usize,
        entry_to_point_id_map: Option<EntryToPointIdMap<'_>>,
    ) -> Status;

    fn encode_prediction_data(&mut self, buffer: &mut Vec<u8>) -> Status;
}

#[cfg(feature = "decoder")]
pub trait PredictionSchemeDecoder<'a, DataType>: PredictionScheme<'a> {
    /// Reconstructs all values in place: `data` holds the decoded corrections
    /// on entry and the original values on return. Safe for every scheme
    /// because each entry reads its correction only at the offset it is about
    /// to write, and predictions come from entries already reconstructed.
    fn compute_original_values(
        &mut self,
        data: &mut [DataType],
        size: usize,
        num_components: usize,
        entry_to_point_id_map: Option<EntryToPointIdMap<'_>>,
    ) -> Status;

    fn decode_prediction_data(
        &mut self,
        buffer: &mut crate::decoder_buffer::DecoderBuffer,
    ) -> Status;
}

#[cfg(test)]
mod tests {
    use super::PredictionSchemeTransformType;

    /// The bitstream has four transform types, and a decoder that accepts a
    /// fifth accepts files C++ Draco refuses -- its own check is
    /// `transform_type >= NUM_PREDICTION_SCHEME_TRANSFORM_TYPES`, run before
    /// it builds anything. This enum also carries names for the *predictors*,
    /// which share no numbering with the transforms and were being parsed as
    /// if they did.
    #[test]
    fn only_the_four_transform_types_the_bitstream_has_are_accepted() {
        for byte in 0..=3u8 {
            assert!(
                PredictionSchemeTransformType::try_from(byte).is_ok(),
                "byte {byte} names a transform the format defines"
            );
        }
        for byte in 4..=255u8 {
            assert!(
                PredictionSchemeTransformType::try_from(byte).is_err(),
                "byte {byte} is past the last transform type and must be refused"
            );
        }
    }
}
