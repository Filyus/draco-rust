//! Prediction-scheme abstraction and method/transform enums.
//!
//! A prediction scheme predicts each attribute value from previously coded
//! values so that only a small residual is entropy-coded. Two enums identify
//! what a bitstream used — [`PredictionSchemeMethod`] (which predictor) and
//! `PredictionSchemeTransformType` (how prediction and correction combine) —
//! and [`PredictionScheme`] is the trait implemented by the concrete predictors
//! and transforms in the sibling `prediction_scheme_*` modules. Port of Draco's
//! `prediction_scheme.h` family.

use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::PointIndex;
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
            4 => Ok(PredictionSchemeTransformType::Parallelogram),
            5 => Ok(PredictionSchemeTransformType::TexCoordsPortable),
            6 => Ok(PredictionSchemeTransformType::GeometricNormal),
            7 => Ok(PredictionSchemeTransformType::MultiParallelogram),
            8 => Ok(PredictionSchemeTransformType::ConstrainedMultiParallelogram),
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
    fn set_parent_attribute(&mut self, att: &'a PointAttribute) -> Status;
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
pub trait PredictionSchemeDecodingTransform<DataType, CorrType> {
    fn init(&mut self, num_components: usize);
    fn compute_original_value(
        &self,
        predicted_vals: &[DataType],
        corr_vals: &[CorrType],
        out_original_vals: &mut [DataType],
    );
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
pub trait PredictionSchemeDecoder<'a, DataType, CorrType>: PredictionScheme<'a> {
    fn compute_original_values(
        &mut self,
        in_corr: &[CorrType],
        out_data: &mut [DataType],
        size: usize,
        num_components: usize,
        entry_to_point_id_map: Option<EntryToPointIdMap<'_>>,
    ) -> Status;

    fn decode_prediction_data(
        &mut self,
        buffer: &mut crate::decoder_buffer::DecoderBuffer,
    ) -> Status;
}
