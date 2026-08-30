//! Valence-based EdgeBreaker traversal decoder.
//!
//! [`MeshEdgebreakerTraversalValenceDecoder`] decodes EdgeBreaker symbols using
//! per-vertex valence context (Draco's higher-compression "valence" traversal),
//! adapting symbol probabilities to local mesh degree. Port of Draco's
//! `mesh_edgebreaker_traversal_valence_decoder.h`.

use crate::corner_table::CornerTable;
use crate::decoder_buffer::DecoderBuffer;
use crate::edgebreaker_connectivity_decoder::EdgebreakerTraversalDecoder;
use crate::geometry_indices::{CornerIndex, VertexIndex};
use crate::mesh_edgebreaker_shared::{EdgeFaceName, TopologySplitEventData};
use crate::rans_bit_decoder::RAnsBitDecoder;
use crate::status::{DracoError, Status};
use crate::symbol_encoding::decode_symbols;
use crate::symbol_encoding::SymbolEncodingOptions;

pub struct MeshEdgebreakerTraversalValenceDecoder<'a> {
    #[allow(dead_code)]
    corner_table: Option<&'a CornerTable>,
    num_vertices: usize,
    vertex_valences: Vec<i32>,
    last_symbol: i32,
    active_context: i32,
    min_valence: i32,
    max_valence: i32,
    context_symbols: Vec<Vec<u32>>,
    context_counters: Vec<i32>,
    topology_split_data: Vec<TopologySplitEventData>,
    split_event_remaining: usize,
    pub(crate) start_face_decoder: RAnsBitDecoder<'a>,
    pub(crate) has_start_face_bits: bool,
    pub(crate) start_face_bits_legacy: Option<Vec<bool>>,
    start_face_bits_legacy_index: usize,
    pub(crate) processed_connectivity_corners: Vec<u32>,
    /// Pre-1.0 (bitstream < 2.2) only: raw bits of the main traversal symbol
    /// stream, replayed when no valence context is active (component starts /
    /// mispredictions). `None` for modern streams, where that case is always E.
    direct_symbol_bits: Option<Vec<bool>>,
    direct_symbol_bit_index: usize,
}

