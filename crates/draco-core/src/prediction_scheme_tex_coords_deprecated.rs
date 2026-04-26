use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::{CornerIndex, PointIndex, INVALID_ATTRIBUTE_VALUE_INDEX};
use crate::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use crate::prediction_scheme::{
    PredictionScheme, PredictionSchemeDecoder, PredictionSchemeDecodingTransform,
    PredictionSchemeMethod, PredictionSchemeTransformType,
};
use crate::{
    decoder_buffer::DecoderBuffer, draco_types::DataType, rans_bit_decoder::RAnsBitDecoder,
};

pub struct MeshPredictionSchemeTexCoordsDeprecatedDecoder<'a, Transform> {
    transform: Transform,
    mesh_data: Option<MeshPredictionSchemeData<'a>>,
    orientations: Vec<bool>,
    pos_attribute: Option<&'a PointAttribute>,
}

impl<'a, Transform> MeshPredictionSchemeTexCoordsDeprecatedDecoder<'a, Transform> {
    pub fn new(transform: Transform) -> Self {
        Self {
            transform,
            mesh_data: None,
            orientations: Vec::new(),
            pos_attribute: None,
        }
    }

    pub fn init(&mut self, mesh_data: &MeshPredictionSchemeData<'a>) -> bool {
        self.mesh_data = Some(mesh_data.clone());
        true
    }

    fn get_position_for_entry_id(
        &self,
        entry_id: i32,
        entry_to_point_id_map: crate::prediction_scheme::EntryToPointIdMap<'_>,
    ) -> Option<[f32; 3]> {
        let point_id = entry_to_point_id_map.get(usize::try_from(entry_id).ok()?)?;
        let att = self.pos_attribute?;
        let val_index = att.mapped_index(PointIndex(point_id));
        if val_index == INVALID_ATTRIBUTE_VALUE_INDEX {
            return None;
        }

        let mut pos = [0.0f32; 3];
        for (component, out) in pos.iter_mut().enumerate() {
            *out = read_component_as_f32(att, val_index.0 as usize, component)?;
        }
        Some(pos)
    }

    fn get_tex_coord_for_entry_id(&self, entry_id: i32, data: &[i32]) -> Option<[f32; 2]> {
        let offset = usize::try_from(entry_id).ok()?.checked_mul(2)?;
        Some([*data.get(offset)? as f32, *data.get(offset + 1)? as f32])
    }

