use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::prediction_scheme::{
    PredictionScheme, PredictionSchemeMethod, PredictionSchemeTransformType,
};
use std::marker::PhantomData;

#[cfg(feature = "decoder")]
use std::ops::Add;

#[cfg(feature = "encoder")]
use std::ops::Sub;

#[cfg(feature = "decoder")]
use crate::prediction_scheme::{PredictionSchemeDecoder, PredictionSchemeDecodingTransform};

#[cfg(feature = "encoder")]
use crate::prediction_scheme::{PredictionSchemeEncoder, PredictionSchemeEncodingTransform};

#[cfg(feature = "encoder")]
pub struct PredictionSchemeDeltaEncodingTransform<DataType, CorrType> {
    num_components: usize,
    _marker: PhantomData<(DataType, CorrType)>,
}

#[cfg(feature = "encoder")]
impl<DataType, CorrType> Default for PredictionSchemeDeltaEncodingTransform<DataType, CorrType> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "encoder")]
impl<DataType, CorrType> PredictionSchemeDeltaEncodingTransform<DataType, CorrType> {
    pub fn new() -> Self {
        Self {
            num_components: 0,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "encoder")]
impl<DataType, CorrType> PredictionSchemeEncodingTransform<DataType, CorrType>
    for PredictionSchemeDeltaEncodingTransform<DataType, CorrType>
where
    DataType: Copy + Sub<Output = CorrType>,
    CorrType: Copy,
{
    fn init(&mut self, _orig_data: &[DataType], _size: usize, num_components: usize) {
        self.num_components = num_components;
    }

    fn compute_correction(
        &self,
        original_vals: &[DataType],
        predicted_vals: &[DataType],
        out_corr_vals: &mut [CorrType],
    ) {
        for i in 0..self.num_components {
            out_corr_vals[i] = original_vals[i] - predicted_vals[i];
        }
    }

    fn encode_transform_data(&mut self, _buffer: &mut Vec<u8>) -> bool {
        true
    }

    fn get_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::Delta
    }
}

#[cfg(feature = "decoder")]
pub struct PredictionSchemeDeltaDecodingTransform<DataType, CorrType> {
    num_components: usize,
    _marker: PhantomData<(DataType, CorrType)>,
}

#[cfg(feature = "decoder")]
impl<DataType, CorrType> Default for PredictionSchemeDeltaDecodingTransform<DataType, CorrType> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "decoder")]
impl<DataType, CorrType> PredictionSchemeDeltaDecodingTransform<DataType, CorrType> {
    pub fn new() -> Self {
        Self {
            num_components: 0,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "decoder")]
impl<DataType, CorrType> PredictionSchemeDecodingTransform<DataType, CorrType>
    for PredictionSchemeDeltaDecodingTransform<DataType, CorrType>
where
    DataType: Copy + Add<CorrType, Output = DataType>,
    CorrType: Copy,
{
    #[inline]
    fn init(&mut self, num_components: usize) {
        self.num_components = num_components;
    }

    #[inline]
    fn compute_original_value(
        &self,
        predicted_vals: &[DataType],
        corr_vals: &[CorrType],
        out_original_vals: &mut [DataType],
    ) {
        for i in 0..self.num_components {
            out_original_vals[i] = predicted_vals[i] + corr_vals[i];
        }
    }

    fn decode_transform_data(
        &mut self,
        _buffer: &mut crate::decoder_buffer::DecoderBuffer,
    ) -> bool {
        true
    }

    fn get_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::Delta
    }
}

#[cfg(feature = "encoder")]
pub struct PredictionSchemeDeltaEncoder<DataType, CorrType, Transform> {
    transform: Transform,
    _marker: PhantomData<(DataType, CorrType)>,
}

#[cfg(feature = "encoder")]
impl<DataType, CorrType, Transform> PredictionSchemeDeltaEncoder<DataType, CorrType, Transform>
where
    DataType: Copy + Default,
    CorrType: Copy + Default,
    Transform: PredictionSchemeEncodingTransform<DataType, CorrType>,
{
    pub fn new(transform: Transform) -> Self {
        Self {
            transform,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "encoder")]
impl<DataType, CorrType, Transform> PredictionScheme<'static>
    for PredictionSchemeDeltaEncoder<DataType, CorrType, Transform>
where
    Transform: PredictionSchemeEncodingTransform<DataType, CorrType>,
{
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::Difference
    }

    fn is_initialized(&self) -> bool {
        true
    }

    fn get_num_parent_attributes(&self) -> i32 {
        0
    }

    fn get_parent_attribute_type(&self, _i: i32) -> GeometryAttributeType {
        GeometryAttributeType::Invalid
    }

    fn set_parent_attribute(&mut self, _att: &'static PointAttribute) -> bool {
        false
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }

    fn are_corrections_positive(&self) -> bool {
        self.transform.are_corrections_positive()
    }
}