impl<'a> MeshEdgebreakerTraversalValenceDecoder<'a> {
    pub fn new(
        start_face_decoder: RAnsBitDecoder<'a>,
        has_start_face_bits: bool,
        topology_split_data: Vec<TopologySplitEventData>,
        start_face_bits_legacy: Option<Vec<bool>>,
        direct_symbol_bits: Option<Vec<bool>>,
    ) -> Self {
        let split_event_remaining = topology_split_data.len();
        Self {
            corner_table: None,
            num_vertices: 0,
            vertex_valences: Vec::new(),
            last_symbol: -1,
            active_context: -1,
            min_valence: 2,
            max_valence: 7,
            context_symbols: Vec::new(),
            context_counters: Vec::new(),
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

    /// Decode the next "direct" EdgeBreaker symbol from the legacy main traversal
    /// stream (bitstream < 2.2). Each symbol is `1` bit for `C`, otherwise `3`
    /// bits (mirrors `MeshEdgeBreakerTraversalDecoder::DecodeSymbol`). Returns the
    /// internal symbol id (C=0, S=1, L=2, R=3, E=4), or `None` if there is no
    /// legacy stream or it is exhausted.
    fn decode_direct_symbol(&mut self) -> Option<i32> {
        let bits = self.direct_symbol_bits.as_ref()?;
        let i = self.direct_symbol_bit_index;
        let first = *bits.get(i)?;
        let topology = if !first {
            self.direct_symbol_bit_index += 1;
            0u32
        } else {
            // Two suffix bits, LSB first: suffix = b1 | (b2 << 1); topology = 1 | (suffix << 1).
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

    /// Initialize decoder contexts by reading varint counts and symbol streams from buffer.
    pub fn init_from_buffer(
        &mut self,
        in_buffer: &mut DecoderBuffer,
        num_vertices: usize,
        bitstream_version: u16,
        max_context_symbols: usize,
    ) -> Status {
        self.num_vertices = num_vertices;

        // Before bitstream 2.2, the valence stream prefixes its own split-symbol
        // count and a valence-mode byte, right before the context symbol streams
        // (current streams carry the split count in the connectivity header and
        // always use the 2..7 mode). C++ keeps this behind
        // DRACO_BACKWARDS_COMPATIBILITY_SUPPORTED; without the reads the buffer is
        // misaligned and the context decode below fails.
        #[cfg(feature = "legacy_bitstream_decode")]
        if bitstream_version < 0x0202 {
            // The split-symbol count is re-encoded in the valence stream here;
            // read it only to keep the buffer aligned. The caller already folded
            // it into `num_vertices` (max_num_vertices = encoded + split).
            let num_split_symbols = if bitstream_version < 0x0200 {
                in_buffer.decode_u32().map_err(|_| {
                    DracoError::buffer("Buffer ran out reading the legacy split-symbol count")
                })? as usize
            } else {
                in_buffer.decode_varint().map_err(|_| {
                    DracoError::buffer("Buffer ran out reading the legacy split-symbol count")
                })? as usize
            };
            if num_split_symbols >= self.num_vertices {
                return Err(DracoError::general(format!(
                    "Legacy valence stream claims {num_split_symbols} split symbols against {} vertices",
                    self.num_vertices
                )));
            }
            // Valence mode byte; only EDGEBREAKER_VALENCE_MODE_2_7 (0) is supported.
            match in_buffer.decode_u8() {
                Ok(0) => {}
                Ok(mode) => {
                    return Err(DracoError::unsupported_feature(format!(
                        "Valence mode {mode}: only mode 0 (valences 2..7) is defined"
                    )))
                }
                Err(_) => {
                    return Err(DracoError::buffer(
                        "Buffer ran out reading the legacy valence mode byte",
                    ))
                }
            }
        }
        #[cfg(not(feature = "legacy_bitstream_decode"))]
        let _ = bitstream_version;

        // Not sized from `num_vertices` directly -- that count is
        // `num_encoded_vertices + num_split_symbols`, and the first term is a
        // varint the header supplies on its own, so honouring it here would
        // cost four bytes per claimed vertex before a single one has been
        // decoded. `reserve_vertices` (called by the connectivity decoder,
        // after this) reserves against its own already-validated bound
        // instead. `on_vertex_created` grows the table as vertices actually
        // appear either way, and every read goes through `get`/`get_mut`, so a
        // vertex that was never created reads as absent rather than as
        // valence zero -- which is a refusal a table pre-sized from the raw
        // claim would swallow.
        self.vertex_valences.clear();

        self.min_valence = 2;
        self.max_valence = 7;
        let num_unique_valences = (self.max_valence - self.min_valence + 1) as usize;
        self.context_symbols = vec![Vec::new(); num_unique_valences];
        self.context_counters = vec![0; num_unique_valences];

        // For each context, read count and symbols
        let mut total_context_symbols = 0usize;
        for i in 0..num_unique_valences {
            // Read varint count
            let num_symbols = in_buffer.decode_varint().map_err(|_| {
                DracoError::buffer(format!(
                    "Buffer ran out reading the symbol count for valence context {i}"
                ))
            })? as usize;
            total_context_symbols = match total_context_symbols.checked_add(num_symbols) {
                Some(v) if v <= max_context_symbols => v,
                _ => {
                    return Err(DracoError::general(format!(
                        "Valence contexts claim more than {max_context_symbols} symbols in total, \
                         which is more than the connectivity has"
                    )))
                }
            };
            if num_symbols > 0 {
                // Not resized to `num_symbols` first: that count is the
                // stream's claim, and `decode_symbols` grows this as symbols
                // are decoded, so a claim the stream cannot meet costs one
                // small reservation rather than the whole context.
                let options = SymbolEncodingOptions::default();
                decode_symbols(
                    num_symbols,
                    1,
                    &options,
                    in_buffer,
                    &mut self.context_symbols[i],
                )
                .map_err(|err| err.context(format!("Valence context {i} of 6")))?;
                // Set counter to read from back
                self.context_counters[i] = num_symbols as i32;
            }
        }

        Ok(())
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

impl<'a> EdgebreakerTraversalDecoder for MeshEdgebreakerTraversalValenceDecoder<'a> {
    fn reserve_traversal_order(&mut self, faces: usize) {
        self.processed_connectivity_corners.reserve(faces);
    }

    fn reserve_vertices(&mut self, vertices: usize) {
        self.vertex_valences.reserve(vertices);
    }

    fn decode_symbol(&mut self) -> Result<u32, DracoError> {
        if self.active_context != -1 {
            let ctx = self.active_context as usize;
            let counter = self.context_counters.get_mut(ctx).ok_or_else(|| {
                DracoError::general("Invalid Edgebreaker valence context".to_string())
            })?;
            *counter -= 1;
            if *counter < 0 {
                return Err(DracoError::general(
                    "Edgebreaker valence context symbol stream exhausted".to_string(),
                ));
            }
            let symbol_id = *self
                .context_symbols
                .get(ctx)
                .and_then(|symbols| symbols.get(*counter as usize))
                .ok_or_else(|| {
                    DracoError::general(
                        "Edgebreaker valence context symbol stream exhausted".to_string(),
                    )
                })?;
            // symbol_id is EdgebreakerSymbol id (0..4). Validate and assign directly.
            if symbol_id > 4 {
                return Err(DracoError::general(format!(
                    "Invalid Edgebreaker valence symbol {symbol_id}"
                )));
            }
            self.last_symbol = symbol_id as i32;
        } else {
            // No active valence context (component start / misprediction). For
            // bitstream < 2.2 the symbol is taken from the main traversal stream;
            // modern streams always have E here (no legacy stream, so default 4).
            self.last_symbol = self.decode_direct_symbol().unwrap_or(4);
        }
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

    fn on_vertex_created(&mut self, vertex: VertexIndex, symbol_id: i32, _corner_index: i32) {
        // When vertex is created, set its initial valence to 0
        if (vertex.0 as usize) >= self.vertex_valences.len() {
            self.vertex_valences.resize((vertex.0 as usize) + 1, 0);
            self.num_vertices = self.vertex_valences.len();
        }
        // For E, L, R, etc, the actual valence update happens when new_active_corner_reached is called.
        let _ = symbol_id;
    }

    fn on_vertices_swapped(&mut self, _v1: VertexIndex, _v2: VertexIndex) {}

    fn on_start_face_decoded(&mut self, _corner: CornerIndex) {}

    fn on_split_symbol_decoded(&mut self, _corner: CornerIndex) {
        // no-op
    }

    fn new_active_corner_reached(&mut self, corner: CornerIndex, corner_table: &CornerTable) {
        if corner == crate::geometry_indices::INVALID_CORNER_INDEX
            || corner.0 as usize >= corner_table.num_corners()
        {
            self.active_context = -1;
            return;
        }

        // Update valences based on last_symbol
        // Rust uses symbol_id values (0-4) not C++ TOPOLOGY bit patterns (0,1,3,5,7)
        // Mapping: C=0, S=1, L=2, R=3, E=4
        let next = corner_table.next(corner);
        let prev = corner_table.previous(corner);
        let updated = match self.last_symbol {
            0 | 1 => {
                // Center (C) or Split (S)
                self.checked_add_corner_vertex_valence(corner_table, next, 1)
                    && self.checked_add_corner_vertex_valence(corner_table, prev, 1)
            }
            3 => {
                // Right (R)
                self.checked_add_corner_vertex_valence(corner_table, corner, 1)
                    && self.checked_add_corner_vertex_valence(corner_table, next, 1)
                    && self.checked_add_corner_vertex_valence(corner_table, prev, 2)
            }
            2 => {
                // Left (L)
                self.checked_add_corner_vertex_valence(corner_table, corner, 1)
                    && self.checked_add_corner_vertex_valence(corner_table, next, 2)
                    && self.checked_add_corner_vertex_valence(corner_table, prev, 1)
            }
            4 => {
                // End (E)
                self.checked_add_corner_vertex_valence(corner_table, corner, 2)
                    && self.checked_add_corner_vertex_valence(corner_table, next, 2)
                    && self.checked_add_corner_vertex_valence(corner_table, prev, 2)
            }
            _ => true,
        };
        if !updated {
            self.active_context = -1;
            return;
        }

        let Some(active_valence) = self.checked_corner_vertex_valence(corner_table, next) else {
            self.active_context = -1;
            return;
        };
        let clamped = if active_valence < self.min_valence {
            self.min_valence
        } else if active_valence > self.max_valence {
            self.max_valence
        } else {
            active_valence
        };
        self.active_context = clamped - self.min_valence;

        // Record processed connectivity corner (like InternalTraversalDecoder)
        self.processed_connectivity_corners.push(corner.0);
    }
}

#[cfg(all(test, feature = "legacy_bitstream_decode"))]
mod legacy_tests {
    use super::*;

    fn append_varint(bytes: &mut Vec<u8>, mut value: u64) {
        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    #[test]
    fn rejects_oversized_legacy_context_symbol_count_before_resize() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u32.to_le_bytes()); // legacy split-symbol count
        bytes.push(0); // valence mode 2..7
        append_varint(&mut bytes, u32::MAX as u64); // first context symbol count

        let mut buffer = DecoderBuffer::new(&bytes);
        buffer.set_version(1, 1);
        let mut decoder = MeshEdgebreakerTraversalValenceDecoder::new(
            RAnsBitDecoder::new(),
            false,
            Vec::new(),
            None,
            None,
        );

        let err = decoder
            .init_from_buffer(&mut buffer, 4, 0x0101, 4)
            .unwrap_err();
        assert!(err.message().contains('4'), "{err}");
    }

    #[test]
    fn rejects_legacy_split_symbol_count_at_or_above_vertex_count() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&4u32.to_le_bytes()); // legacy split-symbol count
        bytes.push(0); // would be valid valence mode if the count were accepted

        let mut buffer = DecoderBuffer::new(&bytes);
        buffer.set_version(1, 1);
        let mut decoder = MeshEdgebreakerTraversalValenceDecoder::new(
            RAnsBitDecoder::new(),
            false,
            Vec::new(),
            None,
            None,
        );

        // Both numbers appear: which count outran which is the refusal.
        let err = decoder
            .init_from_buffer(&mut buffer, 4, 0x0101, 4)
            .unwrap_err();
        assert!(err.message().contains("4 split symbols"), "{err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry_indices::VertexIndex;

    #[test]
    fn valence_active_corner_rejects_out_of_range_corner_without_panic() {
        let start_face_decoder = RAnsBitDecoder::new();
        let mut decoder = MeshEdgebreakerTraversalValenceDecoder::new(
            start_face_decoder,
            false,
            Vec::new(),
            None,
            None,
        );
        decoder.vertex_valences = vec![0; 3];
        decoder.last_symbol = 4;

        let mut corner_table = CornerTable::new(1);
        assert!(corner_table.init(&[[VertexIndex(0), VertexIndex(1), VertexIndex(2),]]));

        decoder.new_active_corner_reached(CornerIndex(3), &corner_table);

        assert_eq!(decoder.active_context, -1);
        assert!(decoder.processed_connectivity_corners.is_empty());
    }

    #[test]
    fn valence_active_corner_rejects_out_of_range_vertex_without_panic() {
        let start_face_decoder = RAnsBitDecoder::new();
        let mut decoder = MeshEdgebreakerTraversalValenceDecoder::new(
            start_face_decoder,
            false,
            Vec::new(),
            None,
            None,
        );
        decoder.vertex_valences = vec![0; 3];
        decoder.last_symbol = 4;

        let mut corner_table = CornerTable::new(1);
        assert!(corner_table.init(&[[VertexIndex(0), VertexIndex(99), VertexIndex(2),]]));

        decoder.new_active_corner_reached(CornerIndex(0), &corner_table);

        assert_eq!(decoder.active_context, -1);
        assert!(decoder.processed_connectivity_corners.is_empty());
    }
}