    fn compute_predicted_value(
        &mut self,
        corner_id: CornerIndex,
        data: &[i32],
        data_id: i32,
        entry_to_point_id_map: crate::prediction_scheme::EntryToPointIdMap<'_>,
        predicted_value: &mut [i32; 2],
    ) -> bool {
        let mesh_data = self.mesh_data.as_ref().unwrap();
        let corner_table = mesh_data.corner_table().unwrap();
        let vertex_to_data_map = mesh_data.vertex_to_data_map().unwrap();

        let next_corner_id = corner_table.next(corner_id);
        let prev_corner_id = corner_table.previous(corner_id);

        let next_vert_id = corner_table.vertex(next_corner_id).0 as usize;
        let prev_vert_id = corner_table.vertex(prev_corner_id).0 as usize;

        let Some(&next_data_id) = vertex_to_data_map.get(next_vert_id) else {
            return false;
        };
        let Some(&prev_data_id) = vertex_to_data_map.get(prev_vert_id) else {
            return false;
        };

        if prev_data_id < data_id && next_data_id < data_id {
            let Some(n_uv) = self.get_tex_coord_for_entry_id(next_data_id, data) else {
                return false;
            };
            let Some(p_uv) = self.get_tex_coord_for_entry_id(prev_data_id, data) else {
                return false;
            };

            if n_uv == p_uv {
                predicted_value[0] = f32_to_i32_deprecated(p_uv[0], false);
                predicted_value[1] = f32_to_i32_deprecated(p_uv[1], false);
                return true;
            }

            let Some(tip_pos) = self.get_position_for_entry_id(data_id, entry_to_point_id_map)
            else {
                return false;
            };
            let Some(next_pos) =
                self.get_position_for_entry_id(next_data_id, entry_to_point_id_map)
            else {
                return false;
            };
            let Some(prev_pos) =
                self.get_position_for_entry_id(prev_data_id, entry_to_point_id_map)
            else {
                return false;
            };

            let pn = vec3_sub(prev_pos, next_pos);
            let cn = vec3_sub(tip_pos, next_pos);
            let pn_norm2_squared = vec3_squared_norm(pn);
            let (s, t) = if pn_norm2_squared > 0.0 {
                let s = vec3_dot(pn, cn) / pn_norm2_squared;
                let t = (vec3_squared_norm(vec3_sub(cn, vec3_mul_scalar(pn, s)))
                    / pn_norm2_squared)
                    .sqrt();
                (s, t)
            } else {
                (0.0, 0.0)
            };

            let pn_uv = vec2_sub(p_uv, n_uv);
            let pnus = pn_uv[0] * s + n_uv[0];
            let pnut = pn_uv[0] * t;
            let pnvs = pn_uv[1] * s + n_uv[1];
            let pnvt = pn_uv[1] * t;

            let Some(orientation) = self.orientations.pop() else {
                return false;
            };
            let predicted_uv = if orientation {
                [pnus - pnvt, pnvs + pnut]
            } else {
                [pnus + pnvt, pnvs - pnut]
            };

            predicted_value[0] = f32_to_i32_deprecated(predicted_uv[0], true);
            predicted_value[1] = f32_to_i32_deprecated(predicted_uv[1], true);
            return true;
        }

        let data_offset = if prev_data_id < data_id {
            let mut offset = prev_data_id;
            if next_data_id < data_id {
                offset = next_data_id;
            } else if data_id > 0 {
                offset = data_id - 1;
            }
            usize::try_from(offset).ok().and_then(|v| v.checked_mul(2))
        } else if next_data_id < data_id {
            usize::try_from(next_data_id)
                .ok()
                .and_then(|v| v.checked_mul(2))
        } else if data_id > 0 {
            usize::try_from(data_id - 1)
                .ok()
                .and_then(|v| v.checked_mul(2))
        } else {
            predicted_value[0] = 0;
            predicted_value[1] = 0;
            return true;
        };

        let Some(data_offset) = data_offset else {
            return false;
        };
        let Some(&u) = data.get(data_offset) else {
            return false;
        };
        let Some(&v) = data.get(data_offset + 1) else {
            return false;
        };
        predicted_value[0] = u;
        predicted_value[1] = v;
        true
    }
}

impl<'a, Transform> PredictionScheme<'a>
    for MeshPredictionSchemeTexCoordsDeprecatedDecoder<'a, Transform>
where
    Transform: PredictionSchemeDecodingTransform<i32, i32>,
{
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated
    }

    fn is_initialized(&self) -> bool {
        self.pos_attribute.is_some() && self.mesh_data.is_some()
    }

    fn get_num_parent_attributes(&self) -> i32 {
        1
    }

    fn get_parent_attribute_type(&self, _i: i32) -> GeometryAttributeType {
        GeometryAttributeType::Position
    }

    fn set_parent_attribute(&mut self, att: &'a PointAttribute) -> bool {
        if att.attribute_type() != GeometryAttributeType::Position || att.num_components() != 3 {
            return false;
        }
        self.pos_attribute = Some(att);
        true
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }
}

impl<'a, Transform> PredictionSchemeDecoder<'a, i32, i32>
    for MeshPredictionSchemeTexCoordsDeprecatedDecoder<'a, Transform>
