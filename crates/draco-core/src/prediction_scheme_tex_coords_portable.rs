//! Portable texture-coordinate predictor.
//!
//! Predicts UV coordinates from the triangle's 3D positions using integer-only
//! arithmetic (`int_sqrt`, etc.) for bit-exact results across platforms. The
//! current tex-coord prediction scheme. Port of Draco's
//! `prediction_scheme_tex_coords_portable_*`.
//!
//! Upstream keeps the prediction itself in one place --
//! `mesh_prediction_scheme_tex_coords_portable_predictor.h`, included by both
//! its encoder and decoder -- and so does this file now:
//! [`MeshPredictionSchemeTexCoordsPortablePredictor`] carries the whole
//! computation and both schemes call it, with `is_encoder` selecting only the
//! orientation handling. What is genuinely encoder-only -- trying both
//! orientations and keeping the closer one -- lives behind that flag; the
//! arithmetic is written once, so the two sides cannot disagree about overflow
//! discipline. What that disagreement costs is a stream the encoder writes and
//! the decoder refuses, pinned by
//! `fuzz/seeds/encode_drc/texcoord_portable_encoder_wraps_where_decoder_refuses.bin`.

use crate::geometry_attribute::GeometryAttributeType;
use crate::geometry_indices::{CornerIndex, PointIndex, INVALID_ATTRIBUTE_VALUE_INDEX};
use crate::math_utils::int_sqrt;
use crate::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use crate::portable_attribute::PredictionParent;
use crate::prediction_scheme::{
    PredictionScheme, PredictionSchemeMethod, PredictionSchemeTransformType,
};

#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
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

/// The prediction both tex-coord-portable schemes run. Port of upstream's
/// `MeshPredictionSchemeTexCoordsPortablePredictor`, which its encoder and
/// decoder headers share; this file's two schemes hold one each and delegate.
///
/// The arithmetic discipline is upstream's, written down once: the three
/// guards of [`tex_coords_prediction_overflow_checks`] refuse, on both sides,
/// exactly where upstream's `ComputePredictedValue` returns false; everything
/// past them wraps in two's complement, because upstream runs those same
/// expressions in C++ signed and unsigned 64-bit math and every step below
/// the guards is defined-wrap there. One computation means the decoder
/// reproduces the encoder bit for bit on whatever input gets this far,
/// including the extreme ones.
#[cfg(any(feature = "decoder", feature = "encoder"))]
struct MeshPredictionSchemeTexCoordsPortablePredictor<'a> {
    pos_parent: Option<PredictionParent<'a>>,
    mesh_data: Option<MeshPredictionSchemeData<'a>>,
    /// Encoded / decoded array of UV flips. The encoder pushes one per
    /// predicted entry, the decoder pops one; the encoder walks its entries in
    /// reverse and the decoder forward, so a stack is the order both agree on.
    orientations: Vec<bool>,
}

#[cfg(any(feature = "decoder", feature = "encoder"))]
impl<'a> MeshPredictionSchemeTexCoordsPortablePredictor<'a> {
    fn new() -> Self {
        Self {
            pos_parent: None,
            mesh_data: None,
            orientations: Vec::new(),
        }
    }

    fn init(&mut self, mesh_data: &MeshPredictionSchemeData<'a>) {
        self.mesh_data = Some(mesh_data.clone());
    }

    fn set_parent_attribute(&mut self, parent: PredictionParent<'a>) -> Status {
        if parent.attribute_type() != GeometryAttributeType::Position {
            return Err(DracoError::invalid_parameter(format!(
                "Portable texture-coordinate prediction needs a position parent, got {:?}",
                parent.attribute_type()
            )));
        }
        if parent.num_components() != 3 {
            return Err(DracoError::invalid_parameter(format!(
                "Portable texture-coordinate prediction needs a 3-component parent, got {}",
                parent.num_components()
            )));
        }
        self.pos_parent = Some(parent);
        Ok(())
    }

    fn is_initialized(&self) -> bool {
        self.pos_parent.is_some() && self.mesh_data.is_some()
    }

