use crate::corner_table::CornerTable;
use crate::draco_types::DataType;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::{CornerIndex, PointIndex, INVALID_CORNER_INDEX};

#[cfg(feature = "decoder")]
use crate::geometry_indices::INVALID_ATTRIBUTE_VALUE_INDEX;
use crate::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use crate::normal_compression_utils::OctahedronToolBox;
use crate::prediction_scheme::{
    PredictionScheme, PredictionSchemeMethod, PredictionSchemeTransformType,
};

#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "decoder")]
use crate::prediction_scheme::{PredictionSchemeDecoder, PredictionSchemeDecodingTransform};
#[cfg(feature = "decoder")]
use crate::prediction_scheme_normal_octahedron_canonicalized_decoding_transform::PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform;
#[cfg(feature = "decoder")]
use crate::rans_bit_decoder::RAnsBitDecoder;

#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
#[cfg(feature = "encoder")]
use crate::prediction_scheme::{PredictionSchemeEncoder, PredictionSchemeEncodingTransform};
#[cfg(feature = "encoder")]
use crate::rans_bit_encoder::RAnsBitEncoder;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalPredictionMode {
    OneTriangle = 0,
    TriangleArea = 1,
}

// Draco-compatible mesh geometric normal predictor used for normal attributes.
// Draco bitstreams use the NormalOctahedronCanonicalized transform data for this scheme.
#[cfg(feature = "decoder")]
pub struct MeshPredictionSchemeGeometricNormalDecoder<'a> {
    transform: PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform,
    mesh_data: Option<MeshPredictionSchemeData<'a>>,
    pos_attribute: Option<&'a PointAttribute>,
    entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'a>>,
    prediction_mode: NormalPredictionMode,
    octahedron_tool_box: OctahedronToolBox,
    flip_normal_bits: Vec<bool>,
    flip_normal_bit_index: usize,
}

#[cfg(feature = "decoder")]
impl<'a> MeshPredictionSchemeGeometricNormalDecoder<'a> {
    pub fn new(transform: PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform) -> Self {
        Self {
            transform,
            mesh_data: None,
            pos_attribute: None,
            entry_to_point_id_map: None,
            prediction_mode: NormalPredictionMode::TriangleArea,
            octahedron_tool_box: OctahedronToolBox::new(),
            flip_normal_bits: Vec::new(),
            flip_normal_bit_index: 0,
        }
    }

    pub fn set_entry_to_point_id_map(
        &mut self,
        point_ids: crate::prediction_scheme::EntryToPointIdMap<'a>,
    ) {
        self.entry_to_point_id_map = Some(point_ids);
    }

    pub fn init(&mut self, mesh_data: &MeshPredictionSchemeData<'a>) -> bool {
        self.mesh_data = Some(mesh_data.clone());
        true
    }

    fn is_initialized(&self) -> bool {
        self.mesh_data
            .as_ref()
            .and_then(|m| m.corner_table())
            .is_some()
            && self
                .mesh_data
                .as_ref()
                .and_then(|m| m.data_to_corner_map())
                .is_some()
            && self.pos_attribute.is_some()
            && self.entry_to_point_id_map.is_some()
    }