#[cfg(feature = "encoder")]
impl<DataType, CorrType, Transform> PredictionSchemeEncoder<'static, DataType, CorrType>
    for PredictionSchemeDeltaEncoder<DataType, CorrType, Transform>
where
    DataType: Copy + Default,
    CorrType: Copy + Default,
    Transform: PredictionSchemeEncodingTransform<DataType, CorrType>,
{
    fn compute_correction_values(
        &mut self,
        in_data: &[DataType],
        out_corr: &mut [CorrType],
        size: usize,
        num_components: usize,
        _entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> bool {
        self.transform.init(in_data, size, num_components);

        // Encode data from the back using D(i) = D(i) - D(i - 1).
        let mut i = size - num_components;
        while i > 0 {
            let original = &in_data[i..i + num_components];
            let predicted = &in_data[i - num_components..i];
            let corr = &mut out_corr[i..i + num_components];
            self.transform.compute_correction(original, predicted, corr);

            if i < num_components {
                break;
            }
            i -= num_components;
        }

        // Encode correction for the first element.
        // Pre-allocate zero values outside the loop
        let zero_vals = vec![DataType::default(); num_components];
        let original = &in_data[0..num_components];
        let corr = &mut out_corr[0..num_components];
        self.transform
            .compute_correction(original, &zero_vals, corr);

        true
    }

    fn encode_prediction_data(&mut self, buffer: &mut Vec<u8>) -> bool {
        self.transform.encode_transform_data(buffer)
    }
}

#[cfg(feature = "decoder")]
pub struct PredictionSchemeDeltaDecoder<DataType, CorrType, Transform> {
    transform: Transform,
    _marker: PhantomData<(DataType, CorrType)>,
}

#[cfg(feature = "decoder")]
impl<DataType, CorrType, Transform> PredictionSchemeDeltaDecoder<DataType, CorrType, Transform>
where
    DataType: Copy + Default,
    CorrType: Copy + Default,
    Transform: PredictionSchemeDecodingTransform<DataType, CorrType>,
{
    pub fn new(transform: Transform) -> Self {
        Self {
            transform,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "decoder")]
impl<DataType, CorrType, Transform> PredictionScheme<'static>
    for PredictionSchemeDeltaDecoder<DataType, CorrType, Transform>
where
    Transform: PredictionSchemeDecodingTransform<DataType, CorrType>,
{
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::Difference
    }

    fn is_initialized(&self) -> bool {
        true
    }

    fn get_num_parent_attributes(&self) -> i32 {
        0
    }

    fn get_parent_attribute_type(&self, _i: i32) -> GeometryAttributeType {
        GeometryAttributeType::Invalid
    }

    fn set_parent_attribute(&mut self, _att: &'static PointAttribute) -> bool {
        false
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }

    fn are_corrections_positive(&self) -> bool {
        self.transform.are_corrections_positive()
    }
}

#[cfg(feature = "decoder")]
impl<DataType, CorrType, Transform> PredictionSchemeDecoder<'static, DataType, CorrType>
    for PredictionSchemeDeltaDecoder<DataType, CorrType, Transform>
where
    DataType: Copy + Default,
    CorrType: Copy + Default,
    Transform: PredictionSchemeDecodingTransform<DataType, CorrType>,
{
    fn compute_original_values(
        &mut self,
        in_corr: &[CorrType],
        out_data: &mut [DataType],
        size: usize,
        num_components: usize,
        _entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> bool {
        self.transform.init(num_components);

        // Decode the original value for the first element.
        // Pre-allocate buffer that will be reused for zero_vals and then predicted vals
        let mut predicted = vec![DataType::default(); num_components];
        let corr = &in_corr[0..num_components];
        let out = &mut out_data[0..num_components];
        self.transform.compute_original_value(&predicted, corr, out); // predicted is all zeros here

        // Decode data from the front using D(i) = D(i) + D(i - 1).
        for i in (num_components..size).step_by(num_components) {
            // Copy previous values to the pre-allocated buffer
            predicted.copy_from_slice(&out_data[i - num_components..i]);

            let corr = &in_corr[i..i + num_components];
            let out = &mut out_data[i..i + num_components];

            self.transform.compute_original_value(&predicted, corr, out);
        }

        true
    }

    fn decode_prediction_data(
        &mut self,
        buffer: &mut crate::decoder_buffer::DecoderBuffer,
    ) -> bool {
        self.transform.decode_transform_data(buffer)
    }
}
