//! Portable texture-coordinate predictor.
//!
//! Predicts UV coordinates from the triangle's 3D positions using integer-only
//! arithmetic (`int_sqrt`, etc.) for bit-exact results across platforms. The
//! current tex-coord prediction scheme. Port of Draco's
//! `prediction_scheme_tex_coords_portable_*`.

use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::{CornerIndex, PointIndex};
use crate::math_utils::int_sqrt;
use crate::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use crate::prediction_scheme::{
    PredictionScheme, PredictionSchemeMethod, PredictionSchemeTransformType,
};

#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "decoder")]
use crate::geometry_indices::INVALID_ATTRIBUTE_VALUE_INDEX;
#[cfg(feature = "decoder")]
use crate::prediction_scheme::{PredictionSchemeDecoder, PredictionSchemeDecodingTransform};
#[cfg(feature = "decoder")]
use crate::prediction_scheme_wrap::PredictionSchemeWrapDecodingTransform;
#[cfg(feature = "decoder")]
use crate::rans_bit_decoder::RAnsBitDecoder;

#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
#[cfg(feature = "encoder")]
use crate::prediction_scheme::{PredictionSchemeEncoder, PredictionSchemeEncodingTransform};
#[cfg(feature = "encoder")]
use crate::prediction_scheme_wrap::PredictionSchemeWrapEncodingTransform;
#[cfg(feature = "encoder")]
use crate::rans_bit_encoder::RAnsBitEncoder;
use crate::status::{DracoError, Status};

#[cfg(feature = "decoder")]
pub struct MeshPredictionSchemeTexCoordsPortableDecoder<'a> {
    transform: PredictionSchemeWrapDecodingTransform<i32>,
    mesh_data: Option<MeshPredictionSchemeData<'a>>,
    orientations: Vec<bool>,
    pos_attribute: Option<&'a PointAttribute>,
}

#[cfg(feature = "decoder")]
impl<'a> MeshPredictionSchemeTexCoordsPortableDecoder<'a> {
    pub fn new(transform: PredictionSchemeWrapDecodingTransform<i32>) -> Self {
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
    ) -> Option<[i64; 3]> {
        let entry_id = usize::try_from(entry_id).ok()?;
        let point_id = entry_to_point_id_map.get(entry_id)?;
        let att = self.pos_attribute?;
        let mut pos = [0i64; 3];
        let val_index = att.mapped_index(PointIndex(point_id));
        if val_index == INVALID_ATTRIBUTE_VALUE_INDEX {
            return None;
        }
        if !read_vector3(att, val_index.0 as usize, &mut pos) {
            return None;
        }
        Some(pos)
    }

    fn get_tex_coord_for_entry_id(&self, entry_id: i32, data: &[i32]) -> Option<[i64; 2]> {
        let offset = usize::try_from(entry_id).ok()?.checked_mul(2)?;
        let u = *data.get(offset)? as i64;
        let v = *data.get(offset + 1)? as i64;
        Some([u, v])
    }