    fn get_position_for_corner(&self, corner_id: CornerIndex) -> [i32; 3] {
        if corner_id == INVALID_CORNER_INDEX {
            return [0, 0, 0];
        }

        let Some(mesh_data) = self.mesh_data.as_ref() else {
            return [0, 0, 0];
        };
        let Some(corner_table) = mesh_data.corner_table() else {
            return [0, 0, 0];
        };
        let Some(vertex_to_data_map) = mesh_data.vertex_to_data_map() else {
            return [0, 0, 0];
        };
        let Some(pos_attribute) = self.pos_attribute else {
            return [0, 0, 0];
        };

        // The corner table used for prediction may be seam-adjusted, which can
        // introduce new vertex ids that don't correspond to original PointIndex.
        // Use vertex_to_data_map + entry_to_point_id_map to resolve to an original
        // point id.
        let v = corner_table.vertex(corner_id);
        let data_id = *vertex_to_data_map.get(v.0 as usize).unwrap_or(&-1);
        if data_id < 0 {
            return [0, 0, 0];
        }
        let data_id = data_id as usize;
        let Some(entry_to_point_id_map) = self.entry_to_point_id_map else {
            return [0, 0, 0];
        };
        let Some(point_id) = entry_to_point_id_map.get(data_id) else {
            return [0, 0, 0];
        };
        let pos_val_id = pos_attribute.mapped_index(PointIndex(point_id));
        if pos_val_id == INVALID_ATTRIBUTE_VALUE_INDEX {
            return [0, 0, 0];
        }

        let mut pos = [0i64; 3];
        if !read_vector3_as_i64(pos_attribute, pos_val_id.0 as usize, &mut pos) {
            return [0, 0, 0];
        }

        let clamp_i32 = |x: i64| -> i32 {
            if x > i32::MAX as i64 {
                i32::MAX
            } else if x < i32::MIN as i64 {
                i32::MIN
            } else {
                x as i32
            }
        };
        [clamp_i32(pos[0]), clamp_i32(pos[1]), clamp_i32(pos[2])]
    }

    fn compute_predicted_value(&self, corner_id: CornerIndex, prediction: &mut [i32; 3]) {
        if corner_id == INVALID_CORNER_INDEX {
            prediction[0] = 0;
            prediction[1] = 0;
            prediction[2] = 0;
            return;
        }

        let Some(mesh_data) = self.mesh_data.as_ref() else {
            prediction[0] = 0;
            prediction[1] = 0;
            prediction[2] = 0;
            return;
        };
        let Some(corner_table) = mesh_data.corner_table() else {
            prediction[0] = 0;
            prediction[1] = 0;
            prediction[2] = 0;
            return;
        };
        let pos_cent = self.get_position_for_corner(corner_id);

        let mut normal = [0i128; 3];

        let mut cit = VertexCornersIterator::new(corner_table, corner_id);
        while !cit.end() {
            let c_next;
            let c_prev;

            if self.prediction_mode == NormalPredictionMode::OneTriangle {
                c_next = corner_table.next(corner_id);
                c_prev = corner_table.previous(corner_id);
            } else {
                c_next = corner_table.next(cit.corner());
                c_prev = corner_table.previous(cit.corner());
            }

            let pos_prev = self.get_position_for_corner(c_prev);
            let pos_next = self.get_position_for_corner(c_next);

            let v_next = [
                pos_next[0] as i64 - pos_cent[0] as i64,
                pos_next[1] as i64 - pos_cent[1] as i64,
                pos_next[2] as i64 - pos_cent[2] as i64,
            ];
            let v_prev = [
                pos_prev[0] as i64 - pos_cent[0] as i64,
                pos_prev[1] as i64 - pos_cent[1] as i64,
                pos_prev[2] as i64 - pos_cent[2] as i64,
            ];

            let cross = [
                v_next[1] as i128 * v_prev[2] as i128 - v_next[2] as i128 * v_prev[1] as i128,
                v_next[2] as i128 * v_prev[0] as i128 - v_next[0] as i128 * v_prev[2] as i128,
                v_next[0] as i128 * v_prev[1] as i128 - v_next[1] as i128 * v_prev[0] as i128,
            ];
            normal[0] += cross[0];
            normal[1] += cross[1];
            normal[2] += cross[2];

            if self.prediction_mode == NormalPredictionMode::OneTriangle {
                break;
            }

            cit.next(corner_table);
        }

        if normal[0] == 0 && normal[1] == 0 && normal[2] == 0 {
            prediction[0] = 0;
            prediction[1] = 0;
            prediction[2] = 0;
            return;
        }

        let upper_bound = 1i128 << 29;
        let abs_sum = normal[0].abs() + normal[1].abs() + normal[2].abs();
        if abs_sum > upper_bound {
            let quotient = abs_sum / upper_bound;
            normal[0] /= quotient;
            normal[1] /= quotient;
            normal[2] /= quotient;
        }

        prediction[0] = normal[0] as i32;
        prediction[1] = normal[1] as i32;
        prediction[2] = normal[2] as i32;
    }
}