    // Refuses the same positions on both sides: a point outside the entry map,
    // one the position attribute maps nowhere, and a value no `i64` can hold --
    // a non-finite or out-of-range float among them. Standing in zeros for
    // those would have one side predict from a position the other does not.
    fn get_position_for_entry_id(
        &self,
        entry_id: i32,
        entry_to_point_id_map: crate::prediction_scheme::EntryToPointIdMap<'_>,
    ) -> Option<[i64; 3]> {
        let entry_id = usize::try_from(entry_id).ok()?;
        let point_id = entry_to_point_id_map.get(entry_id)?;
        let parent = self.pos_parent?;
        let mut pos = [0i64; 3];
        let val_index = parent.mapped_index(PointIndex(point_id));
        if val_index == INVALID_ATTRIBUTE_VALUE_INDEX {
            return None;
        }
        if !parent.read_vector3_as_i64(val_index.0 as usize, &mut pos) {
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

    /// Computes the predicted UV coordinate for `data_id`, from whatever is
    /// already coded in `data`.
    ///
    /// `is_encoder` selects only the orientation handling: the encoder computes
    /// both orientations, keeps the closer one and records the choice; the
    /// decoder pops the choice that was recorded for it. Everything else --
    /// neighbour lookup, positions, arithmetic, refusal conditions -- is this
    /// one body, which is why the two sides cannot disagree about it.
    fn compute_predicted_value(
        &mut self,
        is_encoder: bool,
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

        // Upstream looks the neighbour corners up with `Next`/`Previous` and
        // then reads their vertex; `vertex_after`/`vertex_before` are those
        // compositions in one lookup, and both spell the same vertices. A
        // corner on a mesh Draco cannot build a complete corner table for --
        // non-manifold, or with degenerate faces -- has no vertex, and the
        // lookup reports that as the invalid index rather than failing; the
        // map lookup below then refuses the prediction, which is where
        // upstream's `std::vector::at` would throw.
        let next_vert_id = corner_table.vertex_after(corner_id).0 as usize;
        let prev_vert_id = corner_table.vertex_before(corner_id).0 as usize;

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
                // A degenerate UV triangle: no reliable prediction.
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

            // We use the positions of the triangle above to predict the texture
            // coordinate on the tip corner C, through the projection X of C
            // onto |prev_pos - next_pos|:
            //
            //              C
            //             /.  \
            //            / .     \
            //           /  .        \
            //          N---X----------P
            let pn = vec3_sub(&prev_pos, &next_pos);
            let pn_norm2_squared = vec3_squared_norm(&pn);

            if pn_norm2_squared != 0 {
                let cn = vec3_sub(&tip_pos, &next_pos);
                let cn_dot_pn = vec3_dot(&pn, &cn);
                let pn_uv = vec2_sub(&p_uv, &n_uv);

                // Because the computation is integer, the normalized factor is
                // never materialized; everything runs in the scaled space
                // |x_uv = X_UV * PN.Norm2Squared()|. The three guards bound
                // exactly the products that scaled space produces.
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

                // CX_UV is PN_UV rotated by 90 degrees and scaled by
                // CX.Norm2() / PN.Norm2(); in the scaled space the factor is
                // CX.Norm2() * PN.Norm2(). That product is upstream's
                // `uint64_t` multiply, where wrapping is defined, and the
                // wrapping is deliberate here too.
                let mut cx_uv = [pn_uv[1], -pn_uv[0]]; // Rotated PN_UV.
                let norm_squared = int_sqrt(cx_norm2_squared.wrapping_mul(pn_norm2_squared));
                cx_uv = vec2_mul(&cx_uv, norm_squared as i64);

                let predicted_uv;
                if is_encoder {
                    // Compute both possible vectors and keep the one closer to
                    // the value being coded; the choice travels to the decoder
                    // as one orientation bit. The distance is upstream's
                    // `SquaredNorm` on an `int64` vector, wrapping and all.
                    let pred_0 = vec2_wrapping_add_div_u64(&x_uv, &cx_uv, pn_norm2_squared);
                    let pred_1 = vec2_wrapping_sub_div_u64(&x_uv, &cx_uv, pn_norm2_squared);

                    let Some(c_uv) = self.get_tex_coord_for_entry_id(data_id, data) else {
                        return false;
                    };

                    let dist_0 = vec2_squared_norm(&vec2_sub(&c_uv, &pred_0));
                    let dist_1 = vec2_squared_norm(&vec2_sub(&c_uv, &pred_1));

                    if dist_0 < dist_1 {
                        predicted_uv = pred_0;
                        self.orientations.push(true);
                    } else {
                        predicted_uv = pred_1;
                        self.orientations.push(false);
                    }
                } else {
                    if self.orientations.is_empty() {
                        return false;
                    }
                    let Some(orientation) = self.orientations.pop() else {
                        return false;
                    };
                    // Perform the combine in unsigned type, as upstream's
                    // decoder does to avoid signed overflow -- bit-identical to
                    // the encoder's signed wrapping above -- and divide in
                    // `i64`, which is where the cast lands.
                    predicted_uv = if orientation {
                        vec2_wrapping_add_div_u64(&x_uv, &cx_uv, pn_norm2_squared)
                    } else {
                        vec2_wrapping_sub_div_u64(&x_uv, &cx_uv, pn_norm2_squared)
                    };
                }

                predicted_value[0] = predicted_uv[0] as i32;
                predicted_value[1] = predicted_uv[1] as i32;
                return true;
            }
        }

        // Delta coding, for a corner whose neighbours cannot carry a geometric
        // prediction. Upstream writes this branch with a plain `if` on the
        // previous corner and follows it with another plain `if` whose `else`
        // covers everything remaining, so the previous corner never survives to
        // be used -- upstream issue #1117 -- and the overwrite is what both
        // sides spell, because the decoder has to predict what the encoder
        // wrote. A negative id is a vertex that never received data; upstream
        // multiplies it out and indexes the value array with it, which is
        // memory-undefined, and this refuses the prediction instead.
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

// These integer vector helpers use wrapping arithmetic to match C++ Draco's
// int64 intermediate math (which relies on two's-complement wraparound) and to
// preserve the documented portable-texcoord cast/wrapping order. For valid
// quantized streams the operands are in range, so wrapping is identical to plain
// arithmetic; on malformed streams with out-of-range positions it wraps like C++
// instead of panicking under overflow checks (debug/fuzz builds).
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec3_sub(a: &[i64; 3], b: &[i64; 3]) -> [i64; 3] {
    [
        a[0].wrapping_sub(b[0]),
        a[1].wrapping_sub(b[1]),
        a[2].wrapping_sub(b[2]),
    ]
}
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec3_add(a: &[i64; 3], b: &[i64; 3]) -> [i64; 3] {
    [
        a[0].wrapping_add(b[0]),
        a[1].wrapping_add(b[1]),
        a[2].wrapping_add(b[2]),
    ]
}
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec3_squared_norm(a: &[i64; 3]) -> u64 {
    a[0].wrapping_mul(a[0])
        .wrapping_add(a[1].wrapping_mul(a[1]))
        .wrapping_add(a[2].wrapping_mul(a[2])) as u64
}
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec3_dot(a: &[i64; 3], b: &[i64; 3]) -> i64 {
    a[0].wrapping_mul(b[0])
        .wrapping_add(a[1].wrapping_mul(b[1]))
        .wrapping_add(a[2].wrapping_mul(b[2]))
}
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec3_mul_scalar(a: &[i64; 3], s: i64) -> [i64; 3] {
    [
        a[0].wrapping_mul(s),
        a[1].wrapping_mul(s),
        a[2].wrapping_mul(s),
    ]
}
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec3_div_scalar(a: &[i64; 3], s: i64) -> [i64; 3] {
    [
        a[0].wrapping_div(s),
        a[1].wrapping_div(s),
        a[2].wrapping_div(s),
    ]
}

#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec2_sub(a: &[i64; 2], b: &[i64; 2]) -> [i64; 2] {
    [a[0].wrapping_sub(b[0]), a[1].wrapping_sub(b[1])]
}
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec2_add(a: &[i64; 2], b: &[i64; 2]) -> [i64; 2] {
    [a[0].wrapping_add(b[0]), a[1].wrapping_add(b[1])]
}
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec2_mul(a: &[i64; 2], s: i64) -> [i64; 2] {
    [a[0].wrapping_mul(s), a[1].wrapping_mul(s)]
}
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec2_squared_norm(a: &[i64; 2]) -> i64 {
    a[0].wrapping_mul(a[0])
        .wrapping_add(a[1].wrapping_mul(a[1]))
}

/// The three overflow guards upstream applies before the scaled-space
/// multiplications below them.
///
/// Upstream keeps one predictor shared by its encoder and decoder, so these
/// run on both sides there; the shared predictor above runs them on both
/// sides here, and they are the only refusals the geometric branch has.
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

/// `(a + b) / d` with the combine performed in unsigned arithmetic -- the
/// spell upstream's decoder uses "to avoid signed integer overflow", and
/// bit-identical to the signed wrapping add an `i64` computation produces.
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec2_wrapping_add_div_u64(a: &[i64; 2], b: &[i64; 2], divisor: u64) -> [i64; 2] {
    let divisor = divisor as i64;
    [
        ((a[0] as u64).wrapping_add(b[0] as u64) as i64) / divisor,
        ((a[1] as u64).wrapping_add(b[1] as u64) as i64) / divisor,
    ]
}

/// `(a - b) / d`, the subtracting twin of `vec2_wrapping_add_div_u64`.
#[cfg(any(feature = "decoder", feature = "encoder"))]
fn vec2_wrapping_sub_div_u64(a: &[i64; 2], b: &[i64; 2], divisor: u64) -> [i64; 2] {
    let divisor = divisor as i64;
    [
        ((a[0] as u64).wrapping_sub(b[0] as u64) as i64) / divisor,
        ((a[1] as u64).wrapping_sub(b[1] as u64) as i64) / divisor,
    ]
}

#[cfg(feature = "decoder")]
pub struct MeshPredictionSchemeTexCoordsPortableDecoder<'a> {
    transform: PredictionSchemeWrapDecodingTransform<i32>,
    predictor: MeshPredictionSchemeTexCoordsPortablePredictor<'a>,
}

#[cfg(feature = "decoder")]
impl<'a> MeshPredictionSchemeTexCoordsPortableDecoder<'a> {
    pub fn new(transform: PredictionSchemeWrapDecodingTransform<i32>) -> Self {
        Self {
            transform,
            predictor: MeshPredictionSchemeTexCoordsPortablePredictor::new(),
        }
    }