    fn compute_predicted_value(
        &mut self,
        corner_id: CornerIndex,
        data: &[i32],
        data_id: i32,
        entry_to_point_id_map: crate::prediction_scheme::EntryToPointIdMap<'_>,
        predicted_value: &mut [i32; 2],
    ) -> bool {
        let Some(mesh_data) = self.mesh_data.as_ref() else {
            return false;
        };
        let Some(corner_table) = mesh_data.corner_table() else {
            return false;
        };
        let Some(vertex_to_data_map) = mesh_data.vertex_to_data_map() else {
            return false;
        };

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
                predicted_value[0] = p_uv[0] as i32;
                predicted_value[1] = p_uv[1] as i32;
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

            let pn = vec3_sub(&prev_pos, &next_pos);
            let pn_norm2_squared = vec3_squared_norm(&pn);

            if pn_norm2_squared != 0 {
                let cn = vec3_sub(&tip_pos, &next_pos);
                let cn_dot_pn = vec3_dot(&pn, &cn);
                let pn_uv = vec2_sub(&p_uv, &n_uv);

                if !tex_coords_prediction_overflow_checks(
                    &n_uv,
                    &pn_uv,
                    &pn,
                    cn_dot_pn,
                    pn_norm2_squared,
                ) {
                    return false;
                }

                let Some(n_uv_scaled) = checked_vec2_mul_u64(&n_uv, pn_norm2_squared) else {
                    return false;
                };
                let Some(pn_uv_scaled) = checked_vec2_mul_i64(&pn_uv, cn_dot_pn) else {
                    return false;
                };
                let Some(x_uv) = checked_vec2_add(&n_uv_scaled, &pn_uv_scaled) else {
                    return false;
                };

                let Some(pn_scaled) = checked_vec3_mul_i64(&pn, cn_dot_pn) else {
                    return false;
                };
                let Some(x_pos) = checked_vec3_add(
                    &next_pos,
                    &vec3_div_scalar(&pn_scaled, pn_norm2_squared as i64),
                ) else {
                    return false;
                };

                let cx_norm2_squared = vec3_squared_norm(&vec3_sub(&tip_pos, &x_pos));

                let mut cx_uv = [pn_uv[1], -pn_uv[0]]; // Rotated
                let norm_squared_input = cx_norm2_squared.wrapping_mul(pn_norm2_squared);
                let norm_squared = int_sqrt(norm_squared_input);
                let Some(scaled_cx_uv) = checked_vec2_mul_u64(&cx_uv, norm_squared) else {
                    return false;
                };
                cx_uv = scaled_cx_uv;

                if self.orientations.is_empty() {
                    return false;
                }
                let Some(orientation) = self.orientations.pop() else {
                    return false;
                };

                let predicted_uv = if orientation {
                    vec2_wrapping_add_div_u64(&x_uv, &cx_uv, pn_norm2_squared)
                } else {
                    vec2_wrapping_sub_div_u64(&x_uv, &cx_uv, pn_norm2_squared)
                };

                predicted_value[0] = predicted_uv[0] as i32;
                predicted_value[1] = predicted_uv[1] as i32;
                return true;
            }
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

#[cfg(feature = "decoder")]
impl<'a> PredictionScheme<'a> for MeshPredictionSchemeTexCoordsPortableDecoder<'a> {
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionTexCoordsPortable
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

    fn set_parent_attribute(&mut self, att: &'a PointAttribute) -> Status {
        if att.attribute_type() != GeometryAttributeType::Position {
            return Err(DracoError::InvalidParameter(format!(
                "Portable texture-coordinate prediction needs a position parent, got {:?}",
                att.attribute_type()
            )));
        }
        if att.num_components() != 3 {
            return Err(DracoError::InvalidParameter(format!(
                "Portable texture-coordinate prediction needs a 3-component parent, got {}",
                att.num_components()
            )));
        }
        // Safe: lifetime 'a is now tracked by the compiler
        self.pos_attribute = Some(att);
        Ok(())
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }
}

#[cfg(feature = "decoder")]
impl<'a> PredictionSchemeDecoder<'a, i32, i32>
    for MeshPredictionSchemeTexCoordsPortableDecoder<'a>
{
    fn decode_prediction_data(&mut self, buffer: &mut DecoderBuffer) -> Status {
        let num_orientations: i32 = buffer.decode::<i32>().map_err(|_| {
            DracoError::BufferError(
                "Stream ends before the texture-coordinate orientation count".to_string(),
            )
        })?;
        if num_orientations < 0 {
            return Err(DracoError::DracoError(format!(
                "Stream declares {num_orientations} orientations"
            )));
        }
        // C++ Draco omits any bound here, which lets a malformed count (raw
        // i32, up to ~2.1 billion) drive a multi-second loop and a
        // multi-gigabyte reservation - the loop below cannot end early, because
        // `decode_next_bit` returns `false` both for a zero bit and for an
        // exhausted buffer.
        //
        // The bound is structural, not size-based: `compute_original_values`
        // pops exactly one orientation per predicted entry, so a count above the
        // entry count can never be consumed whatever the stream contains. An
        // earlier version compared against the remaining buffer in bits, which
        // is the same false premise the decoder's header guards had - these
        // orientations are rANS-coded, and a run of identical ones compresses to
        // far less than a bit each. Upstream bounds its own decoded counts this
        // way, e.g. `num_topology_splits > num_faces`.
        let Some(max_orientations) = self
            .mesh_data
            .as_ref()
            .and_then(|mesh_data| mesh_data.data_to_corner_map())
            .map(|map| map.len())
        else {
            return Err(DracoError::DracoError(
                "Portable texture-coordinate prediction has no data-to-corner map".to_string(),
            ));
        };
        if num_orientations as usize > max_orientations {
            return Err(DracoError::DracoError(format!(
                "Stream declares {num_orientations} orientations, more than the {max_orientations} entries that can consume them"
            )));
        }

        self.orientations.clear();
        self.orientations.reserve(num_orientations as usize);

        let mut last_orientation = true;
        let mut decoder = RAnsBitDecoder::new();
        if !decoder.start_decoding(buffer) {
            return Err(DracoError::BufferError(
                "Texture-coordinate orientation rANS stream is truncated".to_string(),
            ));
        }

        for _ in 0..num_orientations {
            let is_same = decoder.decode_next_bit();
            let orientation = if is_same {
                last_orientation
            } else {
                !last_orientation
            };
            self.orientations.push(orientation);
            last_orientation = orientation;
        }
        decoder.end_decoding();

        // Draco then decodes the wrap transform data (min/max bounds).
        self.transform.decode_transform_data(buffer)
    }

    fn compute_original_values(
        &mut self,
        in_corr: &[i32],
        out_data: &mut [i32],
        _size: usize,
        num_components: usize,
        entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> Status {
        if num_components != 2 {
            return Err(DracoError::InvalidParameter(format!(
                "Portable texture-coordinate prediction needs 2 components, got {num_components}"
            )));
        }
        let missing = |what: &str| {
            DracoError::DracoError(format!(
                "Portable texture-coordinate prediction has no {what}"
            ))
        };
        if self.pos_attribute.is_none() {
            return Err(missing("position parent"));
        }

        self.transform.init(num_components);

        let Some(entry_map) = entry_to_point_id_map else {
            return Err(missing("entry-to-point map"));
        };

        let Some(mesh_data) = self.mesh_data.as_ref() else {
            return Err(missing("mesh data"));
        };
        let Some(data_to_corner_map) = mesh_data.data_to_corner_map() else {
            return Err(missing("data-to-corner map"));
        };
        if entry_map.len() < data_to_corner_map.len() {
            return Err(DracoError::DracoError(format!(
                "Portable texture-coordinate prediction needs {} entries, the map has {}",
                data_to_corner_map.len(),
                entry_map.len()
            )));
        }
        let corner_map_size = data_to_corner_map.len();
        let Some(required_values) = corner_map_size.checked_mul(num_components) else {
            return Err(DracoError::DracoError(
                "Portable texture-coordinate prediction value count overflow".to_string(),
            ));
        };
        if in_corr.len() < required_values || out_data.len() < required_values {
            return Err(DracoError::DracoError(format!(
                "Portable texture-coordinate prediction needs {required_values} values, has {} corrections and {} outputs",
                in_corr.len(),
                out_data.len()
            )));
        }

        let mut predicted_value = [0i32; 2];
        for p in 0..corner_map_size {
            let corner_id = CornerIndex(data_to_corner_map[p]);

            // We pass `out_data` as `data` because it contains the values decoded so far.
            if !self.compute_predicted_value(
                corner_id,
                out_data,
                p as i32,
                entry_map,
                &mut predicted_value,
            ) {
                return Err(DracoError::DracoError(format!(
                    "Portable texture-coordinate prediction failed at entry {p}"
                )));
            }

            let dst_offset = p * num_components;
            self.transform.compute_original_value(
                &predicted_value,
                &in_corr[dst_offset..dst_offset + 2],
                &mut out_data[dst_offset..dst_offset + 2],
            );
        }
        Ok(())
    }
}

// Helper functions for vector math
fn read_vector3(att: &PointAttribute, index: usize, out: &mut [i64; 3]) -> bool {
    for c in 0..3 {
        let Some(value) = read_component_as_i64(att, index, c) else {
            return false;
        };
        out[c] = value;
    }
    true
}

fn read_component_as_i64(att: &PointAttribute, index: usize, component: usize) -> Option<i64> {
    use crate::draco_types::DataType;
    let buffer = att.buffer();
    let byte_stride = usize::try_from(att.byte_stride()).ok()?;
    let byte_offset = index
        .checked_mul(byte_stride)?
        .checked_add(component.checked_mul(att.data_type().byte_length())?)?;

    match att.data_type() {
        DataType::Int8 => Some(i8::from_le_bytes(read_bytes::<1>(buffer, byte_offset)?) as i64),
        DataType::Uint8 => Some(u8::from_le_bytes(read_bytes::<1>(buffer, byte_offset)?) as i64),
        DataType::Int16 => Some(i16::from_le_bytes(read_bytes::<2>(buffer, byte_offset)?) as i64),
        DataType::Uint16 => Some(u16::from_le_bytes(read_bytes::<2>(buffer, byte_offset)?) as i64),
        DataType::Int32 => Some(i32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?) as i64),
        DataType::Uint32 => Some(u32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?) as i64),
        DataType::Int64 => Some(i64::from_le_bytes(read_bytes::<8>(buffer, byte_offset)?)),
        DataType::Uint64 => {
            i64::try_from(u64::from_le_bytes(read_bytes::<8>(buffer, byte_offset)?)).ok()
        }
        DataType::Float32 => {
            float_to_i64(f32::from_le_bytes(read_bytes::<4>(buffer, byte_offset)?) as f64)
        }
        DataType::Float64 => {
            float_to_i64(f64::from_le_bytes(read_bytes::<8>(buffer, byte_offset)?))
        }
        DataType::Bool => Some(u8::from_le_bytes(read_bytes::<1>(buffer, byte_offset)?) as i64),
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

fn float_to_i64(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    if value < i64::MIN as f64 || value >= i64::MAX as f64 {
        return None;
    }
    Some(value as i64)
}

// These integer vector helpers use wrapping arithmetic to match C++ Draco's
// int64 intermediate math (which relies on two's-complement wraparound) and to
// preserve the documented portable-texcoord cast/wrapping order. For valid
// quantized streams the operands are in range, so wrapping is identical to plain
// arithmetic; on malformed streams with out-of-range positions it wraps like C++
// instead of panicking under overflow checks (debug/fuzz builds).
fn vec3_sub(a: &[i64; 3], b: &[i64; 3]) -> [i64; 3] {
    [
        a[0].wrapping_sub(b[0]),
        a[1].wrapping_sub(b[1]),
        a[2].wrapping_sub(b[2]),
    ]
}
#[cfg(feature = "encoder")]
fn vec3_add(a: &[i64; 3], b: &[i64; 3]) -> [i64; 3] {
    [
        a[0].wrapping_add(b[0]),
        a[1].wrapping_add(b[1]),
        a[2].wrapping_add(b[2]),
    ]
}
fn vec3_squared_norm(a: &[i64; 3]) -> u64 {
    a[0].wrapping_mul(a[0])
        .wrapping_add(a[1].wrapping_mul(a[1]))
        .wrapping_add(a[2].wrapping_mul(a[2])) as u64
}
fn vec3_dot(a: &[i64; 3], b: &[i64; 3]) -> i64 {
    a[0].wrapping_mul(b[0])
        .wrapping_add(a[1].wrapping_mul(b[1]))
        .wrapping_add(a[2].wrapping_mul(b[2]))
}
#[cfg(feature = "encoder")]
fn vec3_mul_scalar(a: &[i64; 3], s: i64) -> [i64; 3] {
    [
        a[0].wrapping_mul(s),
        a[1].wrapping_mul(s),
        a[2].wrapping_mul(s),
    ]
}
fn vec3_div_scalar(a: &[i64; 3], s: i64) -> [i64; 3] {
    [
        a[0].wrapping_div(s),
        a[1].wrapping_div(s),
        a[2].wrapping_div(s),
    ]
}

fn vec2_sub(a: &[i64; 2], b: &[i64; 2]) -> [i64; 2] {
    [a[0].wrapping_sub(b[0]), a[1].wrapping_sub(b[1])]
}
#[cfg(feature = "encoder")]
fn vec2_add(a: &[i64; 2], b: &[i64; 2]) -> [i64; 2] {
    [a[0].wrapping_add(b[0]), a[1].wrapping_add(b[1])]
}
#[cfg(feature = "encoder")]
fn vec2_mul(a: &[i64; 2], s: i64) -> [i64; 2] {
    [a[0].wrapping_mul(s), a[1].wrapping_mul(s)]
}
#[cfg(feature = "encoder")]
fn vec2_div_scalar(a: &[i64; 2], s: i64) -> [i64; 2] {
    [a[0].wrapping_div(s), a[1].wrapping_div(s)]
}

/// The three overflow guards upstream applies before the scaled-space
/// multiplications below them.
///
/// Upstream keeps one predictor shared by its encoder and decoder, so these
/// run on both sides there; this file has a separate encoding and decoding
/// half, and both must call this or the two implementations disagree about
/// which meshes are encodable at all.
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn tex_coords_prediction_overflow_checks(
    n_uv: &[i64; 2],
    pn_uv: &[i64; 2],
    pn: &[i64; 3],
    cn_dot_pn: i64,
    pn_norm2_squared: u64,
) -> bool {
    let n_uv_absmax = vec2_absmax(n_uv);
    if exceeds_i64_product_limit_u64(n_uv_absmax, pn_norm2_squared) {
        return false;
    }

    let pn_uv_absmax = vec2_absmax(pn_uv);
    if pn_uv_absmax == 0 || exceeds_i64_product_limit_u64(cn_dot_pn.unsigned_abs(), pn_uv_absmax) {
        return false;
    }

    let pn_absmax = vec3_absmax(pn);
    if pn_absmax == 0 || exceeds_i64_product_limit_u64(cn_dot_pn.unsigned_abs(), pn_absmax) {
        return false;
    }

    true
}

#[cfg(any(feature = "decoder", feature = "encoder"))]
fn exceeds_i64_product_limit_u64(a_abs: u64, b_abs: u64) -> bool {
    a_abs != 0 && b_abs > (i64::MAX as u64) / a_abs
}

#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec2_absmax(v: &[i64; 2]) -> u64 {
    v[0].unsigned_abs().max(v[1].unsigned_abs())
}

#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec3_absmax(v: &[i64; 3]) -> u64 {
    v[0].unsigned_abs()
        .max(v[1].unsigned_abs())
        .max(v[2].unsigned_abs())
}