where
    Transform: PredictionSchemeDecodingTransform<i32, i32>,
{
    fn decode_prediction_data(&mut self, buffer: &mut DecoderBuffer) -> bool {
        let bitstream_version =
            ((buffer.version_major() as u16) << 8) | (buffer.version_minor() as u16);
        let num_orientations = if bitstream_version < 0x0202 {
            match buffer.decode_u32() {
                Ok(v) => v,
                Err(_) => return false,
            }
        } else {
            match buffer.decode_varint() {
                Ok(v) => v as u32,
                Err(_) => return false,
            }
        };

        if num_orientations == 0 {
            return false;
        }
        let Some(mesh_data) = self.mesh_data.as_ref() else {
            return false;
        };
        let Some(corner_table) = mesh_data.corner_table() else {
            return false;
        };
        if num_orientations > corner_table.num_corners() as u32 {
            return false;
        }

        self.orientations.clear();
        self.orientations.reserve(num_orientations as usize);

        let mut last_orientation = true;
        let mut decoder = RAnsBitDecoder::new();
        if !decoder.start_decoding(buffer) {
            return false;
        }
        for _ in 0..num_orientations {
            if !decoder.decode_next_bit() {
                last_orientation = !last_orientation;
            }
            self.orientations.push(last_orientation);
        }
        decoder.end_decoding();

        self.transform.decode_transform_data(buffer)
    }

    fn compute_original_values(
        &mut self,
        in_corr: &[i32],
        out_data: &mut [i32],
        _size: usize,
        num_components: usize,
        entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> bool {
        if num_components != 2 || self.mesh_data.is_none() || self.pos_attribute.is_none() {
            return false;
        }
        let Some(entry_map) = entry_to_point_id_map else {
            return false;
        };
        let mesh_data = self.mesh_data.as_ref().unwrap();
        let Some(data_to_corner_map) = mesh_data.data_to_corner_map() else {
            return false;
        };
        if entry_map.len() < data_to_corner_map.len() {
            return false;
        }
        let required_values = match data_to_corner_map.len().checked_mul(num_components) {
            Some(v) => v,
            None => return false,
        };
        if in_corr.len() < required_values || out_data.len() < required_values {
            return false;
        }

        self.transform.init(num_components);
        let mut predicted_value = [0i32; 2];
        for (p, &corner) in data_to_corner_map.iter().enumerate() {
            if !self.compute_predicted_value(
                CornerIndex(corner),
                out_data,
                p as i32,
                entry_map,
                &mut predicted_value,
            ) {
                return false;
            }
            let dst_offset = p * num_components;
            self.transform.compute_original_value(
                &predicted_value,
                &in_corr[dst_offset..dst_offset + num_components],
                &mut out_data[dst_offset..dst_offset + num_components],
            );
        }

        true
    }
}

fn read_component_as_f32(att: &PointAttribute, index: usize, component: usize) -> Option<f32> {
    let buffer = att.buffer();
    let byte_stride = usize::try_from(att.byte_stride()).ok()?;
    let byte_offset = index
        .checked_mul(byte_stride)?
        .checked_add(component.checked_mul(att.data_type().byte_length())?)?;

    match att.data_type() {
        DataType::Int8 => Some(i8::from_le_bytes(read_bytes::<1>(buffer, byte_offset)?) as f32),
        DataType::Uint8 => Some(u8::from_le_bytes(read_bytes::<1>(buffer, byte_offset)?) as f32),
        DataType::Int16 => Some(i16::from_le_bytes(read_bytes::<2>(buffer, byte_offset)?) as f32),
        DataType::Uint16 => Some(u16::from_le_bytes(read_bytes::<2>(buffer, byte_offset)?) as f32),
        DataType::Int32 => Some(i32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?) as f32),
        DataType::Uint32 => Some(u32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?) as f32),
        DataType::Float32 => Some(f32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?)),
        DataType::Float64 => Some(f64::from_le_bytes(read_bytes::<8>(buffer, byte_offset)?) as f32),
        _ => None,
    }
}

