use crate::corner_table::CornerTable;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::{CornerIndex, INVALID_CORNER_INDEX};
use crate::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use crate::prediction_scheme::{
    PredictionScheme, PredictionSchemeMethod, PredictionSchemeTransformType,
};
use std::marker::PhantomData;

#[cfg(feature = "decoder")]
use crate::prediction_scheme::{PredictionSchemeDecoder, PredictionSchemeDecodingTransform};

#[cfg(feature = "encoder")]
use crate::prediction_scheme::{PredictionSchemeEncoder, PredictionSchemeEncodingTransform};

pub trait ParallelogramDataType: Copy + Default + 'static {
    fn compute_parallelogram_prediction(next: Self, prev: Self, opp: Self) -> Self;
    /// Perform addition as unsigned (matching C++ AddAsUnsigned).
    /// This avoids signed overflow by treating the values as unsigned during addition.
    fn add_as_unsigned(a: Self, b: Self) -> Self;
}

impl ParallelogramDataType for i32 {
    fn compute_parallelogram_prediction(next: Self, prev: Self, opp: Self) -> Self {
        ((next as i64 + prev as i64) - opp as i64) as i32
    }

    fn add_as_unsigned(a: Self, b: Self) -> Self {
        // C++ AddAsUnsigned: cast to unsigned, add, cast back to signed
        // In Rust, this is equivalent to wrapping_add
        (a as u32).wrapping_add(b as u32) as i32
    }
}

// Geometric parallelogram prediction requires 8 parameters: attribute data arrays,
// vertex/corner mappings, face indices, and transform context. This is a mathematical
// computation that genuinely needs all these inputs; grouping them would add indirection.
// Helper function
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_parallelogram_prediction<DataType: ParallelogramDataType>(
    data_entry_id: i32,
    ci: CornerIndex,
    table: &CornerTable,
    vertex_to_data_map: &[i32],
    in_data: &[DataType],
    num_components: usize,
    out_prediction: &mut [DataType],
) -> bool {
    let oci = table.opposite(ci);
    if oci == INVALID_CORNER_INDEX {
        return false;
    }

    // Get entries from the OPPOSITE corner (oci), not the current corner (ci).
    // The opposite corner (oci) is the tip of the neighbor face.
    // The shared vertices are next(oci) and prev(oci).
    // The opposite vertex is vertex(oci).
    let vert_opp_idx = table.vertex(oci).0 as usize;
    let vert_next_idx = table.vertex(table.next(oci)).0 as usize;
    let vert_prev_idx = table.vertex(table.previous(oci)).0 as usize;

    if vert_opp_idx >= vertex_to_data_map.len()
        || vert_next_idx >= vertex_to_data_map.len()
        || vert_prev_idx >= vertex_to_data_map.len()
    {
        return false;
    }

    let vert_opp = vertex_to_data_map[vert_opp_idx];
    let vert_next = vertex_to_data_map[vert_next_idx];
    let vert_prev = vertex_to_data_map[vert_prev_idx];

    // Ensure all three neighbors have been processed already.
    if vert_opp >= 0
        && vert_next >= 0
        && vert_prev >= 0
        && vert_opp < data_entry_id
        && vert_next < data_entry_id
        && vert_prev < data_entry_id
    {
        let v_opp_off = (vert_opp as usize) * num_components;
        let v_next_off = (vert_next as usize) * num_components;
        let v_prev_off = (vert_prev as usize) * num_components;

        for c in 0..num_components {
            // Parallelogram: Prediction = (Next + Prev) - Opp
            out_prediction[c] = DataType::compute_parallelogram_prediction(
                in_data[v_next_off + c],
                in_data[v_prev_off + c],
                in_data[v_opp_off + c],
            );
        }

        #[cfg(feature = "debug_logs")]
        if crate::debug_env_enabled("DRACO_VERBOSE") {
            println!(
                "Parallelogram pred p={} c={} (o={}): entries({}, {}, {})",
                data_entry_id, ci.0, oci.0, vert_opp, vert_next, vert_prev
            );
        }
        return true;
    }
    false
}