#[cfg(feature = "decoder")]
fn checked_vec2_add(a: &[i64; 2], b: &[i64; 2]) -> Option<[i64; 2]> {
    Some([a[0].checked_add(b[0])?, a[1].checked_add(b[1])?])
}

#[cfg(feature = "decoder")]
fn checked_vec3_add(a: &[i64; 3], b: &[i64; 3]) -> Option<[i64; 3]> {
    Some([
        a[0].checked_add(b[0])?,
        a[1].checked_add(b[1])?,
        a[2].checked_add(b[2])?,
    ])
}

#[cfg(feature = "decoder")]
fn checked_vec2_mul_i64(a: &[i64; 2], s: i64) -> Option<[i64; 2]> {
    Some([a[0].checked_mul(s)?, a[1].checked_mul(s)?])
}

#[cfg(feature = "decoder")]
fn checked_vec2_mul_u64(a: &[i64; 2], s: u64) -> Option<[i64; 2]> {
    if s > i64::MAX as u64 && (a[0] != 0 || a[1] != 0) {
        return None;
    }
    checked_vec2_mul_i64(a, s as i64)
}

#[cfg(feature = "decoder")]
fn checked_vec3_mul_i64(a: &[i64; 3], s: i64) -> Option<[i64; 3]> {
    Some([
        a[0].checked_mul(s)?,
        a[1].checked_mul(s)?,
        a[2].checked_mul(s)?,
    ])
}

