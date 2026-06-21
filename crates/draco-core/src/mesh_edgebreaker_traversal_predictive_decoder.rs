//! Predictive EdgeBreaker traversal decoder (legacy "type 1" traversal).
//!
//! The default connectivity traversal in Draco 0.9.1 and earlier. It maintains
//! per-vertex valences of the decoded portion of the mesh and uses them to
//! predict the next symbol: after a `C` or `R` symbol it predicts `R` when the
//! pivot vertex valence is `< 6`, otherwise `C`. A binary prediction stream says
//! whether each prediction was correct; mispredicted (and unpredicted) symbols
//! come from the main traversal symbol stream. Replaced by the valence traversal
//! in Draco 0.10.0. Port of Draco's
//! `mesh_edgebreaker_traversal_predictive_decoder.h`.

use crate::corner_table::CornerTable;
use crate::edgebreaker_connectivity_decoder::EdgebreakerTraversalDecoder;
use crate::geometry_indices::{CornerIndex, VertexIndex};
use crate::mesh_edgebreaker_shared::{EdgeFaceName, TopologySplitEventData};
use crate::rans_bit_decoder::RAnsBitDecoder;

// Internal EdgeBreaker symbol ids (match the rest of the decoder): C=0, S=1,
// L=2, R=3, E=4.
const SYMBOL_C: i32 = 0;
const SYMBOL_R: i32 = 3;

pub struct MeshEdgebreakerTraversalPredictiveDecoder<'a> {
    num_vertices: usize,
    vertex_valences: Vec<i32>,
    last_symbol: i32,
    predicted_symbol: i32,
    prediction_decoder: RAnsBitDecoder<'a>,
    topology_split_data: Vec<TopologySplitEventData>,
    split_event_remaining: usize,
    pub(crate) start_face_decoder: RAnsBitDecoder<'a>,
    pub(crate) has_start_face_bits: bool,
    pub(crate) start_face_bits_legacy: Option<Vec<bool>>,
    start_face_bits_legacy_index: usize,
    pub(crate) processed_connectivity_corners: Vec<u32>,
    /// Raw bits of the main traversal symbol stream, decoded on demand for
    /// unpredicted / mispredicted symbols (1 bit for `C`, otherwise 3 bits).
    direct_symbol_bits: Option<Vec<bool>>,
    direct_symbol_bit_index: usize,
}

impl<'a> MeshEdgebreakerTraversalPredictiveDecoder<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        start_face_decoder: RAnsBitDecoder<'a>,
        has_start_face_bits: bool,
        topology_split_data: Vec<TopologySplitEventData>,
        start_face_bits_legacy: Option<Vec<bool>>,
        direct_symbol_bits: Option<Vec<bool>>,
        prediction_decoder: RAnsBitDecoder<'a>,
        num_vertices: usize,
    ) -> Self {
        let split_event_remaining = topology_split_data.len();
        Self {
            num_vertices,
            vertex_valences: vec![0; num_vertices],
            last_symbol: -1,
            predicted_symbol: -1,
            prediction_decoder,
            topology_split_data,
            split_event_remaining,
            start_face_decoder,
            has_start_face_bits,
            start_face_bits_legacy,
            start_face_bits_legacy_index: 0,
            processed_connectivity_corners: Vec::new(),
            direct_symbol_bits,
            direct_symbol_bit_index: 0,
        }
    }

    /// Decode the next symbol directly from the main traversal stream: 1 bit for
    /// `C`, otherwise 3 bits (mirrors `MeshEdgeBreakerTraversalDecoder::DecodeSymbol`).
    /// Returns the internal symbol id, or `None` if the stream is exhausted.
    fn decode_direct_symbol(&mut self) -> Option<i32> {
        let bits = self.direct_symbol_bits.as_ref()?;
        let i = self.direct_symbol_bit_index;
        let first = *bits.get(i)?;
        let topology = if !first {
            self.direct_symbol_bit_index += 1;
            0u32
        } else {
            let b1 = *bits.get(i + 1)?;
            let b2 = *bits.get(i + 2)?;
            self.direct_symbol_bit_index += 3;
            1u32 | ((b1 as u32) << 1) | ((b2 as u32) << 2)
        };
        match topology {
            0 => Some(0),
            1 => Some(1),
            3 => Some(2),
            5 => Some(3),
            7 => Some(4),
            _ => None,
        }
    }

    fn checked_add_corner_vertex_valence(
        &mut self,
        corner_table: &CornerTable,
        corner: CornerIndex,
        delta: i32,
    ) -> bool {
        if corner == crate::geometry_indices::INVALID_CORNER_INDEX
            || corner.0 as usize >= corner_table.num_corners()
        {
            return false;
        }
        let vertex = corner_table.vertex(corner);
        let Some(valence) = self.vertex_valences.get_mut(vertex.0 as usize) else {
            return false;
        };
        *valence += delta;
        true
    }

    fn checked_corner_vertex_valence(
        &self,
        corner_table: &CornerTable,
        corner: CornerIndex,
    ) -> Option<i32> {
        if corner == crate::geometry_indices::INVALID_CORNER_INDEX
            || corner.0 as usize >= corner_table.num_corners()
        {
            return None;
        }
        let vertex = corner_table.vertex(corner);
        self.vertex_valences.get(vertex.0 as usize).copied()
    }
}