#[cfg(feature = "decoder")]
impl<'a> PredictionScheme<'a> for MeshPredictionSchemeGeometricNormalDecoder<'a> {
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionGeometricNormal
    }

    fn is_initialized(&self) -> bool {
        self.is_initialized()
    }

    fn get_num_parent_attributes(&self) -> i32 {
        1
    }

    fn get_parent_attribute_type(&self, i: i32) -> GeometryAttributeType {
        assert_eq!(i, 0);
        GeometryAttributeType::Position
    }

    fn set_parent_attribute(&mut self, att: &'a PointAttribute) -> bool {
        if att.attribute_type() != GeometryAttributeType::Position {
            return false;
        }
        if att.num_components() != 3 {
            return false;
        }
        // Safe: lifetime 'a is now tracked by the compiler
        self.pos_attribute = Some(att);
        true
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::NormalOctahedronCanonicalized
    }
}

#[cfg(feature = "decoder")]
impl<'a> PredictionSchemeDecoder<'a, i32, i32> for MeshPredictionSchemeGeometricNormalDecoder<'a> {
    fn compute_original_values(
        &mut self,
        in_corr: &[i32],
        out_data: &mut [i32],
        _size: usize,
        num_components: usize,
        _entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> bool {
        if !self.is_initialized() {
            return false;
        }
        self.transform.init(num_components);

        let Some(mesh_data) = self.mesh_data.as_ref() else {
            return false;
        };
        let Some(data_to_corner_map) = mesh_data.data_to_corner_map() else {
            return false;
        };
        let corner_map_size = data_to_corner_map.len();
        if corner_map_size * num_components > in_corr.len()
            || corner_map_size * num_components > out_data.len()
        {
            return false;
        }

        let mut pred_normal_3d = [0i32; 3];

        for i in 0..corner_map_size {
            let corner_id = CornerIndex(data_to_corner_map[i]);
            self.compute_predicted_value(corner_id, &mut pred_normal_3d);
            self.octahedron_tool_box
                .canonicalize_integer_vector(&mut pred_normal_3d);

            if self
                .flip_normal_bits
                .get(self.flip_normal_bit_index)
                .copied()
                .unwrap_or(false)
            {
                pred_normal_3d[0] = -pred_normal_3d[0];
                pred_normal_3d[1] = -pred_normal_3d[1];
                pred_normal_3d[2] = -pred_normal_3d[2];
            }
            self.flip_normal_bit_index += 1;

            let (s, t) = self
                .octahedron_tool_box
                .integer_vector_to_quantized_octahedral_coords(&pred_normal_3d);
            let prediction = [s, t];

            let offset = i * num_components;
            self.transform.compute_original_value(
                &prediction,
                &in_corr[offset..offset + num_components],
                &mut out_data[offset..offset + num_components],
            );
        }
        true
    }

    fn decode_prediction_data(&mut self, buffer: &mut DecoderBuffer) -> bool {
        let start_pos = buffer.position();
        let bitstream_version: u16 =
            ((buffer.version_major() as u16) << 8) | (buffer.version_minor() as u16);
        if bitstream_version < 0x0202 && !cfg!(feature = "legacy_bitstream_decode") {
            return false;
        }

        let try_decode_at_pos = |this: &mut Self, buf: &mut DecoderBuffer| -> bool {
            if !this.transform.decode_transform_data(buf) {
                return false;
            }

            // Set up octahedral toolbox from decoded transform.
            this.octahedron_tool_box
                .set_quantization_bits(this.transform.quantization_bits());

            // Backward compatibility: bitstreams < 2.2 store prediction mode.
            if bitstream_version < 0x0202 {
                let mode = match buf.decode_u8() {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                if mode > NormalPredictionMode::TriangleArea as u8 {
                    return false;
                }
                this.prediction_mode = if mode == 0 {
                    NormalPredictionMode::OneTriangle
                } else {
                    NormalPredictionMode::TriangleArea
                };
            }

            let num_values = match this.mesh_data.as_ref().and_then(|m| m.data_to_corner_map()) {
                Some(map) => map.len(),
                None => return false,
            };

            this.flip_normal_bits.clear();
            this.flip_normal_bits.reserve(num_values);

            let mut decoder = RAnsBitDecoder::new();
            if !decoder.start_decoding(buf) {
                return false;
            }

            for _ in 0..num_values {
                this.flip_normal_bits.push(decoder.decode_next_bit());
            }
            decoder.end_decoding();
            this.flip_normal_bit_index = 0;
            true
        };

        if try_decode_at_pos(self, buffer) {
            return true;
        }

        let _ = buffer.set_position(start_pos);
        false
    }
}

struct VertexCornersIterator {
    _start_corner: CornerIndex,
    corner: CornerIndex,
    left_corner: CornerIndex,
    is_end: bool,
}

impl VertexCornersIterator {
    fn new(corner_table: &CornerTable, corner_id: CornerIndex) -> Self {
        if corner_id == INVALID_CORNER_INDEX {
            return Self {
                _start_corner: INVALID_CORNER_INDEX,
                corner: INVALID_CORNER_INDEX,
                left_corner: INVALID_CORNER_INDEX,
                is_end: true,
            };
        }

        let mut start_corner = corner_id;
        let mut corner = corner_id;
        let mut left_corner = corner_id;

        let mut c = corner_table.swing_left(corner_id);
        while c != INVALID_CORNER_INDEX {
            corner = c;
            left_corner = c;
            if c == start_corner {
                break;
            }
            c = corner_table.swing_left(c);
        }
        start_corner = corner;

        Self {
            _start_corner: start_corner,
            corner,
            left_corner,
            is_end: false,
        }
    }

    fn corner(&self) -> CornerIndex {
        self.corner
    }

    fn end(&self) -> bool {
        self.is_end || self.corner == INVALID_CORNER_INDEX
    }

    fn next(&mut self, corner_table: &CornerTable) {
        if self.corner == INVALID_CORNER_INDEX {
            return;
        }
        self.corner = corner_table.swing_right(self.corner);
        if self.corner == self.left_corner {
            self.corner = INVALID_CORNER_INDEX;
            self.is_end = true;
        } else if self.corner == INVALID_CORNER_INDEX {
            self.is_end = true;
        }
    }
}

#[cfg(feature = "encoder")]
pub struct PredictionSchemeGeometricNormalEncodingTransform {
    octahedron_tool_box: OctahedronToolBox,
}

#[cfg(feature = "encoder")]
impl Default for PredictionSchemeGeometricNormalEncodingTransform {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "encoder")]
impl PredictionSchemeGeometricNormalEncodingTransform {
    pub fn new() -> Self {
        Self {
            octahedron_tool_box: OctahedronToolBox::new(),
        }
    }