#[cfg(feature = "decoder")]
fn vec2_wrapping_add_div_u64(a: &[i64; 2], b: &[i64; 2], divisor: u64) -> [i64; 2] {
    let divisor = divisor as i64;
    [
        ((a[0] as u64).wrapping_add(b[0] as u64) as i64) / divisor,
        ((a[1] as u64).wrapping_add(b[1] as u64) as i64) / divisor,
    ]
}

#[cfg(feature = "decoder")]
fn vec2_wrapping_sub_div_u64(a: &[i64; 2], b: &[i64; 2], divisor: u64) -> [i64; 2] {
    let divisor = divisor as i64;
    [
        ((a[0] as u64).wrapping_sub(b[0] as u64) as i64) / divisor,
        ((a[1] as u64).wrapping_sub(b[1] as u64) as i64) / divisor,
    ]
}

#[cfg(feature = "encoder")]
pub struct PredictionSchemeTexCoordsPortableEncodingTransform {
    inner: PredictionSchemeWrapEncodingTransform<i32>,
}

#[cfg(feature = "encoder")]
impl Default for PredictionSchemeTexCoordsPortableEncodingTransform {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "encoder")]
impl PredictionSchemeTexCoordsPortableEncodingTransform {
    pub fn new() -> Self {
        Self {
            inner: PredictionSchemeWrapEncodingTransform::<i32>::new(),
        }
    }
}