impl<'a> EdgebreakerTraversalDecoder for MeshEdgebreakerTraversalPredictiveDecoder<'a> {
    fn decode_symbol(&mut self) -> Result<u32, String> {
        // Use the prediction if we have one and the prediction bit confirms it.
        if self.predicted_symbol != -1 && self.prediction_decoder.decode_next_bit() {
            self.last_symbol = self.predicted_symbol;
            return Ok(self.last_symbol as u32);
        }
        // No prediction, or the symbol was mispredicted: decode it directly.
        self.last_symbol = self
            .decode_direct_symbol()
            .ok_or_else(|| "Edgebreaker predictive symbol stream exhausted".to_string())?;
        Ok(self.last_symbol as u32)
    }

    fn decode_start_face_configuration(&mut self) -> bool {
        if let Some(ref bits) = self.start_face_bits_legacy {
            let idx = self.start_face_bits_legacy_index;
            self.start_face_bits_legacy_index += 1;
            return bits.get(idx).copied().unwrap_or(true);
        }
        if self.has_start_face_bits {
            self.start_face_decoder.decode_next_bit()
        } else {
            true
        }
    }

    fn merge_vertices(&mut self, dest: VertexIndex, source: VertexIndex) {
        if (dest.0 as usize) < self.vertex_valences.len()
            && (source.0 as usize) < self.vertex_valences.len()
        {
            self.vertex_valences[dest.0 as usize] += self.vertex_valences[source.0 as usize];
        }
    }

    fn is_topology_split(&mut self, encoder_symbol_id: i32) -> Option<(EdgeFaceName, i32)> {
        if self.split_event_remaining > 0 {
            let event = &self.topology_split_data[self.split_event_remaining - 1];
            if event.source_symbol_id == encoder_symbol_id as u32 {
                self.split_event_remaining -= 1;
                return Some((event.source_edge, event.split_symbol_id as i32));
            } else if event.source_symbol_id > encoder_symbol_id as u32 {
                return Some((EdgeFaceName::LeftFaceEdge, -1));
            }
        }
        None
    }

    fn on_vertex_created(&mut self, vertex: VertexIndex, _symbol_id: i32, _corner_index: i32) {
        if (vertex.0 as usize) >= self.vertex_valences.len() {
            self.vertex_valences.resize((vertex.0 as usize) + 1, 0);
            self.num_vertices = self.vertex_valences.len();
        }
    }

    fn on_vertices_swapped(&mut self, _v1: VertexIndex, _v2: VertexIndex) {}

    fn on_start_face_decoded(&mut self, _corner: CornerIndex) {}

    fn on_split_symbol_decoded(&mut self, _corner: CornerIndex) {}

    fn new_active_corner_reached(&mut self, corner: CornerIndex, corner_table: &CornerTable) {
        if corner == crate::geometry_indices::INVALID_CORNER_INDEX
            || corner.0 as usize >= corner_table.num_corners()
        {
            self.predicted_symbol = -1;
            return;
        }

        // Update valences based on last_symbol (C=0, S=1, L=2, R=3, E=4).
        let next = corner_table.next(corner);
        let prev = corner_table.previous(corner);
        let updated = match self.last_symbol {
            0 | 1 => {
                self.checked_add_corner_vertex_valence(corner_table, next, 1)
                    && self.checked_add_corner_vertex_valence(corner_table, prev, 1)
            }
            3 => {
                self.checked_add_corner_vertex_valence(corner_table, corner, 1)
                    && self.checked_add_corner_vertex_valence(corner_table, next, 1)
                    && self.checked_add_corner_vertex_valence(corner_table, prev, 2)
            }
            2 => {
                self.checked_add_corner_vertex_valence(corner_table, corner, 1)
                    && self.checked_add_corner_vertex_valence(corner_table, next, 2)
                    && self.checked_add_corner_vertex_valence(corner_table, prev, 1)
            }
            4 => {
                self.checked_add_corner_vertex_valence(corner_table, corner, 2)
                    && self.checked_add_corner_vertex_valence(corner_table, next, 2)
                    && self.checked_add_corner_vertex_valence(corner_table, prev, 2)
            }
            _ => true,
        };
        if !updated {
            self.predicted_symbol = -1;
            return;
        }

        // Predict the next symbol from the pivot vertex valence. Only C and R
        // symbols seed a prediction.
        if self.last_symbol == SYMBOL_C || self.last_symbol == SYMBOL_R {
            match self.checked_corner_vertex_valence(corner_table, next) {
                Some(pivot_valence) if pivot_valence < 6 => self.predicted_symbol = SYMBOL_R,
                Some(_) => self.predicted_symbol = SYMBOL_C,
                None => self.predicted_symbol = -1,
            }
        } else {
            self.predicted_symbol = -1;
        }

        self.processed_connectivity_corners.push(corner.0);
    }
}