#[cfg(feature = "encoder")]
pub struct PredictionSchemeParallelogramEncodingTransform<DataType, CorrType> {
    _marker: PhantomData<(DataType, CorrType)>,
}

#[cfg(feature = "encoder")]
impl<DataType, CorrType> Default
    for PredictionSchemeParallelogramEncodingTransform<DataType, CorrType>
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "encoder")]
impl<DataType, CorrType> PredictionSchemeParallelogramEncodingTransform<DataType, CorrType> {
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "encoder")]
impl<DataType, CorrType> PredictionSchemeEncodingTransform<DataType, CorrType>
    for PredictionSchemeParallelogramEncodingTransform<DataType, CorrType>
where
    DataType: ParallelogramDataType + Into<i64>,
    CorrType: Copy + Default + From<DataType> + std::ops::Sub<Output = CorrType> + From<i32>,
    i64: From<DataType>,
    i32: From<CorrType>,
{
    fn get_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::Parallelogram
    }

    fn init(&mut self, _orig_data: &[DataType], _size: usize, _num_components: usize) {
        // No init needed
    }

    fn compute_correction(
        &self,
        original_vals: &[DataType],
        predicted_vals: &[DataType],
        out_corr_vals: &mut [CorrType],
    ) {
        for i in 0..original_vals.len() {
            // Simple difference for now, assuming CorrType can handle it
            // In C++, this uses IntType for corrections.
            // We need to be careful about types.
            // For now, assume DataType=i32, CorrType=i32
            let o: i64 = original_vals[i].into();
            let p: i64 = predicted_vals[i].into();
            let diff = (o - p) as i32;
            out_corr_vals[i] = diff.into();
        }
    }

    fn encode_transform_data(&mut self, _buffer: &mut Vec<u8>) -> bool {
        true
    }
}

#[cfg(feature = "decoder")]
pub struct PredictionSchemeParallelogramDecodingTransform<DataType, CorrType> {
    _marker: PhantomData<(DataType, CorrType)>,
}

#[cfg(feature = "decoder")]
impl<DataType, CorrType> Default
    for PredictionSchemeParallelogramDecodingTransform<DataType, CorrType>
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "decoder")]
impl<DataType, CorrType> PredictionSchemeParallelogramDecodingTransform<DataType, CorrType> {
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "decoder")]
impl<DataType, CorrType> PredictionSchemeDecodingTransform<DataType, CorrType>
    for PredictionSchemeParallelogramDecodingTransform<DataType, CorrType>
where
    DataType: ParallelogramDataType
        + std::ops::Add<Output = DataType>
        + From<CorrType>
        + From<i32>
        + Into<i64>,
    CorrType: Copy + Default + Into<i32>,
    i64: From<DataType>,
    i32: From<CorrType>,
{
    fn get_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::Parallelogram
    }

    fn init(&mut self, _num_components: usize) {
        // No init needed
    }

    fn compute_original_value(
        &self,
        predicted_vals: &[DataType],
        corr_vals: &[CorrType],
        out_original_vals: &mut [DataType],
    ) {
        for i in 0..predicted_vals.len() {
            let p: i64 = predicted_vals[i].into();
            let c: i32 = corr_vals[i].into();
            let o = p + c as i64;
            out_original_vals[i] = (o as i32).into(); // Assuming i32
        }
    }

    fn decode_transform_data(
        &mut self,
        _buffer: &mut crate::decoder_buffer::DecoderBuffer,
    ) -> bool {
        true
    }
}

#[cfg(feature = "encoder")]
pub struct MeshPredictionSchemeParallelogramEncoder<'a, DataType, CorrType, Transform> {
    #[allow(dead_code)]
    attribute: &'a PointAttribute,
    transform: Transform,
    mesh_data: MeshPredictionSchemeData<'a>,
    _marker: PhantomData<(DataType, CorrType)>,
}