#[cfg(feature = "encoder")]
impl PredictionSchemeEncodingTransform<i32, i32>
    for PredictionSchemeTexCoordsPortableEncodingTransform
{
    fn get_type(&self) -> PredictionSchemeTransformType {
        // In Draco, TexCoordsPortable is a prediction *method*, while the
        // integer prediction transform used for corrections is Wrap.
        PredictionSchemeTransformType::Wrap
    }

    fn init(&mut self, _data: &[i32], _size: usize, _num_components: usize) {
        self.inner.init(_data, _size, _num_components);
    }

    fn compute_correction(
        &self,
        original_vals: &[i32],
        predicted_vals: &[i32],
        out_corr_vals: &mut [i32],
    ) {
        self.inner
            .compute_correction(original_vals, predicted_vals, out_corr_vals);
    }

    fn encode_transform_data(&mut self, _buffer: &mut Vec<u8>) -> Status {
        self.inner.encode_transform_data(_buffer)
    }
}

#[cfg(feature = "encoder")]
pub struct MeshPredictionSchemeTexCoordsPortableEncoder<'a> {
    transform: PredictionSchemeTexCoordsPortableEncodingTransform,
    mesh_data: Option<MeshPredictionSchemeData<'a>>,
    orientations: Vec<bool>,
    pos_attribute: Option<&'a PointAttribute>,
    /// Target bitstream, packed as `0xMMmm`; 0 means the newest. The count is
    /// a fixed `i32` at every version, so only the orientation rANS stream's
    /// size prefix depends on this.
    bitstream_version: u16,
}

#[cfg(feature = "encoder")]
impl<'a> MeshPredictionSchemeTexCoordsPortableEncoder<'a> {
    pub fn new(transform: PredictionSchemeTexCoordsPortableEncodingTransform) -> Self {
        Self {
            transform,
            mesh_data: None,
            orientations: Vec::new(),
            pos_attribute: None,
            bitstream_version: 0,
        }
    }

    /// Targets a specific bitstream version, which the caller reads off the
    /// encoder options. Without this the newest layout is written.
    pub fn set_bitstream_version(&mut self, major: u8, minor: u8) {
        self.bitstream_version = crate::version::bitstream_version(major, minor);
    }

    pub fn init(&mut self, mesh_data: &MeshPredictionSchemeData<'a>) -> bool {
        self.mesh_data = Some(mesh_data.clone());
        true
    }

    fn get_position_for_entry_id(
        &self,
        entry_id: i32,
        entry_to_point_id_map: crate::prediction_scheme::EntryToPointIdMap<'_>,
    ) -> [i64; 3] {
        let Some(point_id) = entry_to_point_id_map.get(entry_id as usize) else {
            return [0, 0, 0];
        };
        let att = self.pos_attribute.unwrap();
        let mut pos = [0i64; 3];
        let val_index = att.mapped_index(PointIndex(point_id));
        read_vector3(att, val_index.0 as usize, &mut pos);
        pos
    }

    fn get_tex_coord_for_entry_id(&self, entry_id: i32, data: &[i32]) -> Option<[i64; 2]> {
        let offset = (entry_id * 2) as usize;
        Some([*data.get(offset)? as i64, *data.get(offset + 1)? as i64])
    }

    fn compute_predicted_value(
        &mut self,
        corner_id: CornerIndex,
        data: &[i32],
        data_id: i32,
        entry_to_point_id_map: crate::prediction_scheme::EntryToPointIdMap<'_>,
        predicted_value: &mut [i32; 2],
    ) -> bool {
        let Some(mesh_data) = self.mesh_data.as_ref() else {
            return false;
        };
        let Some(corner_table) = mesh_data.corner_table() else {
            return false;
        };
        let Some(vertex_to_data_map) = mesh_data.vertex_to_data_map() else {
            return false;
        };

        let next_corner_id = corner_table.next(corner_id);
        let prev_corner_id = corner_table.previous(corner_id);

        // A corner on a mesh Draco cannot build a complete corner table for --
        // non-manifold, or with degenerate faces -- has no vertex, and
        // `vertex()` reports that as an invalid index rather than failing. Look
        // it up rather than index with it: upstream reaches the same lookup
        // through `std::vector::at`, and a mesh that gets here is one C++
        // Draco rejects, so refusing to predict ends the encode the same way.
        // The decoding half of this file already guards these; the encoding
        // half never was.
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
                predicted_value[0] = p_uv[0] as i32;
                predicted_value[1] = p_uv[1] as i32;
                return true;
            }

