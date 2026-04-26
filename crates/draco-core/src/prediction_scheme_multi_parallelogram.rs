use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::{CornerIndex, INVALID_CORNER_INDEX};
use crate::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use crate::prediction_scheme::{
    PredictionScheme, PredictionSchemeMethod, PredictionSchemeTransformType,
};
use crate::prediction_scheme_parallelogram::{
    compute_parallelogram_prediction, ParallelogramDataType,
};
use std::marker::PhantomData;

#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "decoder")]
use crate::prediction_scheme::{PredictionSchemeDecoder, PredictionSchemeDecodingTransform};

#[cfg(feature = "decoder")]
pub struct MeshPredictionSchemeMultiParallelogramDecoder<'a, DataType, CorrType, Transform> {
    transform: Transform,
    mesh_data: MeshPredictionSchemeData<'a>,
    _marker: PhantomData<(DataType, CorrType)>,
}

#[cfg(feature = "decoder")]
impl<'a, DataType, CorrType, Transform>
    MeshPredictionSchemeMultiParallelogramDecoder<'a, DataType, CorrType, Transform>
{
    pub fn new(transform: Transform, mesh_data: MeshPredictionSchemeData<'a>) -> Self {
        Self {
            transform,
            mesh_data,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "decoder")]
impl<'a, DataType, CorrType, Transform> PredictionScheme<'a>
    for MeshPredictionSchemeMultiParallelogramDecoder<'a, DataType, CorrType, Transform>
where
    Transform: PredictionSchemeDecodingTransform<DataType, CorrType>,
{
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionMultiParallelogram
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

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }
}

#[cfg(feature = "decoder")]
impl<'a, DataType, CorrType, Transform> PredictionSchemeDecoder<'a, DataType, CorrType>
    for MeshPredictionSchemeMultiParallelogramDecoder<'a, DataType, CorrType, Transform>