#[cfg(feature = "encoder")]
impl<'a, DataType, CorrType, Transform>
    MeshPredictionSchemeParallelogramEncoder<'a, DataType, CorrType, Transform>
{
    pub fn new(
        attribute: &'a PointAttribute,
        transform: Transform,
        mesh_data: MeshPredictionSchemeData<'a>,
    ) -> Self {
        Self {
            attribute,
            transform,
            mesh_data,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "encoder")]
impl<'a, DataType, CorrType, Transform> PredictionScheme<'a>
    for MeshPredictionSchemeParallelogramEncoder<'a, DataType, CorrType, Transform>
where
    Transform: PredictionSchemeEncodingTransform<DataType, CorrType>,
{
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionParallelogram
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }

    fn is_initialized(&self) -> bool {
        self.mesh_data.corner_table().is_some()
    }

    fn get_num_parent_attributes(&self) -> i32 {
        0
    }

    fn get_parent_attribute_type(&self, _i: i32) -> GeometryAttributeType {
        GeometryAttributeType::Invalid
    }

    fn set_parent_attribute(&mut self, _att: &'a PointAttribute) -> bool {
        false
    }
}

#[cfg(feature = "encoder")]
impl<'a, DataType, CorrType, Transform> PredictionSchemeEncoder<'a, DataType, CorrType>
    for MeshPredictionSchemeParallelogramEncoder<'a, DataType, CorrType, Transform>
where
    DataType: ParallelogramDataType + std::fmt::Debug,
    CorrType: Copy + Default + std::fmt::Debug,
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

        let table = self.mesh_data.corner_table().unwrap();
        let vertex_to_data_map = self.mesh_data.vertex_to_data_map().unwrap();
        let data_to_corner_map = self.mesh_data.data_to_corner_map().unwrap();

        let num_entries = size / num_components;
        let mut pred_vals = vec![DataType::default(); num_components];

        // Process from the end (highest data_id) down to 1.
        // Index 0 is handled separately at the end.
        for p in (1..num_entries).rev() {
            let corner_id = CornerIndex(data_to_corner_map[p]);
            let dst_offset = p * num_components;

            let is_parallelogram = compute_parallelogram_prediction(
                p as i32,
                corner_id,
                table,
                vertex_to_data_map,
                in_data,
                num_components,
                &mut pred_vals,
            );

            if !is_parallelogram {
                // Delta coding: use previous entry as prediction
                let src_offset = (p - 1) * num_components;
                let original = &in_data[dst_offset..dst_offset + num_components];
                let predicted = &in_data[src_offset..src_offset + num_components];
                let corr = &mut out_corr[dst_offset..dst_offset + num_components];
                self.transform.compute_correction(original, predicted, corr);
            } else {
                let original = &in_data[dst_offset..dst_offset + num_components];
                let corr = &mut out_corr[dst_offset..dst_offset + num_components];
                self.transform
                    .compute_correction(original, &pred_vals, corr);
            }
        }

        // First element: no prediction (use zero as prediction)
        for i in 0..num_components {
            pred_vals[i] = DataType::default();
        }
        let original = &in_data[0..num_components];
        let corr = &mut out_corr[0..num_components];
        self.transform
            .compute_correction(original, &pred_vals, corr);

        true
    }

    fn encode_prediction_data(&mut self, buffer: &mut Vec<u8>) -> bool {
        self.transform.encode_transform_data(buffer)
    }
}

#[cfg(feature = "decoder")]
pub struct MeshPredictionSchemeParallelogramDecoder<'a, DataType, CorrType, Transform> {
    #[allow(dead_code)]
    attribute: &'a PointAttribute,
    transform: Transform,
    mesh_data: MeshPredictionSchemeData<'a>,
    _marker: PhantomData<(DataType, CorrType)>,
}