            let tip_pos = self.get_position_for_entry_id(data_id, entry_to_point_id_map);
            let next_pos = self.get_position_for_entry_id(next_data_id, entry_to_point_id_map);
            let prev_pos = self.get_position_for_entry_id(prev_data_id, entry_to_point_id_map);

            let pn = vec3_sub(&prev_pos, &next_pos);
            let pn_norm2_squared = vec3_squared_norm(&pn);

            if pn_norm2_squared != 0 {
                let cn = vec3_sub(&tip_pos, &next_pos);
                let cn_dot_pn = vec3_dot(&pn, &cn);
                let pn_uv = vec2_sub(&p_uv, &n_uv);

                // Same guards the decoding half above applies, and for the same
                // reason: past these bounds the scaled-space products leave
                // `i64`. Upstream shares one predictor between its encoder and
                // decoder and so checks on both sides; without this the encoder
                // silently wrapped and produced a stream for input C++ Draco
                // refuses to encode at all.
                if !tex_coords_prediction_overflow_checks(
                    &n_uv,
                    &pn_uv,
                    &pn,
                    cn_dot_pn,
                    pn_norm2_squared,
                ) {
                    return false;
                }

                let x_uv = vec2_add(
                    &vec2_mul(&n_uv, pn_norm2_squared as i64),
                    &vec2_mul(&pn_uv, cn_dot_pn),
                );

                let x_pos = vec3_add(
                    &next_pos,
                    &vec3_div_scalar(&vec3_mul_scalar(&pn, cn_dot_pn), pn_norm2_squared as i64),
                );

                let cx_norm2_squared = vec3_squared_norm(&vec3_sub(&tip_pos, &x_pos));

                let mut cx_uv = [pn_uv[1], -pn_uv[0]]; // Rotated
                                                       // Upstream computes this in `uint64_t`
                                                       // (`IntSqrt(cx_norm2_squared * pn_norm2_squared)`), where the
                                                       // product wrapping is defined and silent; here the same two
                                                       // values are `i64`, so plain multiplication only differed by
                                                       // panicking in a debug build. The predictor is a heuristic
                                                       // whose output the encoder checks against the real value, so a
                                                       // wrapped magnitude costs compression, never correctness.
                let norm_squared = int_sqrt(cx_norm2_squared.wrapping_mul(pn_norm2_squared));
                cx_uv = vec2_mul(&cx_uv, norm_squared as i64);

                // Encoder logic: compute both and pick best
                let pred_0 = vec2_div_scalar(&vec2_add(&x_uv, &cx_uv), pn_norm2_squared as i64);
                let pred_1 = vec2_div_scalar(&vec2_sub(&x_uv, &cx_uv), pn_norm2_squared as i64);

                let Some(c_uv) = self.get_tex_coord_for_entry_id(data_id, data) else {
                    return false;
                };

                let diff_0 = vec2_sub(&c_uv, &pred_0);
                let diff_1 = vec2_sub(&c_uv, &pred_1);

                let dist_0 = diff_0[0] * diff_0[0] + diff_0[1] * diff_0[1];
                let dist_1 = diff_1[0] * diff_1[0] + diff_1[1] * diff_1[1];

                let predicted_uv;
                if dist_0 < dist_1 {
                    predicted_uv = pred_0;
                    self.orientations.push(true);
                } else {
                    predicted_uv = pred_1;
                    self.orientations.push(false);
                }

                predicted_value[0] = predicted_uv[0] as i32;
                predicted_value[1] = predicted_uv[1] as i32;
                return true;
            }
        }

        // Upstream has a `prev_data_id < data_id` branch ahead of these two, but
        // it writes the offset with a plain `if` and the next branch is another
        // plain `if` whose `else` covers everything remaining -- so whatever the
        // previous corner set is overwritten on every path. Reproducing it as an
        // if/else-if chain would pick the previous corner where Draco picks the
        // next one or the last encoded value.
        let data_offset = if next_data_id < data_id {
            (next_data_id * 2) as usize
        } else if data_id > 0 {
            ((data_id - 1) * 2) as usize
        } else {
            predicted_value[0] = 0;
            predicted_value[1] = 0;
            return true;
        };
        predicted_value[0] = data[data_offset];
        predicted_value[1] = data[data_offset + 1];
        true
    }
}

#[cfg(feature = "encoder")]
impl<'a> PredictionScheme<'a> for MeshPredictionSchemeTexCoordsPortableEncoder<'a> {
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionTexCoordsPortable
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

    fn set_parent_attribute(&mut self, att: &'a PointAttribute) -> Status {
        if att.attribute_type() != GeometryAttributeType::Position {
            return Err(DracoError::InvalidParameter(format!(
                "Portable texture-coordinate prediction needs a position parent, got {:?}",
                att.attribute_type()
            )));
        }
        if att.num_components() != 3 {
            return Err(DracoError::InvalidParameter(format!(
                "Portable texture-coordinate prediction needs a 3-component parent, got {}",
                att.num_components()
            )));
        }
        // Safe: lifetime 'a is now tracked by the compiler
        self.pos_attribute = Some(att);
        Ok(())
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }
}

