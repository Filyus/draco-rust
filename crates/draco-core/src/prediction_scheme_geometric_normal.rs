//! Geometric normal predictor.
//!
//! Predicts vertex normals from the surrounding mesh geometry (face normals
//! gathered through the corner table), so only a small octahedral correction is
//! coded. [`NormalPredictionMode`] selects the prediction variant. Port of
//! Draco's `prediction_scheme_geometric_normal_*`.

use crate::corner_table::CornerTable;
use crate::draco_types::DataType;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::{CornerIndex, PointIndex, INVALID_CORNER_INDEX};

#[cfg(feature = "decoder")]
use crate::geometry_indices::{VertexIndex, INVALID_ATTRIBUTE_VALUE_INDEX};
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
use crate::prediction_scheme_normal_octahedron_canonicalized_encoding_transform::PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform;
#[cfg(feature = "encoder")]
use crate::rans_bit_encoder::RAnsBitEncoder;
use crate::status::{DracoError, Status};

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

    pub fn init(&mut self, mesh_data: &MeshPredictionSchemeData<'a>) {
        self.mesh_data = Some(mesh_data.clone());
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

    /// The position lookup the predictor runs on, resolved from this decoder's
    /// own state. Only the tests reach for it this way; the decode pass builds
    /// the same lookup once and keeps it.
    #[cfg(test)]
    fn corner_positions(&self) -> Option<CornerPositions<'_>> {
        let mesh_data = self.mesh_data.as_ref()?;
        Some(CornerPositions::new(
            mesh_data.corner_table()?,
            mesh_data.vertex_to_data_map()?,
            self.entry_to_point_id_map?,
            self.pos_attribute?,
        ))
    }
}