    pub fn set_quantization_bits(&mut self, q: i32) {
        self.octahedron_tool_box.set_quantization_bits(q);
    }

    pub fn quantization_bits(&self) -> i32 {
        self.octahedron_tool_box.quantization_bits()
    }
}

#[cfg(feature = "encoder")]
impl PredictionSchemeEncodingTransform<i32, i32>
    for PredictionSchemeGeometricNormalEncodingTransform
{
    fn init(&mut self, _orig_data: &[i32], _size: usize, _num_components: usize) {}

    fn compute_correction(
        &self,
        original_vals: &[i32],
        predicted_vals: &[i32],
        out_corr_vals: &mut [i32],
    ) {
        // original_vals are in octahedral coords (s, t)
        // predicted_vals are in octahedral coords (s, t)

        // Correction is (original - predicted) mod max
        // We compute simple difference here, and let the encoder handle ModMax logic if needed.
        // But wait, the encoder logic in C++ does:
        // this->transform().ComputeCorrection(in_data + data_offset, pos_pred_normal_oct.data(), pos_correction.data());
        // And then ModMax.
        // So ComputeCorrection should just be subtraction.

        out_corr_vals[0] = original_vals[0] - predicted_vals[0];
        out_corr_vals[1] = original_vals[1] - predicted_vals[1];
    }

    fn encode_transform_data(&mut self, buffer: &mut Vec<u8>) -> bool {
        buffer.push(self.octahedron_tool_box.quantization_bits() as u8);
        true
    }

    fn get_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::GeometricNormal
    }
}

