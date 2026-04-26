// Copyright 2016 The Draco Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::corner_table::CornerTable;
use crate::encoder_buffer::EncoderBuffer;
use crate::geometry_indices::{CornerIndex, VertexIndex, INVALID_CORNER_INDEX};
use crate::mesh_edgebreaker_shared::EdgebreakerSymbol;

pub struct MeshEdgebreakerTraversalValenceEncoder {
    vertex_valences: Vec<i32>,
    corner_to_vertex_map: Vec<VertexIndex>,
    prev_symbol: i32,
    last_corner: CornerIndex,
    num_symbols: usize,
    min_valence: i32,
    max_valence: i32,
    context_symbols: Vec<Vec<u32>>,
}

impl MeshEdgebreakerTraversalValenceEncoder {
    pub fn new() -> Self {
        Self {
            vertex_valences: Vec::new(),
            corner_to_vertex_map: Vec::new(),
            prev_symbol: -1,
            last_corner: INVALID_CORNER_INDEX,
            num_symbols: 0,
            min_valence: 2,
            max_valence: 7,
            context_symbols: Vec::new(),
        }
    }

    pub fn init(&mut self, corner_table: &CornerTable) {
        self.min_valence = 2;
        self.max_valence = 7;

        // Initialize valences of all vertices.
        self.vertex_valences.resize(corner_table.num_vertices(), 0);
        for i in 0..corner_table.num_vertices() {
            self.vertex_valences[i] = corner_table.valence(VertexIndex(i as u32));
        }

        // Replicate the corner to vertex map from the corner table.
        self.corner_to_vertex_map
            .resize(corner_table.num_corners(), VertexIndex(0));
        for i in 0..corner_table.num_corners() {
            self.corner_to_vertex_map[i] = corner_table.vertex(CornerIndex(i as u32));
        }

        let num_unique_valences = (self.max_valence - self.min_valence + 1) as usize;
        self.context_symbols = vec![Vec::new(); num_unique_valences];
    }

    pub fn new_corner_reached(&mut self, corner: CornerIndex) {
        self.last_corner = corner;
    }

    pub fn encode_symbol(
        &mut self,
        symbol: EdgebreakerSymbol,
        corner_table: &CornerTable,
        visited_faces: &[bool],
    ) {
        self.num_symbols += 1;

        // C++:
        // const CornerIndex next = corner_table_->Next(last_corner_);
        // const CornerIndex prev = corner_table_->Previous(last_corner_);
        let next = corner_table.next(self.last_corner);
        let prev = corner_table.previous(self.last_corner);

        // Get valence on the tip corner of the active edge
        let active_vertex_idx = self.corner_to_vertex_map[next.0 as usize].0 as usize;
        let active_valence = self.vertex_valences[active_vertex_idx];

        match symbol {
            EdgebreakerSymbol::Center | EdgebreakerSymbol::Split => {
                // TOPOLOGY_C, TOPOLOGY_S
                self.vertex_valences[self.corner_to_vertex_map[next.0 as usize].0 as usize] -= 1;
                self.vertex_valences[self.corner_to_vertex_map[prev.0 as usize].0 as usize] -= 1;

                if symbol == EdgebreakerSymbol::Split {
                    // Whenever we reach a split symbol, we need to split the vertex into
                    // two and attach all corners on the left and right sides of the split
                    // vertex to the respective vertices.

                    // Count left faces
                    let mut num_left_faces = 0;
                    let mut act_c = corner_table.opposite(prev);
                    while act_c != INVALID_CORNER_INDEX {
                        if visited_faces[corner_table.face(act_c).0 as usize] {
                            break;
                        }
                        num_left_faces += 1;
                        act_c = corner_table.opposite(corner_table.next(act_c));
                    }

                    let last_corner_v_idx =
                        self.corner_to_vertex_map[self.last_corner.0 as usize].0 as usize;
                    self.vertex_valences[last_corner_v_idx] = num_left_faces + 1;

                    // Create new vertex for right side
                    let new_vert_id = VertexIndex(self.vertex_valences.len() as u32);
                    let mut num_right_faces = 0;

                    act_c = corner_table.opposite(next);
                    while act_c != INVALID_CORNER_INDEX {
                        if visited_faces[corner_table.face(act_c).0 as usize] {
                            break;
                        }
                        num_right_faces += 1;
                        // Map corners on the right side to the newly created vertex.
                        // map_[Next(act_c)] = new_vert_id
                        let next_act_c = corner_table.next(act_c);
                        self.corner_to_vertex_map[next_act_c.0 as usize] = new_vert_id;

                        act_c = corner_table.opposite(corner_table.previous(act_c));
                    }
                    self.vertex_valences.push(num_right_faces + 1);
                }
            }
            EdgebreakerSymbol::Right => {
                // TOPOLOGY_R
                self.vertex_valences
                    [self.corner_to_vertex_map[self.last_corner.0 as usize].0 as usize] -= 1;
                self.vertex_valences[self.corner_to_vertex_map[next.0 as usize].0 as usize] -= 1;
                self.vertex_valences[self.corner_to_vertex_map[prev.0 as usize].0 as usize] -= 2;
            }
            EdgebreakerSymbol::Left => {
                // TOPOLOGY_L
                self.vertex_valences
                    [self.corner_to_vertex_map[self.last_corner.0 as usize].0 as usize] -= 1;
                self.vertex_valences[self.corner_to_vertex_map[next.0 as usize].0 as usize] -= 2;
                self.vertex_valences[self.corner_to_vertex_map[prev.0 as usize].0 as usize] -= 1;
            }
            EdgebreakerSymbol::End => {
                // TOPOLOGY_E
                self.vertex_valences
                    [self.corner_to_vertex_map[self.last_corner.0 as usize].0 as usize] -= 2;
                self.vertex_valences[self.corner_to_vertex_map[next.0 as usize].0 as usize] -= 2;
                self.vertex_valences[self.corner_to_vertex_map[prev.0 as usize].0 as usize] -= 2;
            }
            _ => {} // Hole?
        }

        if self.prev_symbol != -1 {
            let clamped_valence = if active_valence < self.min_valence {
                self.min_valence
            } else if active_valence > self.max_valence {
                self.max_valence
            } else {
                active_valence
            };

            let context = (clamped_valence - self.min_valence) as usize;
            let sym_id = self.prev_symbol as u32;
            self.context_symbols[context].push(sym_id);
        }

        self.prev_symbol = symbol as i32;
    }

    pub fn num_encoded_symbols(&self) -> usize {
        self.num_symbols
    }

    pub fn done(&self, out_buffer: &mut EncoderBuffer, compression_level: i32) {
        // Store the contexts.
        for symbols in &self.context_symbols {
            out_buffer.encode_varint(symbols.len() as u64);
            if !symbols.is_empty() {
                // Use standard raw symbol encoding.
                // Ideally we should use the compression level from options,
                // but for now we default to some reasonable value or pass it in.
                // C++ uses default options for this specific call usually.
                // Actually, C++ `EncodeSymbols` uses `options` if passed, or default.
                // `MeshEdgebreakerTraversalValenceEncoder` doesn't seem to set distinct options.
                // We'll use a default compression level (e.g. 7) or pass it in.
                // Let's assume we can change signature of done later if needed.
                let options = crate::symbol_encoding::SymbolEncodingOptions {
                    compression_level: compression_level,
                };
                if !crate::symbol_encoding::encode_symbols(symbols, 1, &options, out_buffer) {
                    // Handle error? For now print to stderr
                    eprintln!("Error encoding valence symbols");
                }
            }
        }
    }
}