#[cfg(feature = "decoder")]
impl<'a, DataType, CorrType, Transform>
    MeshPredictionSchemeParallelogramDecoder<'a, DataType, CorrType, Transform>
{
    pub fn new(
        attribute: &'a PointAttribute,
        transform: Transform,
        mesh_data: MeshPredictionSchemeData<'a>,
    ) -> Self {
        Self {
            attribute,
            transform,
            mesh_data,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "decoder")]
impl<'a, DataType, CorrType, Transform> PredictionScheme<'a>
    for MeshPredictionSchemeParallelogramDecoder<'a, DataType, CorrType, Transform>
where
    Transform: PredictionSchemeDecodingTransform<DataType, CorrType>,
{
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionParallelogram
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }

    fn is_initialized(&self) -> bool {
        self.mesh_data.corner_table().is_some()
    }

    fn get_num_parent_attributes(&self) -> i32 {
        0
    }

    fn get_parent_attribute_type(&self, _i: i32) -> GeometryAttributeType {
        GeometryAttributeType::Invalid
    }

    fn set_parent_attribute(&mut self, _att: &'a PointAttribute) -> bool {
        false
    }
}

#[cfg(feature = "decoder")]
impl<'a, DataType, CorrType, Transform> PredictionSchemeDecoder<'a, DataType, CorrType>
    for MeshPredictionSchemeParallelogramDecoder<'a, DataType, CorrType, Transform>
where
    DataType: ParallelogramDataType + std::fmt::Debug,
    CorrType: Copy + Default + std::fmt::Debug,
    Transform: PredictionSchemeDecodingTransform<DataType, CorrType>,
{
    fn compute_original_values(
        &mut self,
        in_corr: &[CorrType],
        out_data: &mut [DataType],
        _size: usize,
        num_components: usize,
        _entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> bool {
        self.transform.init(num_components);

        if num_components == 0 {
            return false;
        }

        let Some(table) = self.mesh_data.corner_table() else {
            return false;
        };
        let Some(vertex_to_data_map) = self.mesh_data.vertex_to_data_map() else {
            return false;
        };
        let Some(data_to_corner_map) = self.mesh_data.data_to_corner_map() else {
            return false;
        };
        let Some(required_values) = data_to_corner_map.len().checked_mul(num_components) else {
            return false;
        };
        if required_values == 0
            || in_corr.len() < required_values
            || out_data.len() < required_values
        {
            return false;
        }

        let mut pred_vals = vec![DataType::default(); num_components];

        // Restore the first value.
        let zero_vals = vec![DataType::default(); num_components];
        let corr = &in_corr[0..num_components];
        let out = &mut out_data[0..num_components];
        self.transform.compute_original_value(&zero_vals, corr, out);

        for p in 1..data_to_corner_map.len() {
            let corner_id = CornerIndex(data_to_corner_map[p]);
            let dst_offset = p * num_components;

            let (decoded_data, remaining_data) = out_data.split_at_mut(dst_offset);
            let current_out = &mut remaining_data[0..num_components];

            let is_parallelogram = compute_parallelogram_prediction(
                p as i32,
                corner_id,
                table,
                vertex_to_data_map,
                decoded_data,
                num_components,
                &mut pred_vals,
            );

            if !is_parallelogram {
                // Delta coding
                let src_offset = (p - 1) * num_components;
                let predicted = &decoded_data[src_offset..src_offset + num_components];
                pred_vals.copy_from_slice(predicted);

                let corr = &in_corr[dst_offset..dst_offset + num_components];
                self.transform
                    .compute_original_value(&pred_vals, corr, current_out);
            } else {
                let corr = &in_corr[dst_offset..dst_offset + num_components];
                self.transform
                    .compute_original_value(&pred_vals, corr, current_out);
            }
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