fn read_bytes<const N: usize>(
    buffer: &crate::data_buffer::DataBuffer,
    byte_offset: usize,
) -> Option<[u8; N]> {
    let mut bytes = [0u8; N];
    if !buffer.try_read(byte_offset, &mut bytes) {
        return None;
    }
    Some(bytes)
}

fn f32_to_i32_deprecated(value: f32, round: bool) -> i32 {
    let value = if round { (value + 0.5).floor() } else { value };
    if value.is_nan() || (value as f64) > i32::MAX as f64 || (value as f64) < i32::MIN as f64 {
        i32::MIN
    } else {
        value as i32
    }
}

fn vec2_sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn vec3_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn vec3_mul_scalar(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn vec3_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn vec3_squared_norm(a: [f32; 3]) -> f32 {
    vec3_dot(a, a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corner_table::CornerTable;
    use crate::encoder_buffer::EncoderBuffer;
    use crate::geometry_attribute::PointAttribute;
    use crate::geometry_indices::VertexIndex;
    use crate::prediction_scheme::PredictionSchemeDecoder;
    use crate::prediction_scheme_wrap::PredictionSchemeWrapDecodingTransform;
    use crate::rans_bit_encoder::RAnsBitEncoder;

    #[test]
    fn deprecated_tex_coords_rejects_truncated_position_buffer() {
        let mut pos = PointAttribute::new();
        pos.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            1,
        );
        pos.buffer_mut().write(0, &1.0f32.to_le_bytes());
        pos.buffer_mut().write(4, &2.0f32.to_le_bytes());
        pos.buffer_mut().resize(8);

        assert_eq!(read_component_as_f32(&pos, 0, 2), None);
    }

    #[test]
    fn deprecated_tex_coords_decodes_orientation_prediction() {
        let mut corner_table = CornerTable::new(1);
        assert!(corner_table.init(&[[VertexIndex(0), VertexIndex(1), VertexIndex(2)]]));
        let data_to_corner_map = [1, 2, 0];
        let vertex_to_data_map = [2, 0, 1];
        let mut mesh_data = MeshPredictionSchemeData::new();
        mesh_data.set(&corner_table, &data_to_corner_map, &vertex_to_data_map);

        let mut pos = PointAttribute::new();
        pos.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            3,
        );
        pos.set_identity_mapping();
        for (i, value) in [[5.0f32, 5.0, 0.0], [0.0, 0.0, 0.0], [10.0, 0.0, 0.0]]
            .iter()
            .enumerate()
        {
            let offset = i * 12;
            pos.buffer_mut().write(offset, &value[0].to_le_bytes());
            pos.buffer_mut().write(offset + 4, &value[1].to_le_bytes());
            pos.buffer_mut().write(offset + 8, &value[2].to_le_bytes());
        }

        let mut prediction_data = EncoderBuffer::new();
        prediction_data.set_version(2, 2);
        prediction_data.encode_varint(1u64);
        let mut bit_encoder = RAnsBitEncoder::new();
        bit_encoder.start_encoding();
        bit_encoder.encode_bit(true);
        bit_encoder.end_encoding(&mut prediction_data);
        prediction_data.encode_u32(0);
        prediction_data.encode_u32(10);

        let mut buffer = DecoderBuffer::new(prediction_data.data());
        buffer.set_version(2, 2);
        let mut decoder = MeshPredictionSchemeTexCoordsDeprecatedDecoder::new(
            PredictionSchemeWrapDecodingTransform::<i32>::new(),
        );
        assert!(decoder.init(&mesh_data));
        assert!(decoder.set_parent_attribute(&pos));
        assert!(decoder.decode_prediction_data(&mut buffer));

        let in_corr = [0, 0, 10, 0, 0, 0];
        let mut out = [0; 6];
        assert!(decoder.compute_original_values(
            &in_corr,
            &mut out,
            6,
            2,
            Some(crate::prediction_scheme::EntryToPointIdMap::from_u32_slice(
                &[1, 2, 0],
            )),
        ));
        assert_eq!(out, [0, 0, 10, 0, 5, 5]);
    }
}