/// Resolves a vertex to its position, once, with no cache.
///
/// The corner table used for prediction may be seam-adjusted, which can
/// introduce new vertex ids that don't correspond to original `PointIndex`.
/// `vertex_to_data_map` + `entry_to_point_id_map` resolve back to an original
/// point id. Any break in that chain means "no position", which the callers
/// treat as the origin — the same answer Draco's C++ arrives at.
#[cfg(feature = "decoder")]
fn position_for_vertex(
    vertex_to_data_map: &[i32],
    entry_to_point_id_map: crate::prediction_scheme::EntryToPointIdMap<'_>,
    pos_attribute: &PointAttribute,
    v: crate::geometry_indices::VertexIndex,
) -> [i32; 3] {
    let data_id = *vertex_to_data_map.get(v.0 as usize).unwrap_or(&-1);
    if data_id < 0 {
        return [0, 0, 0];
    }
    let Some(point_id) = entry_to_point_id_map.get(data_id as usize) else {
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

/// The position lookup above, memoised per vertex.
///
/// The predictor walks every corner around a vertex and reads both neighbours'
/// positions, so a given vertex's position is asked for once per incident
/// corner — six times over on a regular mesh, and again for each of its
/// neighbours' own predictions. The lookup is pure in the vertex: the parent
/// position attribute is fully decoded before the normals that predict from it,
/// and neither map changes during the pass. So decode each position once.
#[cfg(feature = "decoder")]
struct CornerPositions<'b> {
    corner_table: &'b CornerTable,
    vertex_to_data_map: &'b [i32],
    entry_to_point_id_map: crate::prediction_scheme::EntryToPointIdMap<'b>,
    pos_attribute: &'b PointAttribute,
    /// Indexed by vertex id; `cached[v]` says whether `cache[v]` was filled.
    cache: Vec<[i32; 3]>,
    cached: Vec<bool>,
}

#[cfg(feature = "decoder")]
impl<'b> CornerPositions<'b> {
    fn new(
        corner_table: &'b CornerTable,
        vertex_to_data_map: &'b [i32],
        entry_to_point_id_map: crate::prediction_scheme::EntryToPointIdMap<'b>,
        pos_attribute: &'b PointAttribute,
    ) -> Self {
        // A vertex outside the map has no position anyway, so sizing the cache
        // by the map covers every vertex that can produce one.
        let num_vertices = vertex_to_data_map.len();
        Self {
            corner_table,
            vertex_to_data_map,
            entry_to_point_id_map,
            pos_attribute,
            cache: vec![[0i32; 3]; num_vertices],
            cached: vec![false; num_vertices],
        }
    }

    fn get(&mut self, corner_id: CornerIndex) -> [i32; 3] {
        if corner_id == INVALID_CORNER_INDEX {
            return [0, 0, 0];
        }
        self.get_by_vertex(self.corner_table.vertex(corner_id))
    }

    /// Same cache as [`get`](Self::get), keyed directly by vertex.
    ///
    /// A caller that only ever wanted the vertex a corner names -- the fan
    /// walk in [`compute_predicted_value`] is the case this exists for -- can
    /// reach it through [`CornerTable::vertex_after`]/`vertex_before`, one
    /// fused bounds-checked load instead of computing the neighbour corner
    /// with `next`/`previous` and handing it here to be turned back into a
    /// vertex a second time.
    fn get_by_vertex(&mut self, v: VertexIndex) -> [i32; 3] {
        let vi = v.0 as usize;
        if vi >= self.cache.len() {
            // Off the end of the map: the uncached path returns the origin.
            return position_for_vertex(
                self.vertex_to_data_map,
                self.entry_to_point_id_map,
                self.pos_attribute,
                v,
            );
        }
        if self.cached[vi] {
            return self.cache[vi];
        }
        let pos = position_for_vertex(
            self.vertex_to_data_map,
            self.entry_to_point_id_map,
            self.pos_attribute,
            v,
        );
        self.cache[vi] = pos;
        self.cached[vi] = true;
        pos
    }
}

/// Predicts one normal from the geometry around `corner_id`.
///
/// Free-standing rather than a method so the caller can hold the resolved
/// position lookup across the whole pass while still borrowing the decoder's
/// other fields.
#[cfg(feature = "decoder")]
fn compute_predicted_value(
    positions: &mut CornerPositions<'_>,
    prediction_mode: NormalPredictionMode,
    corner_id: CornerIndex,
    prediction: &mut [i32; 3],
) {
    if corner_id == INVALID_CORNER_INDEX {
        prediction[0] = 0;
        prediction[1] = 0;
        prediction[2] = 0;
        return;
    }

    let corner_table = positions.corner_table;
    let pos_cent = positions.get(corner_id);

    let mut normal = [0i128; 3];

    let mut cit = VertexCornersIterator::new(corner_table, corner_id);
    while !cit.end() {
        // c_next/c_prev were never wanted for themselves, only for the vertex
        // each names -- vertex_after/vertex_before answer that in one fused
        // lookup instead of computing the neighbour corner and handing it to
        // `get`, which would immediately turn it back into a vertex.
        let (v_next, v_prev) = if prediction_mode == NormalPredictionMode::OneTriangle {
            (
                corner_table.vertex_after(corner_id),
                corner_table.vertex_before(corner_id),
            )
        } else {
            (
                corner_table.vertex_after(cit.corner()),
                corner_table.vertex_before(cit.corner()),
            )
        };

        let pos_prev = positions.get_by_vertex(v_prev);
        let pos_next = positions.get_by_vertex(v_next);

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

        if prediction_mode == NormalPredictionMode::OneTriangle {
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

    fn set_parent_attribute(&mut self, att: &'a PointAttribute) -> Status {
        if att.attribute_type() != GeometryAttributeType::Position {
            return Err(DracoError::invalid_parameter(format!(
                "Geometric normal prediction needs a position parent, got {:?}",
                att.attribute_type()
            )));
        }
        if att.num_components() != 3 {
            return Err(DracoError::invalid_parameter(format!(
                "Geometric normal prediction needs a 3-component parent, got {}",
                att.num_components()
            )));
        }
        // Safe: lifetime 'a is now tracked by the compiler
        self.pos_attribute = Some(att);
        Ok(())
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        PredictionSchemeTransformType::NormalOctahedronCanonicalized
    }
}

#[cfg(feature = "decoder")]
impl<'a> PredictionSchemeDecoder<'a, i32> for MeshPredictionSchemeGeometricNormalDecoder<'a> {
    fn compute_original_values(
        &mut self,
        data: &mut [i32],
        _size: usize,
        num_components: usize,
        _entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> Status {
        if !self.is_initialized() {
            return Err(DracoError::general(
                "Geometric normal prediction was never initialized".to_string(),
            ));
        }
        self.transform.init(num_components);

        let missing =
            |what: &str| DracoError::general(format!("Geometric normal prediction has no {what}"));
        let Some(mesh_data) = self.mesh_data.as_ref() else {
            return Err(missing("mesh data"));
        };
        let Some(data_to_corner_map) = mesh_data.data_to_corner_map() else {
            return Err(missing("data-to-corner map"));
        };
        let corner_map_size = data_to_corner_map.len();
        if corner_map_size * num_components > data.len() {
            return Err(DracoError::general(format!(
                "Geometric normal prediction needs {} values, has {}",
                corner_map_size * num_components,
                data.len()
            )));
        }

        // Resolve the position lookup once for the whole pass rather than once
        // per corner. `is_initialized` already vouches for the corner table,
        // the position attribute and the entry map; a missing vertex-to-data
        // map leaves every position at the origin, which predicts a zero normal
        // for every entry — the same as the per-corner lookup used to return.
        let mut positions = match (
            mesh_data.corner_table(),
            mesh_data.vertex_to_data_map(),
            self.pos_attribute,
            self.entry_to_point_id_map,
        ) {
            (
                Some(corner_table),
                Some(vertex_to_data_map),
                Some(pos_attribute),
                Some(entry_map),
            ) => Some(CornerPositions::new(
                corner_table,
                vertex_to_data_map,
                entry_map,
                pos_attribute,
            )),
            _ => None,
        };

        let mut pred_normal_3d = [0i32; 3];

        for i in 0..corner_map_size {
            let corner_id = CornerIndex(data_to_corner_map[i]);
            match positions.as_mut() {
                Some(positions) => compute_predicted_value(
                    positions,
                    self.prediction_mode,
                    corner_id,
                    &mut pred_normal_3d,
                ),
                None => pred_normal_3d = [0, 0, 0],
            }
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
            self.transform
                .compute_original_value(&prediction, &mut data[offset..offset + num_components]);
        }
        Ok(())
    }

    fn decode_prediction_data(&mut self, buffer: &mut DecoderBuffer) -> Status {
        let start_pos = buffer.position();
        let bitstream_version: u16 = buffer.bitstream_version();
        if bitstream_version < 0x0202 && !cfg!(feature = "legacy_bitstream_decode") {
            return Err(DracoError::unsupported_feature(format!(
                "Geometric normal prediction below 2.2 needs the legacy_bitstream_decode feature (stream is {}.{})",
                bitstream_version >> 8,
                bitstream_version & 0xff
            )));
        }

        let try_decode_at_pos = |this: &mut Self, buf: &mut DecoderBuffer| -> Status {
            this.transform.decode_transform_data(buf)?;

            // Set up octahedral toolbox from decoded transform.
            this.octahedron_tool_box
                .set_quantization_bits(this.transform.quantization_bits());

            // Backward compatibility: bitstreams < 2.2 store prediction mode.
            if bitstream_version < 0x0202 {
                let mode = buf.decode_u8().map_err(|_| {
                    DracoError::buffer(
                        "Stream ends before the pre-2.2 normal prediction mode".to_string(),
                    )
                })?;
                if mode > NormalPredictionMode::TriangleArea as u8 {
                    return Err(DracoError::unsupported_feature(format!(
                        "Normal prediction mode {mode}"
                    )));
                }
                this.prediction_mode = if mode == 0 {
                    NormalPredictionMode::OneTriangle
                } else {
                    NormalPredictionMode::TriangleArea
                };
            }

            let Some(num_values) = this
                .mesh_data
                .as_ref()
                .and_then(|m| m.data_to_corner_map())
                .map(|map| map.len())
            else {
                return Err(DracoError::general(
                    "Geometric normal prediction has no data-to-corner map".to_string(),
                ));
            };

            this.flip_normal_bits.clear();
            this.flip_normal_bits.reserve(num_values);

            let mut decoder = RAnsBitDecoder::new();
            if !decoder.start_decoding(buf) {
                return Err(DracoError::buffer(
                    "Normal flip-bit rANS stream is truncated".to_string(),
                ));
            }

            for _ in 0..num_values {
                this.flip_normal_bits.push(decoder.decode_next_bit());
            }
            decoder.end_decoding();
            this.flip_normal_bit_index = 0;
            Ok(())
        };

        match try_decode_at_pos(self, buffer) {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = buffer.set_position(start_pos);
                Err(error)
            }
        }
    }
}

/// Walks every corner attached to the vertex of the corner it starts from.
///
/// Starts where it is told and sweeps right; on a closed ring that returns to
/// the start and the walk is done. Only a boundary stops the right sweep early,
/// and only then does the walk go back to the start and sweep left for the rest
/// of the fan.
///
/// The obvious alternative — find the leftmost corner first, then sweep right
/// across the whole fan — costs a second full lap of `swing_left` before the
/// first corner is ever yielded, and on a closed ring that lap ends exactly
/// where it began, having established nothing. Both orders visit the same
/// corners the same number of times, and the only consumer sums exact integer
/// cross products over them, so the order is not observable.
struct VertexCornersIterator {
    /// Where the walk began, and what the right sweep must not return to.
    start_corner: CornerIndex,
    corner: CornerIndex,
    /// Set once the right sweep has run into a boundary.
    sweeping_left: bool,
    is_end: bool,
}

impl VertexCornersIterator {
    fn new(_corner_table: &CornerTable, corner_id: CornerIndex) -> Self {
        Self {
            start_corner: corner_id,
            corner: corner_id,
            sweeping_left: false,
            is_end: corner_id == INVALID_CORNER_INDEX,
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

        if !self.sweeping_left {
            let right = corner_table.swing_right(self.corner);
            if right == self.start_corner {
                // Closed ring, back where it started: every corner is done.
                self.finish();
                return;
            }
            if right != INVALID_CORNER_INDEX {
                self.corner = right;
                return;
            }
            // Boundary. The corners left of the start are still unvisited.
            self.sweeping_left = true;
            self.set_or_finish(corner_table.swing_left(self.start_corner));
            return;
        }

        self.set_or_finish(corner_table.swing_left(self.corner));
    }

    fn set_or_finish(&mut self, corner: CornerIndex) {
        // Returning to the start during the left sweep would mean the ring was
        // closed after all, which the right sweep would have caught; treat it
        // as the end rather than lapping the fan forever.
        if corner == INVALID_CORNER_INDEX || corner == self.start_corner {
            self.finish();
        } else {
            self.corner = corner;
        }
    }

    fn finish(&mut self) {
        self.corner = INVALID_CORNER_INDEX;
        self.is_end = true;
    }
}

#[cfg(feature = "encoder")]
pub struct MeshPredictionSchemeGeometricNormalEncoder<'a> {
    transform: PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform,
    /// Held apart from the transform, as the C++ encoder holds
    /// `octahedron_tool_box_`: the transform canonicalises the correction, the
    /// toolbox turns predicted integer vectors into octahedral coordinates.
    octahedron_tool_box: OctahedronToolBox,
    mesh_data: Option<MeshPredictionSchemeData<'a>>,
    pos_attribute: Option<&'a PointAttribute>,
    prediction_mode: NormalPredictionMode,
    flip_normal_bit_encoder: RAnsBitEncoder,
    /// Target bitstream, packed as `0xMMmm`; 0 means the newest. Only whether
    /// the prediction mode is written out depends on it.
    bitstream_version: u16,
}

#[cfg(feature = "encoder")]
impl<'a> MeshPredictionSchemeGeometricNormalEncoder<'a> {
    pub fn new(transform: PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform) -> Self {
        Self {
            transform,
            octahedron_tool_box: OctahedronToolBox::new(),
            mesh_data: None,
            pos_attribute: None,
            prediction_mode: NormalPredictionMode::TriangleArea,
            flip_normal_bit_encoder: RAnsBitEncoder::new(),
            bitstream_version: 0,
        }
    }

    /// Targets a specific bitstream version, which the caller reads off the
    /// encoder options. Without this the newest layout is written.
    pub fn set_bitstream_version(&mut self, major: u8, minor: u8) {
        self.bitstream_version = crate::version::bitstream_version(major, minor);
    }

    pub fn init(&mut self, mesh_data: &MeshPredictionSchemeData<'a>) {
        self.mesh_data = Some(mesh_data.clone());
    }
}

/// The encoder's position lookup, memoised per vertex.
///
/// Mirrors [`CornerPositions`] on the decoder side, for the same reason: the
/// predictor asks for a vertex's position once per corner incident to it, and
/// again while predicting each of its neighbours. The two differ in what they
/// return -- the encoder works in unclamped `i64` -- so they share the shape
/// rather than the code.
#[cfg(feature = "encoder")]
struct EncoderCornerPositions<'b> {
    corner_table: &'b CornerTable,
    vertex_to_data_map: &'b [i32],
    map: crate::prediction_scheme::EntryToPointIdMap<'b>,
    pos_attribute: &'b PointAttribute,
    cache: Vec<[i64; 3]>,
    cached: Vec<bool>,
}

#[cfg(feature = "encoder")]
impl<'b> EncoderCornerPositions<'b> {
    fn new(
        corner_table: &'b CornerTable,
        vertex_to_data_map: &'b [i32],
        map: crate::prediction_scheme::EntryToPointIdMap<'b>,
        pos_attribute: &'b PointAttribute,
    ) -> Self {
        let num_vertices = vertex_to_data_map.len();
        Self {
            corner_table,
            vertex_to_data_map,
            map,
            pos_attribute,
            cache: vec![[0i64; 3]; num_vertices],
            cached: vec![false; num_vertices],
        }
    }

    fn get(&mut self, ci: CornerIndex) -> [i64; 3] {
        let vertex = self.corner_table.vertex(ci).0 as usize;
        if vertex >= self.cache.len() {
            return self.lookup(vertex);
        }
        if self.cached[vertex] {
            return self.cache[vertex];
        }
        let pos = self.lookup(vertex);
        self.cache[vertex] = pos;
        self.cached[vertex] = true;
        pos
    }

    fn lookup(&self, vertex: usize) -> [i64; 3] {
        let data_id = self.vertex_to_data_map[vertex];

        let Some(point_id) = self.map.get(data_id as usize) else {
            return [0, 0, 0];
        };
        let pos_val_id = self.pos_attribute.mapped_index(PointIndex(point_id));

        let mut pos = [0i64; 3];
        if !read_vector3_as_i64(self.pos_attribute, pos_val_id.0 as usize, &mut pos) {
            return [0, 0, 0];
        }
        pos
    }
}

/// Predicts one normal from the geometry around `corner_id`, encoder side.
#[cfg(feature = "encoder")]
fn compute_encoder_predicted_value(
    positions: &mut EncoderCornerPositions<'_>,
    prediction_mode: NormalPredictionMode,
    corner_id: CornerIndex,
    prediction: &mut [i32; 3],
) {
    if corner_id == INVALID_CORNER_INDEX {
        prediction[0] = 0;
        prediction[1] = 0;
        prediction[2] = 0;
        return;
    }

    let corner_table = positions.corner_table;
    let mut cit = VertexCornersIterator::new(corner_table, corner_id);
    let pos_cent = positions.get(corner_id);

    let mut normal = [0i64; 3];

    while !cit.end() {
        let base = if prediction_mode == NormalPredictionMode::OneTriangle {
            corner_id
        } else {
            cit.corner()
        };
        let c_next = corner_table.next(base);
        let c_prev = corner_table.previous(base);

        let pos_next = positions.get(c_next);
        let pos_prev = positions.get(c_prev);

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

        if prediction_mode == NormalPredictionMode::OneTriangle {
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

    fn set_parent_attribute(&mut self, att: &'a PointAttribute) -> Status {
        if att.attribute_type() != GeometryAttributeType::Position {
            return Err(DracoError::invalid_parameter(format!(
                "Geometric normal prediction needs a position parent, got {:?}",
                att.attribute_type()
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
impl<'a> PredictionSchemeEncoder<'a, i32, i32> for MeshPredictionSchemeGeometricNormalEncoder<'a> {
    fn compute_correction_values(
        &mut self,
        in_data: &[i32],
        out_corr: &mut [i32],
        // Unused, as upstream leaves it: the loop below is bounded by the corner
        // map, not by the scalar count.
        _size: usize,
        num_components: usize,
        entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> Status {
        if !self.is_initialized() {
            return Err(DracoError::general(
                "Geometric normal prediction was never initialized".to_string(),
            ));
        }

        let Some(map) = entry_to_point_id_map else {
            return Err(DracoError::invalid_parameter(
                "Geometric normal prediction needs an entry-to-point map".to_string(),
            ));
        };

        // Expecting in_data in octahedral coordinates (portable attribute)
        if num_components != 2 {
            return Err(DracoError::invalid_parameter(format!(
                "Geometric normal prediction needs 2 octahedral components, got {num_components}"
            )));
        }

        // SetQuantizationBits(transform().quantization_bits()) -- without this
        // the toolbox stays at its default and center_value is 0, which
        // collapses every prediction to the origin.
        if !self
            .octahedron_tool_box
            .set_quantization_bits(self.transform.quantization_bits())
        {
            return Err(DracoError::invalid_parameter(format!(
                "Octahedral quantization bits {} outside the supported range 2..=30",
                self.transform.quantization_bits()
            )));
        }

        self.flip_normal_bit_encoder.start_encoding();

        let mesh_data = self.mesh_data.as_ref().unwrap();
        let data_to_corner_map = mesh_data.data_to_corner_map().unwrap();

        let mut pred_normal_3d = [0i32; 3];
        let mut pos_pred_normal_oct = [0i32; 2];
        let mut neg_pred_normal_oct = [0i32; 2];
        let mut pos_correction = [0i32; 2];
        let mut neg_correction = [0i32; 2];

        // Over the corner map, not over `size`: upstream loops to
        // `data_to_corner_map()->size()`, and `size` counts scalar values, which
        // for a 2-component octahedral attribute is twice the entry count.
        // Resolved once for the pass, and memoising each vertex's position, for
        // the reason spelled out on `EncoderCornerPositions`.
        let mut positions = EncoderCornerPositions::new(
            mesh_data.corner_table().unwrap(),
            mesh_data.vertex_to_data_map().unwrap(),
            map,
            self.pos_attribute.unwrap(),
        );

        let corner_map_size = data_to_corner_map.len();
        for i in 0..corner_map_size {
            let corner_id = CornerIndex(data_to_corner_map[i]);

            compute_encoder_predicted_value(
                &mut positions,
                self.prediction_mode,
                corner_id,
                &mut pred_normal_3d,
            );

            self.octahedron_tool_box
                .canonicalize_integer_vector(&mut pred_normal_3d);

            // Compute octahedral coordinates for both possible directions
            let (s_pos, t_pos) = self
                .octahedron_tool_box
                .integer_vector_to_quantized_octahedral_coords(&pred_normal_3d);
            pos_pred_normal_oct[0] = s_pos;
            pos_pred_normal_oct[1] = t_pos;

            let neg_normal_3d = [-pred_normal_3d[0], -pred_normal_3d[1], -pred_normal_3d[2]];
            let (s_neg, t_neg) = self
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

            pos_correction[0] = self.octahedron_tool_box.mod_max(pos_correction[0]);
            pos_correction[1] = self.octahedron_tool_box.mod_max(pos_correction[1]);
            neg_correction[0] = self.octahedron_tool_box.mod_max(neg_correction[0]);
            neg_correction[1] = self.octahedron_tool_box.mod_max(neg_correction[1]);

            let pos_abs_sum = pos_correction[0].abs() + pos_correction[1].abs();
            let neg_abs_sum = neg_correction[0].abs() + neg_correction[1].abs();

            if pos_abs_sum < neg_abs_sum {
                self.flip_normal_bit_encoder.encode_bit(false);
                out_corr[offset] = self.octahedron_tool_box.make_positive(pos_correction[0]);
                out_corr[offset + 1] = self.octahedron_tool_box.make_positive(pos_correction[1]);
            } else {
                self.flip_normal_bit_encoder.encode_bit(true);
                out_corr[offset] = self.octahedron_tool_box.make_positive(neg_correction[0]);
                out_corr[offset + 1] = self.octahedron_tool_box.make_positive(neg_correction[1]);
            }
        }
        Ok(())
    }

    fn encode_prediction_data(&mut self, buffer: &mut Vec<u8>) -> Status {
        self.transform.encode_transform_data(buffer)?;

        // Pre-2.2 carries the prediction mode between the transform data and
        // the flip bits; 2.2 dropped it and assumes triangle area. Mirror of
        // the decoder's read at the same position.
        if self.bitstream_version != 0 && self.bitstream_version < 0x0202 {
            buffer.push(self.prediction_mode as u8);
        }

        let mut temp_buffer = EncoderBuffer::new();
        // The flip-normal rANS stream's size prefix is a u32 pre-2.2 and a
        // varint after, and `RAnsBitEncoder::end_encoding` reads that off the
        // buffer it is handed -- a fresh one reports version 0, meaning newest.
        temp_buffer.set_version(
            (self.bitstream_version >> 8) as u8,
            (self.bitstream_version & 0xff) as u8,
        );
        self.flip_normal_bit_encoder.end_encoding(&mut temp_buffer);
        buffer.extend_from_slice(temp_buffer.data());
        Ok(())
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
        decoder.init(&mesh_data);
        assert!(decoder.set_parent_attribute(&position_attribute).is_ok());

        let entry_to_point_id_map = [0u32];
        decoder
            .set_entry_to_point_id_map(EntryToPointIdMap::from_u32_slice(&entry_to_point_id_map));

        let mut positions = decoder.corner_positions().expect("lookup resolves");
        assert_eq!(positions.get(CornerIndex(0)), [0, 0, 0]);
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
        decoder.init(&mesh_data);
        assert!(decoder.set_parent_attribute(&position_attribute).is_ok());

        let entry_to_point_id_map = [0u32];
        decoder
            .set_entry_to_point_id_map(EntryToPointIdMap::from_u32_slice(&entry_to_point_id_map));

        let mut positions = decoder.corner_positions().expect("lookup resolves");
        assert_eq!(positions.get(CornerIndex(0)), [0, 0, 0]);
    }

    /// The memoised lookup must answer exactly what the uncached one does, for
    /// every corner and on the second visit as well as the first. Without this,
    /// a wrong cache is caught only by the C++ fingerprint parity suite, which
    /// needs the reference build to be present.
    #[test]
    fn mesh_geometric_normal_position_cache_matches_the_uncached_lookup() {
        // Two triangles sharing an edge, so vertices 1 and 2 each carry two
        // corners and the second visit has to come out of the cache.
        let mut corner_table = CornerTable::new(2);
        corner_table.set_face_vertices(FaceIndex(0), PointIndex(0), PointIndex(1), PointIndex(2));
        corner_table.set_face_vertices(FaceIndex(1), PointIndex(2), PointIndex(1), PointIndex(3));

        let data_to_corner_map = [0u32, 1, 2, 3];
        let vertex_to_data_map = [0, 1, 2, 3];
        let mut mesh_data = MeshPredictionSchemeData::new();
        mesh_data.set(&corner_table, &data_to_corner_map, &vertex_to_data_map);

        let num_points = 4;
        let mut position_attribute = PointAttribute::new();
        position_attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Int32,
            false,
            num_points,
        );
        // Distinct per point, so mixing two vertices up cannot go unnoticed.
        for p in 0..num_points {
            for c in 0..3 {
                let value = (10 * p + c) as i32;
                position_attribute
                    .buffer_mut()
                    .update(&value.to_le_bytes(), Some((p * 3 + c) * 4));
            }
        }

        let mut decoder = MeshPredictionSchemeGeometricNormalDecoder::new(
            PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform::new(),
        );
        decoder.init(&mesh_data);
        assert!(decoder.set_parent_attribute(&position_attribute).is_ok());

        let entry_to_point_id_map = [0u32, 1, 2, 3];
        decoder
            .set_entry_to_point_id_map(EntryToPointIdMap::from_u32_slice(&entry_to_point_id_map));

        let mut positions = decoder.corner_positions().expect("lookup resolves");
        for corner in 0..6u32 {
            let corner = CornerIndex(corner);
            let expected = position_for_vertex(
                &vertex_to_data_map,
                EntryToPointIdMap::from_u32_slice(&entry_to_point_id_map),
                &position_attribute,
                corner_table.vertex(corner),
            );
            assert_eq!(positions.get(corner), expected, "first visit, {corner:?}");
            assert_eq!(positions.get(corner), expected, "cached visit, {corner:?}");
        }

        // The fixture is only meaningful if it really shares vertices between
        // corners; otherwise nothing above ever reads the cache.
        let vertices: Vec<_> = (0..6u32)
            .map(|c| corner_table.vertex(CornerIndex(c)).0)
            .collect();
        let unique: std::collections::BTreeSet<_> = vertices.iter().collect();
        assert!(
            unique.len() < vertices.len(),
            "shared vertices: {vertices:?}"
        );
    }

    /// Builds a corner table from faces given as vertex triples.
    fn corner_table_from(faces: &[[u32; 3]]) -> CornerTable {
        let faces: Vec<[crate::geometry_indices::VertexIndex; 3]> = faces
            .iter()
            .map(|f| f.map(crate::geometry_indices::VertexIndex))
            .collect();
        let mut corner_table = CornerTable::new(faces.len());
        assert!(corner_table.init(&faces), "corner table builds");
        corner_table
    }

    /// Every corner the table says belongs to `v`, found without swinging.
    fn corners_of(corner_table: &CornerTable, v: u32) -> std::collections::BTreeSet<u32> {
        (0..corner_table.num_corners() as u32)
            .filter(|&c| corner_table.vertex(CornerIndex(c)).0 == v)
            .collect()
    }

    fn walk_from(corner_table: &CornerTable, start: CornerIndex) -> Vec<u32> {
        let mut visited = Vec::new();
        let mut cit = VertexCornersIterator::new(corner_table, start);
        while !cit.end() {
            visited.push(cit.corner().0);
            cit.next(corner_table);
            // The fixtures are tiny; a walk longer than the table means the
            // iterator is lapping rather than terminating.
            assert!(visited.len() <= corner_table.num_corners(), "walk loops");
        }
        visited
    }

    /// The walk must cover the vertex's whole fan exactly once, from whichever
    /// corner it starts — including a fan with a boundary, where covering it
    /// means sweeping both ways. Draco's own suite had nothing on this: with
    /// the boundary sweep deleted, only the C++ legacy-normal decode test
    /// noticed.
    #[test]
    fn vertex_corners_iterator_covers_the_fan_once_from_every_start() {
        // Closed fan: three triangles around vertex 0, ring 1-2-3.
        let closed = corner_table_from(&[[0, 1, 2], [0, 2, 3], [0, 3, 1]]);
        // Open fan: the ring 1-2-3-4 is not closed back to 1.
        let open = corner_table_from(&[[0, 1, 2], [0, 2, 3], [0, 3, 4]]);

        for (name, corner_table) in [("closed", &closed), ("open", &open)] {
            let expected = corners_of(corner_table, 0);
            assert!(expected.len() == 3, "{name} fan has three corners at v0");

            for &start in &expected {
                let visited = walk_from(corner_table, CornerIndex(start));
                let unique: std::collections::BTreeSet<_> = visited.iter().copied().collect();
                assert_eq!(
                    unique, expected,
                    "{name} fan from corner {start}: visited {visited:?}"
                );
                assert_eq!(
                    unique.len(),
                    visited.len(),
                    "{name} fan from corner {start} visits a corner twice: {visited:?}"
                );
            }
        }
    }
}