where
    DataType: ParallelogramDataType + Copy + Default + From<i32> + Into<i64> + std::fmt::Debug,
    CorrType: Copy + Default + std::fmt::Debug,
    Transform: PredictionSchemeDecodingTransform<DataType, CorrType>,
{
    fn decode_prediction_data(&mut self, buffer: &mut DecoderBuffer) -> bool {
        self.transform.decode_transform_data(buffer)
    }

    fn compute_original_values(
        &mut self,
        in_corr: &[CorrType],
        out_data: &mut [DataType],
        _size: usize,
        num_components: usize,
        _entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> bool {
        if num_components == 0 {
            return false;
        }

        let table = match self.mesh_data.corner_table() {
            Some(table) => table,
            None => return false,
        };
        let vertex_to_data_map = match self.mesh_data.vertex_to_data_map() {
            Some(map) => map,
            None => return false,
        };
        let data_to_corner_map = match self.mesh_data.data_to_corner_map() {
            Some(map) => map,
            None => return false,
        };
        let required_values = match data_to_corner_map.len().checked_mul(num_components) {
            Some(v) => v,
            None => return false,
        };
        if in_corr.len() < required_values || out_data.len() < required_values {
            return false;
        }

        self.transform.init(num_components);

        let mut pred_vals = vec![DataType::default(); num_components];
        let mut parallelogram_pred_vals = vec![DataType::default(); num_components];

        self.transform.compute_original_value(
            &pred_vals,
            &in_corr[0..num_components],
            &mut out_data[0..num_components],
        );

        for p in 1..data_to_corner_map.len() {
            let start_corner_id = CornerIndex(data_to_corner_map[p]);
            if start_corner_id == INVALID_CORNER_INDEX {
                let src_offset = (p - 1) * num_components;
                let dst_offset = p * num_components;
                pred_vals.copy_from_slice(&out_data[src_offset..src_offset + num_components]);
                self.transform.compute_original_value(
                    &pred_vals,
                    &in_corr[dst_offset..dst_offset + num_components],
                    &mut out_data[dst_offset..dst_offset + num_components],
                );
                continue;
            }

            pred_vals.fill(DataType::default());
            let mut num_parallelograms = 0usize;
            let mut corner_id = start_corner_id;

            while corner_id != INVALID_CORNER_INDEX {
                if compute_parallelogram_prediction(
                    p as i32,
                    corner_id,
                    table,
                    vertex_to_data_map,
                    out_data,
                    num_components,
                    &mut parallelogram_pred_vals,
                ) {
                    for c in 0..num_components {
                        pred_vals[c] =
                            DataType::add_as_unsigned(pred_vals[c], parallelogram_pred_vals[c]);
                    }
                    num_parallelograms += 1;
                }

                corner_id = table.swing_right(corner_id);
                if corner_id == start_corner_id {
                    corner_id = INVALID_CORNER_INDEX;
                }
            }

            let dst_offset = p * num_components;
            if num_parallelograms == 0 {
                let src_offset = (p - 1) * num_components;
                pred_vals.copy_from_slice(&out_data[src_offset..src_offset + num_components]);
            } else {
                for value in &mut pred_vals {
                    *value = DataType::from(((*value).into() / num_parallelograms as i64) as i32);
                }
            }

            self.transform.compute_original_value(
                &pred_vals,
                &in_corr[dst_offset..dst_offset + num_components],
                &mut out_data[dst_offset..dst_offset + num_components],
            );
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corner_table::CornerTable;
    use crate::geometry_indices::VertexIndex;
    use crate::prediction_scheme::PredictionSchemeDecoder;
    use crate::prediction_scheme::{
        PredictionSchemeDecodingTransform, PredictionSchemeTransformType,
    };

    struct IdentityTransform;

    impl PredictionSchemeDecodingTransform<i32, i32> for IdentityTransform {
        fn init(&mut self, _num_components: usize) {}

        fn compute_original_value(
            &self,
            predicted_vals: &[i32],
            corr_vals: &[i32],
            out_original_vals: &mut [i32],
        ) {
            for i in 0..out_original_vals.len() {
                out_original_vals[i] = predicted_vals[i] + corr_vals[i];
            }
        }

        fn decode_transform_data(&mut self, _buffer: &mut DecoderBuffer) -> bool {
            true
        }

        fn get_type(&self) -> PredictionSchemeTransformType {
            PredictionSchemeTransformType::Delta
        }
    }

    #[test]
    #[cfg(feature = "decoder")]
    fn multi_parallelogram_decodes_with_fallback() {
        let mut table = CornerTable::new(1);
        assert!(table.init(&[[VertexIndex(0), VertexIndex(1), VertexIndex(2)]]));
        let data_to_corner_map = [0, 1, 2];
        let vertex_to_data_map = [0, 1, 2];
        let mut mesh_data = MeshPredictionSchemeData::new();
        mesh_data.set(&table, &data_to_corner_map, &vertex_to_data_map);

        let mut decoder = MeshPredictionSchemeMultiParallelogramDecoder::<
            i32,
            i32,
            IdentityTransform,
        >::new(IdentityTransform, mesh_data);

        let in_corr = [10, 2, 3];
        let mut out = [0; 3];
        assert!(decoder.compute_original_values(&in_corr, &mut out, 3, 1, None));
        assert_eq!(out, [10, 12, 15]);
    }

    #[test]
    #[cfg(feature = "decoder")]
    fn multi_parallelogram_averages_multiple_valid_predictions() {
        let mut table = CornerTable::new(4);
        for (corner, vertex) in [
            (0, 3),
            (1, 4),
            (2, 5),
            (3, 3),
            (4, 6),
            (5, 7),
            (6, 0),
            (7, 1),
            (8, 2),
            (9, 1),
            (10, 2),
            (11, 0),
        ] {
            table.map_corner_to_vertex(CornerIndex(corner), VertexIndex(vertex));
        }
        table
            .vertex_corners
            .resize(8, crate::geometry_indices::INVALID_CORNER_INDEX);
        table.set_opposite(CornerIndex(0), CornerIndex(6));
        table.set_opposite(CornerIndex(3), CornerIndex(9));
        table.set_opposite(CornerIndex(2), CornerIndex(4));
        table.set_opposite(CornerIndex(5), CornerIndex(1));

        let data_to_corner_map = [6, 7, 8, 0];
        let vertex_to_data_map = [0, 1, 2, 3, -1, -1, -1, -1];
        let mut mesh_data = MeshPredictionSchemeData::new();
        mesh_data.set(&table, &data_to_corner_map, &vertex_to_data_map);

        let mut decoder = MeshPredictionSchemeMultiParallelogramDecoder::<
            i32,
            i32,
            IdentityTransform,
        >::new(IdentityTransform, mesh_data);

        let in_corr = [10, 20, 20, 5];
        let mut out = [0; 4];
        assert!(decoder.compute_original_values(&in_corr, &mut out, 4, 1, None));
        assert_eq!(out, [10, 30, 50, 55]);
    }
}
