use crate::corner_table::CornerTable;
use crate::decoder_buffer::DecoderBuffer;
use crate::edgebreaker_connectivity_decoder::EdgebreakerTraversalDecoder;
use crate::geometry_indices::{CornerIndex, VertexIndex};
use crate::mesh_edgebreaker_shared::{EdgeFaceName, TopologySplitEventData};
use crate::rans_bit_decoder::RAnsBitDecoder;
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
}

impl<'a> MeshEdgebreakerTraversalValenceDecoder<'a> {
    pub fn new(
        start_face_decoder: RAnsBitDecoder<'a>,
        has_start_face_bits: bool,
        topology_split_data: Vec<TopologySplitEventData>,
        start_face_bits_legacy: Option<Vec<bool>>,
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
        }
    }

    /// Initialize decoder contexts by reading varint counts and symbol streams from buffer.
    pub fn init_from_buffer(&mut self, in_buffer: &mut DecoderBuffer, num_vertices: usize) -> bool {
        self.num_vertices = num_vertices;
        self.vertex_valences.resize(self.num_vertices, 0);

        self.min_valence = 2;
        self.max_valence = 7;
        let num_unique_valences = (self.max_valence - self.min_valence + 1) as usize;
        self.context_symbols = vec![Vec::new(); num_unique_valences];
        self.context_counters = vec![0; num_unique_valences];

        // For each context, read count and symbols
        for i in 0..num_unique_valences {
            // Read varint count
            let num_symbols = match in_buffer.decode_varint() {
                Ok(v) => v as usize,
                Err(_) => return false,
            };
            if num_symbols > 0 {
                self.context_symbols[i].resize(num_symbols, 0);
                let options = SymbolEncodingOptions::default();
                if !decode_symbols(
                    num_symbols,
                    1,
                    &options,
                    in_buffer,
                    &mut self.context_symbols[i],
                ) {
                    return false;
                }
                // Set counter to read from back
                self.context_counters[i] = num_symbols as i32;
            }
        }

        true
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
    fn decode_symbol(&mut self) -> Result<u32, String> {
        if self.active_context != -1 {
            let ctx = self.active_context as usize;
            let counter = self
                .context_counters
                .get_mut(ctx)
                .ok_or_else(|| "Invalid Edgebreaker valence context".to_string())?;
            *counter -= 1;
            if *counter < 0 {
                return Err("Edgebreaker valence context symbol stream exhausted".to_string());
            }
            let symbol_id = *self
                .context_symbols
                .get(ctx)
                .and_then(|symbols| symbols.get(*counter as usize))
                .ok_or_else(|| "Edgebreaker valence context symbol stream exhausted".to_string())?;
            // symbol_id is EdgebreakerSymbol id (0..4). Validate and assign directly.
            if symbol_id > 4 {
                return Err(format!("Invalid Edgebreaker valence symbol {symbol_id}"));
            }
            self.last_symbol = symbol_id as i32;
        } else {
            // If no context, for new sequence the first symbol must be E (End = 4)
            self.last_symbol = 4;
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
        self.active_context = (clamped - self.min_valence) as i32;

        // Record processed connectivity corner (like InternalTraversalDecoder)
        self.processed_connectivity_corners.push(corner.0);
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