#[cfg(feature = "encoder")]
pub struct MeshPredictionSchemeGeometricNormalEncoder<'a> {
    transform: PredictionSchemeGeometricNormalEncodingTransform,
    mesh_data: Option<MeshPredictionSchemeData<'a>>,
    pos_attribute: Option<&'a PointAttribute>,
    prediction_mode: NormalPredictionMode,
    flip_normal_bit_encoder: RAnsBitEncoder,
}

#[cfg(feature = "encoder")]
impl<'a> MeshPredictionSchemeGeometricNormalEncoder<'a> {
    pub fn new(transform: PredictionSchemeGeometricNormalEncodingTransform) -> Self {
        Self {
            transform,
            mesh_data: None,
            pos_attribute: None,
            prediction_mode: NormalPredictionMode::TriangleArea,
            flip_normal_bit_encoder: RAnsBitEncoder::new(),
        }
    }

    pub fn init(&mut self, mesh_data: &MeshPredictionSchemeData<'a>) -> bool {
        self.mesh_data = Some(mesh_data.clone());
        true
    }

    fn compute_predicted_value(
        &self,
        corner_id: CornerIndex,
        prediction: &mut [i32; 3],
        map: crate::prediction_scheme::EntryToPointIdMap<'_>,
    ) {
        // Duplicate logic from decoder for now.
        // Ideally we should share this.
        if corner_id == INVALID_CORNER_INDEX {
            prediction[0] = 0;
            prediction[1] = 0;
            prediction[2] = 0;
            return;
        }

        let mesh_data = self.mesh_data.as_ref().unwrap();
        let corner_table = mesh_data.corner_table().unwrap();

        let mut cit = VertexCornersIterator::new(corner_table, corner_id);

        let pos_cent = self.get_position_for_corner_with_map(corner_id, map);

        let mut normal = [0i64; 3];

        while !cit.end() {
            let c_next;
            let c_prev;

            if self.prediction_mode == NormalPredictionMode::OneTriangle {
                c_next = corner_table.next(corner_id);
                c_prev = corner_table.previous(corner_id);
            } else {
                c_next = corner_table.next(cit.corner());
                c_prev = corner_table.previous(cit.corner());
            }

            let pos_next = self.get_position_for_corner_with_map(c_next, map);
            let pos_prev = self.get_position_for_corner_with_map(c_prev, map);

            let delta_next = [
                pos_next[0] - pos_cent[0],
                pos_next[1] - pos_cent[1],
                pos_next[2] - pos_cent[2],
            ];
            let delta_prev = [
                pos_prev[0] - pos_cent[0],
                pos_prev[1] - pos_cent[1],
                pos_prev[2] - pos_cent[2],
            ];

            let cross = cross_product(&delta_next, &delta_prev);

            normal[0] += cross[0];
            normal[1] += cross[1];
            normal[2] += cross[2];

            cit.next(corner_table);

            if self.prediction_mode == NormalPredictionMode::OneTriangle {
                break;
            }
        }

        let upper_bound = 1 << 29;
        let abs_sum = normal[0].abs() + normal[1].abs() + normal[2].abs();

        if abs_sum > upper_bound {
            let quotient = abs_sum / upper_bound;
            if quotient > 0 {
                normal[0] /= quotient;
                normal[1] /= quotient;
                normal[2] /= quotient;
            }
        }

        prediction[0] = normal[0] as i32;
        prediction[1] = normal[1] as i32;
        prediction[2] = normal[2] as i32;
    }