    pub fn init(&mut self, mesh_data: &MeshPredictionSchemeData<'a>) {
        self.predictor.init(mesh_data);
    }
}

#[cfg(feature = "decoder")]
impl<'a> PredictionScheme<'a> for MeshPredictionSchemeTexCoordsPortableDecoder<'a> {
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionTexCoordsPortable
    }

    fn is_initialized(&self) -> bool {
        self.predictor.is_initialized()
    }

    fn get_num_parent_attributes(&self) -> i32 {
        1
    }

    fn get_parent_attribute_type(&self, _i: i32) -> GeometryAttributeType {
        GeometryAttributeType::Position
    }

    fn set_parent_attribute(&mut self, parent: PredictionParent<'a>) -> Status {
        self.predictor.set_parent_attribute(parent)
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }
}

#[cfg(feature = "decoder")]
impl<'a> PredictionSchemeDecoder<'a, i32> for MeshPredictionSchemeTexCoordsPortableDecoder<'a> {
    fn decode_prediction_data(&mut self, buffer: &mut DecoderBuffer) -> Status {
        let num_orientations: i32 = buffer.decode::<i32>().map_err(|_| {
            DracoError::buffer(
                "Stream ends before the texture-coordinate orientation count".to_string(),
            )
        })?;
        if num_orientations < 0 {
            return Err(DracoError::general(format!(
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
            .predictor
            .mesh_data
            .as_ref()
            .and_then(|mesh_data| mesh_data.data_to_corner_map())
            .map(|map| map.len())
        else {
            return Err(DracoError::general(
                "Portable texture-coordinate prediction has no data-to-corner map".to_string(),
            ));
        };
        if num_orientations as usize > max_orientations {
            return Err(DracoError::general(format!(
                "Stream declares {num_orientations} orientations, more than the {max_orientations} entries that can consume them"
            )));
        }

        self.predictor.orientations.clear();
        self.predictor
            .orientations
            .reserve(num_orientations as usize);

        let mut last_orientation = true;
        let mut decoder = RAnsBitDecoder::new();
        if !decoder.start_decoding(buffer) {
            return Err(DracoError::buffer(
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
            self.predictor.orientations.push(orientation);
            last_orientation = orientation;
        }
        decoder.end_decoding();

        // Draco then decodes the wrap transform data (min/max bounds).
        self.transform.decode_transform_data(buffer)
    }

    fn compute_original_values(
        &mut self,
        data: &mut [i32],
        _size: usize,
        num_components: usize,
        entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> Status {
        if num_components != 2 {
            return Err(DracoError::invalid_parameter(format!(
                "Portable texture-coordinate prediction needs 2 components, got {num_components}"
            )));
        }
        let missing = |what: &str| {
            DracoError::general(format!(
                "Portable texture-coordinate prediction has no {what}"
            ))
        };
        if self.predictor.pos_parent.is_none() {
            return Err(missing("position parent"));
        }

        self.transform.init(num_components)?;

        let Some(entry_map) = entry_to_point_id_map else {
            return Err(missing("entry-to-point map"));
        };

        let Some(mesh_data) = self.predictor.mesh_data.as_ref() else {
            return Err(missing("mesh data"));
        };
        let Some(data_to_corner_map) = mesh_data.data_to_corner_map() else {
            return Err(missing("data-to-corner map"));
        };
        if entry_map.len() < data_to_corner_map.len() {
            return Err(DracoError::general(format!(
                "Portable texture-coordinate prediction needs {} entries, the map has {}",
                data_to_corner_map.len(),
                entry_map.len()
            )));
        }
        let corner_map_size = data_to_corner_map.len();
        let Some(required_values) = corner_map_size.checked_mul(num_components) else {
            return Err(DracoError::general(
                "Portable texture-coordinate prediction value count overflow".to_string(),
            ));
        };
        if data.len() < required_values {
            return Err(DracoError::general(format!(
                "Portable texture-coordinate prediction needs {required_values} values, has {}",
                data.len()
            )));
        }

        let mut predicted_value = [0i32; 2];
        for p in 0..corner_map_size {
            let corner_id = CornerIndex(data_to_corner_map[p]);

            // The buffer itself is the source of the values decoded so far.
            if !self.predictor.compute_predicted_value(
                false,
                corner_id,
                data,
                p as i32,
                entry_map,
                &mut predicted_value,
            ) {
                return Err(DracoError::general(format!(
                    "Portable texture-coordinate prediction failed at entry {p}"
                )));
            }

            let dst_offset = p * num_components;
            self.transform
                .compute_original_value(&predicted_value, &mut data[dst_offset..dst_offset + 2]);
        }
        Ok(())
    }
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
    predictor: MeshPredictionSchemeTexCoordsPortablePredictor<'a>,
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
            predictor: MeshPredictionSchemeTexCoordsPortablePredictor::new(),
            bitstream_version: 0,
        }
    }

    /// Targets a specific bitstream version, which the caller reads off the
    /// encoder options. Without this the newest layout is written.
    pub fn set_bitstream_version(&mut self, major: u8, minor: u8) {
        self.bitstream_version = crate::version::bitstream_version(major, minor);
    }

    pub fn init(&mut self, mesh_data: &MeshPredictionSchemeData<'a>) {
        self.predictor.init(mesh_data);
    }
}

#[cfg(feature = "encoder")]
impl<'a> PredictionScheme<'a> for MeshPredictionSchemeTexCoordsPortableEncoder<'a> {
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionTexCoordsPortable
    }

    fn is_initialized(&self) -> bool {
        self.predictor.is_initialized()
    }

    fn get_num_parent_attributes(&self) -> i32 {
        1
    }

    fn get_parent_attribute_type(&self, _i: i32) -> GeometryAttributeType {
        GeometryAttributeType::Position
    }

    fn set_parent_attribute(&mut self, parent: PredictionParent<'a>) -> Status {
        self.predictor.set_parent_attribute(parent)
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
        let num_orientations = self.predictor.orientations.len() as i32;
        temp_buffer.encode(num_orientations);

        let mut last_orientation = true;
        let mut encoder = RAnsBitEncoder::new();
        encoder.start_encoding();

        for &orientation in &self.predictor.orientations {
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
            return Err(DracoError::invalid_parameter(format!(
                "Portable texture-coordinate prediction needs 2 components, got {num_components}"
            )));
        }
        let missing = |what: &str| {
            DracoError::general(format!(
                "Portable texture-coordinate prediction has no {what}"
            ))
        };
        if self.predictor.pos_parent.is_none() {
            return Err(missing("position parent"));
        }

        // Initialize Wrap bounds for correction wrapping.
        self.transform.init(in_data, in_data.len(), num_components);

        let Some(entry_map) = entry_to_point_id_map else {
            return Err(missing("entry-to-point map"));
        };

        let Some(mesh_data) = self.predictor.mesh_data.as_ref() else {
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

            if !self.predictor.compute_predicted_value(
                true,
                corner_id,
                in_data,
                p as i32,
                entry_map,
                &mut predicted_value,
            ) {
                return Err(DracoError::general(format!(
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
        let _ = vec2_squared_norm(&[i64::MIN, i64::MAX]);
        let _ = vec2_add(&[i64::MAX, i64::MAX], &[i64::MAX, i64::MAX]);
        let _ = vec2_mul(&[i64::MIN, i64::MIN], i64::MIN);
    }

    #[cfg(feature = "decoder")]
    use crate::corner_table::CornerTable;
    use crate::draco_types::DataType;
    use crate::geometry_attribute::PointAttribute;
    #[cfg(feature = "decoder")]
    use crate::geometry_indices::VertexIndex;
    use crate::portable_attribute::PredictionParent;

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

        let mut predictor = MeshPredictionSchemeTexCoordsPortablePredictor::new();
        predictor.init(&mesh_data);

        let mut predicted = [i32::MIN; 2];
        if predictor.compute_predicted_value(
            false,
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
    fn test_tex_coords_portable_decoder_needs_the_orientation_it_reproduces() {
        let corner_table = make_triangle_corner_table();
        let data_to_corner_map = vec![0, 1, 2];
        let vertex_to_data_map = vec![2, 0, 1];
        let mut mesh_data = MeshPredictionSchemeData::new();
        mesh_data.set(&corner_table, &data_to_corner_map, &vertex_to_data_map);

        let pos_att = make_position_attribute(&[[0, 1, 0], [0, 0, 0], [100_000, 0, 0]]);
        let mut predictor = MeshPredictionSchemeTexCoordsPortablePredictor::new();
        assert!(predictor
            .set_parent_attribute(PredictionParent::portable(&pos_att).expect("portable"))
            .is_ok());
        predictor.init(&mesh_data);

        let data = [i32::MAX, i32::MAX, 0, 0, 1, 1];
        let mut predicted = [0; 2];
        // The geometric branch runs here, and it cannot answer without the
        // orientation the encoder recorded: the decoder reproduces a choice
        // that was made for it.
        assert!(!predictor.compute_predicted_value(
            false,
            CornerIndex(0),
            &data,
            2,
            crate::prediction_scheme::EntryToPointIdMap::from_u32_slice(&[0, 1, 2]),
            &mut predicted,
        ));
    }

    /// The invariant the shared predictor exists for: on an input whose scaled
    /// arithmetic leaves `i64`, the encoder's choice and the decoder's
    /// reproduction are still the same number. Before the two copies were one,
    /// the decoder wrapped less than the encoder and refused such a stream.
    #[test]
    #[cfg(feature = "decoder")]
    fn encoder_and_decoder_produce_the_same_prediction_when_the_arithmetic_wraps() {
        let corner_table = make_triangle_corner_table();
        let data_to_corner_map = vec![0, 1, 2];
        let vertex_to_data_map = vec![2, 0, 1];
        let mut mesh_data = MeshPredictionSchemeData::new();
        mesh_data.set(&corner_table, &data_to_corner_map, &vertex_to_data_map);

        // Positions shaped so the cx scaling genuinely leaves `i64`: pn is
        // 200_000 long along x, the tip sits 100_000 up from `next`, so
        // `cn_dot_pn` is 0 and every upstream guard passes, while the rotated
        // pn_uv scaled by sqrt(cx2 * pn2) reaches 1.84e19 -- past 2^63. This
        // is the input the two former copies disagreed about: the old decoder
        // refused it, the old encoder wrapped it.
        let pos_att = make_position_attribute(&[[0, 0, 0], [200_000, 0, 0], [0, 100_000, 0]]);
        let mut predictor = MeshPredictionSchemeTexCoordsPortablePredictor::new();
        assert!(predictor
            .set_parent_attribute(PredictionParent::portable(&pos_att).expect("portable"))
            .is_ok());
        predictor.init(&mesh_data);

        // n_uv = (0, 0), p_uv = (920_000_000, 0), c_uv = (1, 1).
        let data = [0, 0, 900_000_000, 0, 1, 1];
        let map = crate::prediction_scheme::EntryToPointIdMap::from_u32_slice(&[0, 1, 2]);

        let mut predicted = [0; 2];
        assert!(
            predictor.compute_predicted_value(true, CornerIndex(0), &data, 2, map, &mut predicted),
            "the encoder computes through the wrap"
        );
        let orientation = *predictor.orientations.last().expect("one choice recorded");

        predictor.orientations.clear();
        predictor.orientations.push(orientation);
        let mut reproduced = [0; 2];
        assert!(
            predictor.compute_predicted_value(
                false,
                CornerIndex(0),
                &data,
                2,
                map,
                &mut reproduced
            ),
            "the decoder reproduces through the same wrap"
        );
        assert_eq!(predicted, reproduced);
    }
}
