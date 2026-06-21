//! Predictive EdgeBreaker traversal encoder (legacy "type 1" traversal).
//!
//! Encode-side counterpart of the predictive traversal decoder, and the default
//! connectivity encoder in Draco 0.9.1 and earlier. It maintains per-vertex
//! valences of the not-yet-encoded mesh and predicts the symbol preceding the
//! one being encoded: after a `C` or `R` symbol it predicts `R` when the pivot
//! vertex valence is `< 6`, otherwise `C`. Correct predictions cost a single
//! prediction bit; mispredicted and unpredicted symbols are stored explicitly in
//! the main symbol stream. Replaced by the valence traversal in Draco 0.10.0.
//! Port of Draco's `mesh_edgebreaker_traversal_predictive_encoder.h`.

use crate::corner_table::CornerTable;
use crate::encoder_buffer::EncoderBuffer;
use crate::geometry_indices::CornerIndex;
use crate::mesh_edgebreaker_shared::EdgebreakerSymbol;
use crate::rans_bit_encoder::RAnsBitEncoder;

// Internal EdgeBreaker symbol ids (match the encoder): C=0, S=1, L=2, R=3, E=4.
const SYMBOL_C: i32 = 0;
const SYMBOL_R: i32 = 3;
// A value that is never a real symbol id (0..4): forces a misprediction, matching
// C++ TOPOLOGY_INVALID for split (negative-valence) vertices.
const INVALID_SYMBOL: i32 = 99;

#[derive(Default)]
pub struct MeshEdgebreakerTraversalPredictiveEncoder {
    vertex_valences: Vec<i32>,
    prev_symbol: i32,
    num_split_symbols: u32,
    /// Whether each prediction was correct, in encode order.
    predictions: Vec<bool>,
    /// Mispredicted / unpredicted symbols, in encode order, for the main stream.
    stored_symbols: Vec<u32>,
}

impl MeshEdgebreakerTraversalPredictiveEncoder {
    pub fn new() -> Self {
        Self {
            vertex_valences: Vec::new(),
            prev_symbol: -1,
            num_split_symbols: 0,
            predictions: Vec::new(),
            stored_symbols: Vec::new(),
        }
    }

    /// Initialize per-vertex valences from the (fully built) corner table.
    pub fn init(&mut self, corner_table: &CornerTable) {
        self.vertex_valences = (0..corner_table.num_vertices())
            .map(|v| corner_table.valence(crate::geometry_indices::VertexIndex(v as u32)))
            .collect();
    }

    fn predict(&self, pivot: usize) -> i32 {
        match self.vertex_valences.get(pivot).copied() {
            Some(v) if v < 0 => INVALID_SYMBOL,
            Some(v) if v < 6 => SYMBOL_R,
            Some(_) => SYMBOL_C,
            None => INVALID_SYMBOL,
        }
    }

    /// Process one symbol in traversal order (`last_corner` is the corner the
    /// symbol was emitted at). Updates valences, records the prediction for the
    /// preceding symbol, and stores that symbol explicitly when mispredicted.
    pub fn encode_symbol(
        &mut self,
        symbol: u32,
        corner_table: &CornerTable,
        last_corner: CornerIndex,
    ) {
        let next_v = corner_table.vertex(corner_table.next(last_corner)).0 as usize;
        let prev_v = corner_table.vertex(corner_table.previous(last_corner)).0 as usize;
        let corner_v = corner_table.vertex(last_corner).0 as usize;

        let mut predicted_symbol: i32 = -1;
        match symbol {
            0 => {
                // C
                predicted_symbol = self.predict(next_v);
                self.add_valence(next_v, -1);
                self.add_valence(prev_v, -1);
            }
            1 => {
                // S
                self.add_valence(next_v, -1);
                self.add_valence(prev_v, -1);
                if let Some(v) = self.vertex_valences.get_mut(corner_v) {
                    *v = -1;
                }
                self.num_split_symbols += 1;
            }
            3 => {
                // R
                predicted_symbol = self.predict(next_v);
                self.add_valence(corner_v, -1);
                self.add_valence(next_v, -1);
                self.add_valence(prev_v, -2);
            }
            2 => {
                // L
                self.add_valence(corner_v, -1);
                self.add_valence(next_v, -2);
                self.add_valence(prev_v, -1);
            }
            4 => {
                // E
                self.add_valence(corner_v, -2);
                self.add_valence(next_v, -2);
                self.add_valence(prev_v, -2);
            }
            _ => {}
        }

        let mut store_prev = true;
        if predicted_symbol != -1 {
            if predicted_symbol == self.prev_symbol {
                self.predictions.push(true);
                store_prev = false;
            } else if self.prev_symbol != -1 {
                self.predictions.push(false);
            }
        }
        if store_prev && self.prev_symbol != -1 {
            self.stored_symbols.push(self.prev_symbol as u32);
        }
        self.prev_symbol = symbol as i32;
    }

    fn add_valence(&mut self, vertex: usize, delta: i32) {
        if let Some(v) = self.vertex_valences.get_mut(vertex) {
            *v += delta;
        }
    }

    /// Flush the final symbol into the main stream (it has no following symbol to
    /// predict it). Call once after the last [`encode_symbol`].
    pub fn finish(&mut self) {
        if self.prev_symbol != -1 {
            self.stored_symbols.push(self.prev_symbol as u32);
            self.prev_symbol = -1;
        }
    }

    pub fn num_split_symbols(&self) -> u32 {
        self.num_split_symbols
    }

    /// The main-stream symbols (mispredicted / unpredicted), in encode order. The
    /// caller emits them reversed, like the standard traversal symbol stream.
    pub fn stored_symbols(&self) -> &[u32] {
        &self.stored_symbols
    }

    /// Emit the binary prediction stream. The bits are stored in reverse so the
    /// decoder, which traverses the symbols in the opposite order, reads them LIFO.
    pub fn encode_predictions(&self, out_buffer: &mut EncoderBuffer) {
        let mut encoder = RAnsBitEncoder::new();
        encoder.start_encoding();
        for &pred in self.predictions.iter().rev() {
            encoder.encode_bit(pred);
        }
        encoder.end_encoding(out_buffer);
    }
}

/// Maps an internal symbol id to its EdgeBreaker topology bit pattern
/// (bit count, value) for the raw main symbol stream.
pub fn symbol_topology_bits(symbol: u32) -> (u32, u32) {
    match EdgebreakerSymbol::from(symbol) {
        EdgebreakerSymbol::Center => (1, 0),
        EdgebreakerSymbol::Split => (3, 1),
        EdgebreakerSymbol::Left => (3, 3),
        EdgebreakerSymbol::Right => (3, 5),
        EdgebreakerSymbol::End | EdgebreakerSymbol::Hole => (3, 7),
    }
}