    fn get_position_for_corner_with_map(
        &self,
        ci: CornerIndex,
        map: crate::prediction_scheme::EntryToPointIdMap<'_>,
    ) -> [i64; 3] {
        let mesh_data = self.mesh_data.as_ref().unwrap();
        let corner_table = mesh_data.corner_table().unwrap();
        let vertex_to_data_map = mesh_data.vertex_to_data_map().unwrap();

        let vert_id = corner_table.vertex(ci);
        let data_id = vertex_to_data_map[vert_id.0 as usize];

        let Some(point_id) = map.get(data_id as usize) else {
            return [0, 0, 0];
        };
        let pos_att = self.pos_attribute.unwrap();
        let pos_val_id = pos_att.mapped_index(PointIndex(point_id));

        let mut pos = [0i64; 3];
        if !read_vector3_as_i64(pos_att, pos_val_id.0 as usize, &mut pos) {
            return [0, 0, 0];
        }
        pos
    }
}

#[cfg(feature = "encoder")]
impl<'a> PredictionScheme<'a> for MeshPredictionSchemeGeometricNormalEncoder<'a> {
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionGeometricNormal
    }

    fn is_initialized(&self) -> bool {
        self.mesh_data.is_some() && self.pos_attribute.is_some()
    }

    fn get_num_parent_attributes(&self) -> i32 {
        1
    }

    fn get_parent_attribute_type(&self, i: i32) -> GeometryAttributeType {
        if i == 0 {
            GeometryAttributeType::Position
        } else {
            GeometryAttributeType::Invalid
        }
    }

    fn set_parent_attribute(&mut self, att: &'a PointAttribute) -> bool {
        if att.attribute_type() != GeometryAttributeType::Position {
            return false;
        }
        // Safe: lifetime 'a is now tracked by the compiler
        self.pos_attribute = Some(att);
        true
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }
}

#[cfg(feature = "encoder")]
impl<'a> PredictionSchemeEncoder<'a, i32, i32> for MeshPredictionSchemeGeometricNormalEncoder<'a> {
    fn compute_correction_values(
        &mut self,
        in_data: &[i32],
        out_corr: &mut [i32],
        size: usize,
        num_components: usize,
        entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> bool {
        if !self.is_initialized() {
            return false;
        }

        let map = match entry_to_point_id_map {
            Some(m) => m,
            None => return false,
        };

        // Expecting in_data in octahedral coordinates (portable attribute)
        if num_components != 2 {
            return false;
        }

        self.flip_normal_bit_encoder.start_encoding();

        let mesh_data = self.mesh_data.as_ref().unwrap();
        let data_to_corner_map = mesh_data.data_to_corner_map().unwrap();

        let mut pred_normal_3d = [0i32; 3];
        let mut pos_pred_normal_oct = [0i32; 2];
        let mut neg_pred_normal_oct = [0i32; 2];
        let mut pos_correction = [0i32; 2];
        let mut neg_correction = [0i32; 2];

        for i in 0..size {
            let corner_id = CornerIndex(data_to_corner_map[i]);

            self.compute_predicted_value(corner_id, &mut pred_normal_3d, map);

            self.transform
                .octahedron_tool_box
                .canonicalize_integer_vector(&mut pred_normal_3d);

            // Compute octahedral coordinates for both possible directions
            let (s_pos, t_pos) = self
                .transform
                .octahedron_tool_box
                .integer_vector_to_quantized_octahedral_coords(&pred_normal_3d);
            pos_pred_normal_oct[0] = s_pos;
            pos_pred_normal_oct[1] = t_pos;

            let neg_normal_3d = [-pred_normal_3d[0], -pred_normal_3d[1], -pred_normal_3d[2]];
            let (s_neg, t_neg) = self
                .transform
                .octahedron_tool_box
                .integer_vector_to_quantized_octahedral_coords(&neg_normal_3d);
            neg_pred_normal_oct[0] = s_neg;
            neg_pred_normal_oct[1] = t_neg;

            let offset = i * num_components;
            let in_val = &in_data[offset..offset + num_components];

            self.transform
                .compute_correction(in_val, &pos_pred_normal_oct, &mut pos_correction);
            self.transform
                .compute_correction(in_val, &neg_pred_normal_oct, &mut neg_correction);

            pos_correction[0] = self
                .transform
                .octahedron_tool_box
                .mod_max_positive(pos_correction[0]);
            pos_correction[1] = self
                .transform
                .octahedron_tool_box
                .mod_max_positive(pos_correction[1]);
            neg_correction[0] = self
                .transform
                .octahedron_tool_box
                .mod_max_positive(neg_correction[0]);
            neg_correction[1] = self
                .transform
                .octahedron_tool_box
                .mod_max_positive(neg_correction[1]);

            let pos_abs_sum = pos_correction[0].abs() + pos_correction[1].abs();
            let neg_abs_sum = neg_correction[0].abs() + neg_correction[1].abs();

            if pos_abs_sum < neg_abs_sum {
                self.flip_normal_bit_encoder.encode_bit(false);
                out_corr[offset] = self
                    .transform
                    .octahedron_tool_box
                    .make_positive(pos_correction[0]);
                out_corr[offset + 1] = self
                    .transform
                    .octahedron_tool_box
                    .make_positive(pos_correction[1]);
            } else {
                self.flip_normal_bit_encoder.encode_bit(true);
                out_corr[offset] = self
                    .transform
                    .octahedron_tool_box
                    .make_positive(neg_correction[0]);
                out_corr[offset + 1] = self
                    .transform
                    .octahedron_tool_box
                    .make_positive(neg_correction[1]);
            }
        }
        true
    }

    fn encode_prediction_data(&mut self, buffer: &mut Vec<u8>) -> bool {
        if !self.transform.encode_transform_data(buffer) {
            return false;
        }

        let mut temp_buffer = EncoderBuffer::new();
        self.flip_normal_bit_encoder.end_encoding(&mut temp_buffer);
        buffer.extend_from_slice(temp_buffer.data());
        true
    }
}