#[cfg(feature = "encoder")]
impl<'a> PredictionSchemeEncoder<'a, i32, i32>
    for MeshPredictionSchemeTexCoordsPortableEncoder<'a>
{
    fn encode_prediction_data(&mut self, buffer: &mut Vec<u8>) -> Status {
        let mut temp_buffer = EncoderBuffer::new();
        // The orientation rANS stream's size prefix is a u32 pre-2.2 and a
        // varint after; `RAnsBitEncoder::end_encoding` reads that off the
        // buffer it is handed, and a fresh one reports version 0 (newest).
        temp_buffer.set_version(
            (self.bitstream_version >> 8) as u8,
            (self.bitstream_version & 0xff) as u8,
        );
        let num_orientations = self.orientations.len() as i32;
        temp_buffer.encode(num_orientations);

        let mut last_orientation = true;
        let mut encoder = RAnsBitEncoder::new();
        encoder.start_encoding();

        for &orientation in &self.orientations {
            encoder.encode_bit(orientation == last_orientation);
            last_orientation = orientation;
        }
        encoder.end_encoding(&mut temp_buffer);

        buffer.extend_from_slice(temp_buffer.data());

        // Match Draco: after orientations, encode Wrap transform bounds.
        self.transform.encode_transform_data(buffer)
    }

    fn compute_correction_values(
        &mut self,
        in_data: &[i32],
        out_corr: &mut [i32],
        _size: usize,
        num_components: usize,
        entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> Status {
        if num_components != 2 {
            return Err(DracoError::InvalidParameter(format!(
                "Portable texture-coordinate prediction needs 2 components, got {num_components}"
            )));
        }
        let missing = |what: &str| {
            DracoError::DracoError(format!(
                "Portable texture-coordinate prediction has no {what}"
            ))
        };
        if self.pos_attribute.is_none() {
            return Err(missing("position parent"));
        }

        // Initialize Wrap bounds for correction wrapping.
        self.transform.init(in_data, in_data.len(), num_components);

        let Some(entry_map) = entry_to_point_id_map else {
            return Err(missing("entry-to-point map"));
        };

        let Some(mesh_data) = self.mesh_data.as_ref() else {
            return Err(missing("mesh data"));
        };
        let Some(data_to_corner_map) = mesh_data.data_to_corner_map() else {
            return Err(missing("data-to-corner map"));
        };
        let corner_map_size = data_to_corner_map.len();

        let mut predicted_value = [0i32; 2];

        // Iterate in reverse order
        for p in (0..corner_map_size).rev() {
            let corner_id = CornerIndex(data_to_corner_map[p]);

            if !self.compute_predicted_value(
                corner_id,
                in_data,
                p as i32,
                entry_map,
                &mut predicted_value,
            ) {
                return Err(DracoError::DracoError(format!(
                    "Portable texture-coordinate prediction failed at entry {p}"
                )));
            }

            let dst_offset = p * num_components;
            self.transform.compute_correction(
                &in_data[dst_offset..dst_offset + 2],
                &predicted_value,
                &mut out_corr[dst_offset..dst_offset + 2],
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_vector_helpers_wrap_instead_of_panicking_on_extreme_values() {
        // Malformed streams can yield out-of-range i64 positions. These helpers
        // must wrap (matching C++ int64 intermediate math) rather than panic
        // under overflow checks. The values here would overflow plain `*`/`+`/`-`.
        let big = [i64::MAX, i64::MIN, i64::MAX];
        let _ = vec3_squared_norm(&big);
        let _ = vec3_dot(&big, &big);
        let _ = vec3_sub(&big, &[i64::MIN, i64::MAX, i64::MIN]);
        let _ = vec3_div_scalar(&[i64::MIN, i64::MAX, 1], -1);
        let _ = vec2_sub(&[i64::MAX, i64::MIN], &[i64::MIN, i64::MAX]);
    }

    #[cfg(feature = "decoder")]
    use crate::corner_table::CornerTable;
    use crate::draco_types::DataType;
    #[cfg(feature = "decoder")]
    use crate::geometry_indices::VertexIndex;
    #[cfg(feature = "decoder")]
    use crate::prediction_scheme_wrap::PredictionSchemeWrapDecodingTransform;

    #[test]
    fn test_read_component_as_i64_rejects_nan() {
        let mut att = PointAttribute::new();
        att.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            1,
        );
        att.buffer_mut().write(0, &f32::NAN.to_le_bytes());
        att.buffer_mut().write(4, &0.0f32.to_le_bytes());
        att.buffer_mut().write(8, &0.0f32.to_le_bytes());

        assert_eq!(read_component_as_i64(&att, 0, 0), None);
    }

    #[test]
    fn test_read_component_as_i64_accepts_integer_positions() {
        let mut att = PointAttribute::new();
        att.init(
            GeometryAttributeType::Position,
            3,
            DataType::Int32,
            false,
            1,
        );
        att.buffer_mut().write(0, &123i32.to_le_bytes());
        att.buffer_mut().write(4, &(-7i32).to_le_bytes());
        att.buffer_mut().write(8, &99i32.to_le_bytes());

        let mut out = [0i64; 3];
        assert!(read_vector3(&att, 0, &mut out));
        assert_eq!(out, [123, -7, 99]);
    }

    #[test]
    fn test_read_component_as_i64_rejects_truncated_buffer() {
        let mut att = PointAttribute::new();
        att.init(
            GeometryAttributeType::Position,
            3,
            DataType::Int32,
            false,
            1,
        );
        att.buffer_mut().write(0, &123i32.to_le_bytes());
        att.buffer_mut().write(4, &(-7i32).to_le_bytes());
        att.buffer_mut().resize(8);

        let mut out = [0i64; 3];
        assert_eq!(read_component_as_i64(&att, 0, 2), None);
        assert!(!read_vector3(&att, 0, &mut out));
    }

    #[cfg(feature = "decoder")]
    fn make_triangle_corner_table() -> CornerTable {
        let mut corner_table = CornerTable::new(1);
        corner_table.init(&[[VertexIndex(0), VertexIndex(1), VertexIndex(2)]]);
        corner_table.compute_vertex_corners(3);
        corner_table
    }

    #[cfg(feature = "decoder")]
    fn make_position_attribute(values: &[[i32; 3]]) -> PointAttribute {
        let mut att = PointAttribute::new();
        att.init(
            GeometryAttributeType::Position,
            3,
            DataType::Int32,
            false,
            values.len(),
        );
        att.set_identity_mapping();
        for (i, value) in values.iter().enumerate() {
            let offset = i * 12;
            att.buffer_mut().write(offset, &value[0].to_le_bytes());
            att.buffer_mut().write(offset + 4, &value[1].to_le_bytes());
            att.buffer_mut().write(offset + 8, &value[2].to_le_bytes());
        }
        att
    }

    #[cfg(feature = "decoder")]
    fn predicted_for_triangle(
        vertex_to_data_map: Vec<i32>,
        data_id: i32,
        data: &[i32],
    ) -> Option<[i32; 2]> {
        let corner_table = make_triangle_corner_table();
        let data_to_corner_map = vec![0, 1, 2];
        let mut mesh_data = MeshPredictionSchemeData::new();
        mesh_data.set(&corner_table, &data_to_corner_map, &vertex_to_data_map);

        let transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
        let mut decoder = MeshPredictionSchemeTexCoordsPortableDecoder::new(transform);
        assert!(decoder.init(&mesh_data));

        let mut predicted = [i32::MIN; 2];
        if decoder.compute_predicted_value(
            CornerIndex(0),
            data,
            data_id,
            crate::prediction_scheme::EntryToPointIdMap::from_u32_slice(&[0, 1, 2]),
            &mut predicted,
        ) {
            Some(predicted)
        } else {
            None
        }
    }

    #[test]
    #[cfg(feature = "decoder")]
    fn test_tex_coords_portable_fallback_predicts_zero_for_first_value() {
        let predicted = predicted_for_triangle(vec![0, 1, 2], 0, &[7, 8, 9, 10, 11, 12]);
        assert_eq!(predicted, Some([0, 0]));
    }

    #[test]
    #[cfg(feature = "decoder")]
    fn test_tex_coords_portable_fallback_uses_next_when_available() {
        let predicted = predicted_for_triangle(vec![1, 0, 2], 1, &[7, 8, 9, 10, 11, 12]);
        assert_eq!(predicted, Some([7, 8]));
    }

    #[test]
    #[cfg(feature = "decoder")]
    fn test_tex_coords_portable_fallback_uses_previous_entry_when_prev_only_available() {
        let predicted = predicted_for_triangle(vec![2, 9, 0], 2, &[7, 8, 9, 10, 11, 12]);
        assert_eq!(predicted, Some([9, 10]));
    }

    #[test]
    #[cfg(feature = "decoder")]
    fn test_tex_coords_portable_fallback_uses_previous_entry_when_no_neighbor_available() {
        let predicted = predicted_for_triangle(vec![2, 3, 4], 2, &[7, 8, 9, 10, 11, 12]);
        assert_eq!(predicted, Some([9, 10]));
    }

    #[test]
    #[cfg(feature = "decoder")]
    fn test_tex_coords_portable_overflow_risk_returns_false() {
        let corner_table = make_triangle_corner_table();
        let data_to_corner_map = vec![0, 1, 2];
        let vertex_to_data_map = vec![2, 0, 1];
        let mut mesh_data = MeshPredictionSchemeData::new();
        mesh_data.set(&corner_table, &data_to_corner_map, &vertex_to_data_map);

        let pos_att = make_position_attribute(&[[0, 1, 0], [0, 0, 0], [100_000, 0, 0]]);
        let transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
        let mut decoder = MeshPredictionSchemeTexCoordsPortableDecoder::new(transform);
        assert!(decoder.set_parent_attribute(&pos_att).is_ok());
        assert!(decoder.init(&mesh_data));

        let data = [i32::MAX, i32::MAX, 0, 0, 1, 1];
        let mut predicted = [0; 2];
        assert!(!decoder.compute_predicted_value(
            CornerIndex(0),
            &data,
            2,
            crate::prediction_scheme::EntryToPointIdMap::from_u32_slice(&[0, 1, 2]),
            &mut predicted,
        ));
    }
}