#[cfg(feature = "encoder")]
fn cross_product(a: &[i64; 3], b: &[i64; 3]) -> [i64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn read_vector3_as_i64(att: &PointAttribute, index: usize, out: &mut [i64; 3]) -> bool {
    for c in 0..3 {
        let Some(value) = read_component_as_i64(att, index, c) else {
            return false;
        };
        out[c] = value;
    }
    true
}

fn read_component_as_i64(att: &PointAttribute, index: usize, component: usize) -> Option<i64> {
    let buffer = att.buffer();
    let byte_stride = usize::try_from(att.byte_stride()).ok()?;
    let byte_offset = index
        .checked_mul(byte_stride)?
        .checked_add(component.checked_mul(att.data_type().byte_length())?)?;

    let read_i8 = |offset| -> Option<i8> {
        let mut b = [0u8; 1];
        buffer
            .try_read(offset, &mut b)
            .then(|| i8::from_le_bytes(b))
    };
    let read_u8 = |offset| -> Option<u8> {
        let mut b = [0u8; 1];
        buffer
            .try_read(offset, &mut b)
            .then(|| u8::from_le_bytes(b))
    };
    let read_i16 = |offset| -> Option<i16> {
        let mut b = [0u8; 2];
        buffer
            .try_read(offset, &mut b)
            .then(|| i16::from_le_bytes(b))
    };
    let read_u16 = |offset| -> Option<u16> {
        let mut b = [0u8; 2];
        buffer
            .try_read(offset, &mut b)
            .then(|| u16::from_le_bytes(b))
    };
    let read_i32 = |offset| -> Option<i32> {
        let mut b = [0u8; 4];
        buffer
            .try_read(offset, &mut b)
            .then(|| i32::from_le_bytes(b))
    };
    let read_u32 = |offset| -> Option<u32> {
        let mut b = [0u8; 4];
        buffer
            .try_read(offset, &mut b)
            .then(|| u32::from_le_bytes(b))
    };
    let read_i64 = |offset| -> Option<i64> {
        let mut b = [0u8; 8];
        buffer
            .try_read(offset, &mut b)
            .then(|| i64::from_le_bytes(b))
    };
    let read_u64 = |offset| -> Option<u64> {
        let mut b = [0u8; 8];
        buffer
            .try_read(offset, &mut b)
            .then(|| u64::from_le_bytes(b))
    };
    let read_f32 = |offset| -> Option<f32> {
        let mut b = [0u8; 4];
        buffer
            .try_read(offset, &mut b)
            .then(|| f32::from_le_bytes(b))
    };
    let read_f64 = |offset| -> Option<f64> {
        let mut b = [0u8; 8];
        buffer
            .try_read(offset, &mut b)
            .then(|| f64::from_le_bytes(b))
    };

    match att.data_type() {
        DataType::Int8 => read_i8(byte_offset).map(|v| v as i64),
        DataType::Uint8 => read_u8(byte_offset).map(|v| v as i64),
        DataType::Int16 => read_i16(byte_offset).map(|v| v as i64),
        DataType::Uint16 => read_u16(byte_offset).map(|v| v as i64),
        DataType::Int32 => read_i32(byte_offset).map(|v| v as i64),
        DataType::Uint32 => read_u32(byte_offset).map(|v| v as i64),
        DataType::Int64 => read_i64(byte_offset),
        DataType::Uint64 => read_u64(byte_offset).map(|v| v as i64),
        DataType::Float32 => read_f32(byte_offset).map(|v| v as i64),
        DataType::Float64 => read_f64(byte_offset).map(|v| v as i64),
        DataType::Bool => read_u8(byte_offset).map(|v| v as i64),
        _ => Some(0),
    }
}

#[cfg(all(test, feature = "decoder"))]
mod tests {
    use super::*;
    use crate::corner_table::CornerTable;
    use crate::geometry_attribute::GeometryAttributeType;
    use crate::geometry_indices::{FaceIndex, PointIndex};
    use crate::prediction_scheme::EntryToPointIdMap;

    #[test]
    fn mesh_geometric_normal_position_lookup_returns_zero_when_entry_map_is_too_short() {
        let mut corner_table = CornerTable::new(1);
        corner_table.set_face_vertices(FaceIndex(0), PointIndex(0), PointIndex(1), PointIndex(2));

        let data_to_corner_map = [0u32];
        let vertex_to_data_map = [1, 0, 0];
        let mut mesh_data = MeshPredictionSchemeData::new();
        mesh_data.set(&corner_table, &data_to_corner_map, &vertex_to_data_map);

        let mut position_attribute = PointAttribute::new();
        position_attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Int32,
            false,
            1,
        );

        let mut decoder = MeshPredictionSchemeGeometricNormalDecoder::new(
            PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform::new(),
        );
        assert!(decoder.init(&mesh_data));
        assert!(decoder.set_parent_attribute(&position_attribute));

        let entry_to_point_id_map = [0u32];
        decoder
            .set_entry_to_point_id_map(EntryToPointIdMap::from_u32_slice(&entry_to_point_id_map));

        assert_eq!(decoder.get_position_for_corner(CornerIndex(0)), [0, 0, 0]);
    }

    #[test]
    fn mesh_geometric_normal_position_lookup_returns_zero_for_truncated_buffer() {
        let mut corner_table = CornerTable::new(1);
        corner_table.set_face_vertices(FaceIndex(0), PointIndex(0), PointIndex(1), PointIndex(2));

        let data_to_corner_map = [0u32];
        let vertex_to_data_map = [0, 0, 0];
        let mut mesh_data = MeshPredictionSchemeData::new();
        mesh_data.set(&corner_table, &data_to_corner_map, &vertex_to_data_map);

        let mut position_attribute = PointAttribute::new();
        position_attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Int32,
            false,
            1,
        );
        position_attribute.buffer_mut().resize(8);

        let mut decoder = MeshPredictionSchemeGeometricNormalDecoder::new(
            PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform::new(),
        );
        assert!(decoder.init(&mesh_data));
        assert!(decoder.set_parent_attribute(&position_attribute));

        let entry_to_point_id_map = [0u32];
        decoder
            .set_entry_to_point_id_map(EntryToPointIdMap::from_u32_slice(&entry_to_point_id_map));

        assert_eq!(decoder.get_position_for_corner(CornerIndex(0)), [0, 0, 0]);
    }
}
