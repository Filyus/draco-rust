//! EdgeBreaker traversal-decoder trait.
//!
//! [`EdgebreakerTraversalDecoder`] is the interface the EdgeBreaker connectivity
//! decoder uses to pull symbols and side-channel bits, letting the standard and
//! valence traversals share the same connectivity-reconstruction code. Port of
//! Draco's `mesh_edgebreaker_traversal_decoder.h`.

use crate::corner_table::CornerTable;
use crate::geometry_indices::{
    CornerIndex, FaceIndex, VertexIndex, INVALID_CORNER_INDEX, INVALID_VERTEX_INDEX,
};
use crate::mesh_edgebreaker_shared::EdgeFaceName;
use crate::status::DracoError;
use std::collections::HashMap;

pub trait EdgebreakerTraversalDecoder {
    fn decode_symbol(&mut self) -> Result<u32, DracoError>;
    fn decode_start_face_configuration(&mut self) -> bool;
    fn merge_vertices(&mut self, p: VertexIndex, n: VertexIndex);
    fn is_topology_split(&mut self, encoder_symbol_id: i32) -> Option<(EdgeFaceName, i32)>;
    fn on_vertex_created(&mut self, vertex: VertexIndex, symbol_id: i32, corner_index: i32);
    fn on_vertices_swapped(&mut self, v1: VertexIndex, v2: VertexIndex);
    fn on_start_face_decoded(&mut self, corner: CornerIndex);
    fn on_split_symbol_decoded(&mut self, _corner: CornerIndex) {}

    // Matches C++ traversal_decoder_.NewActiveCornerReached(active_corner_stack.back()).
    // Called after each decoded symbol/face to record the traversal order.
    fn new_active_corner_reached(&mut self, _corner: CornerIndex, _corner_table: &CornerTable) {}

    /// Reserves the traversal-order record for `faces` entries.
    ///
    /// One corner is recorded per decoded face, so a decoder that keeps that
    /// record grows a vector to the face count by doubling: seven allocations
    /// totalling 1.4 MB to reach 271 KB on a 69k-face mesh, each copying what
    /// came before. The bound is the caller's -- the same input-capped face
    /// count the corner table is reserved with -- and a decoder that records
    /// nothing ignores it.
    fn reserve_traversal_order(&mut self, _faces: usize) {}

    /// Reserves per-vertex decoder state for `vertices` entries.
    ///
    /// The predictive and valence traversal decoders each grow a per-vertex
    /// valence table by one element per newly created vertex, deliberately
    /// left unsized against the bitstream's own vertex count (an unvalidated
    /// header claim -- see each decoder's `vertex_valences` field for why).
    /// `vertices` here is the caller's already-validated bound, the same one
    /// the corner table and `is_vert_hole` are reserved with, so honouring it
    /// costs nothing a hostile stream could not already cost through normal
    /// decode. A decoder with no such table ignores it.
    fn reserve_vertices(&mut self, _vertices: usize) {}
}

pub struct EdgebreakerConnectivityDecoder {
    pub corner_table: CornerTable,
    pub is_vert_hole: Vec<bool>,
    /// Face and vertex counts the bitstream declared. They bound the decode,
    /// but nothing is allocated for them up front -- see [`Self::try_new`].
    declared_num_faces: i32,
    max_num_vertices: usize,
    active_corner_stack: Vec<CornerIndex>,
    topology_split_active_corners: HashMap<i32, CornerIndex>,
    invalid_vertices: Vec<VertexIndex>,
}

impl EdgebreakerConnectivityDecoder {
    pub fn new(num_faces: i32, max_num_vertices: i32) -> Self {
        Self {
            corner_table: CornerTable::new(0),
            is_vert_hole: Vec::new(),
            declared_num_faces: num_faces,
            max_num_vertices: max_num_vertices.max(0) as usize,
            active_corner_stack: Vec::new(),
            topology_split_active_corners: HashMap::new(),
            invalid_vertices: Vec::new(),
        }
    }

    /// Fallible constructor mirroring [`EdgebreakerConnectivityDecoder::new`].
    ///
    /// Neither table is sized from `num_faces` / `max_num_vertices`: those are
    /// counts the bitstream asserts, and honouring them before anything has
    /// checked them lets a few hundred bytes of malformed input ask for
    /// gigabytes. Both grow as faces and vertices are actually decoded, so the
    /// memory a stream costs is proportional to the geometry it really carries,
    /// and the declared counts are kept only to bound the decode.
    pub fn try_new(
        num_faces: i32,
        max_num_vertices: i32,
    ) -> Result<Self, crate::status::DracoError> {
        Ok(Self::new(num_faces, max_num_vertices))
    }

    /// Marks `vertex` as not lying on a hole, growing the table to reach it.
    ///
    /// Entries are `true` until written, which is what sizing the table up
    /// front from the declared vertex count used to give for free.
    /// One bounds check on the common path, not two: reaching the entry
    /// through `get_mut` both proves the table already covers the vertex and
    /// yields the slot, where testing the length and then indexing left the
    /// indexing's own check standing behind the test. Growth is the cold arm.
    #[inline]
    fn mark_vert_not_hole(&mut self, vertex: VertexIndex, context: &str) -> Result<(), DracoError> {
        let index = self.vertex_index(vertex, context)?;
        match self.is_vert_hole.get_mut(index) {
            Some(slot) => *slot = false,
            None => {
                // Growth is not the cold arm it looks like: a vertex is
                // discovered here before it is anywhere else, so the table
                // reaches its end once per vertex, and `resize(index + 1)`
                // reached that through `extend_with`'s element-at-a-time loop
                // and its set-length-on-drop guard -- to fill a `true` this
                // line then overwrites. The gap before this vertex, if there
                // is one, is what `resize` is for; the vertex itself is a
                // push. Capacity is already reserved by the caller.
                self.is_vert_hole.resize(index, true);
                self.is_vert_hole.push(false);
            }
        }
        Ok(())
    }

    pub fn decode_connectivity<T: EdgebreakerTraversalDecoder>(
        &mut self,
        num_symbols: i32,
        traversal_decoder: &mut T,
        remove_invalid_vertices: bool,
        input_face_bound: usize,
    ) -> Result<i32, DracoError> {
        let max_num_vertices = self.max_num_vertices as i32;
        let mut num_faces = 0;

        // One allocation for the table instead of a realloc every time it
        // doubles, and one fill instead of one per face. How many faces are
        // worth this is the caller's decision: it holds the header's face count
        // and the size of what is left in the buffer, and hands over whichever
        // is smaller. Filling that bound writes no entry the face-at-a-time
        // append would not have written -- the loop below builds exactly
        // `declared_num_faces` faces or fails -- and it leaves the appends'
        // per-face capacity test, pointer reload and length update out.
        //
        // Past this point the table is longer than the faces built so far, and
        // the one thing that reads differently is `num_corners()`: every
        // accessor answers the invalid sentinel for a corner of an unbuilt
        // face, which is what it answered when that corner was past the end.
        // The truncation after the loop puts the count back.
        self.corner_table.try_fill_faces(input_face_bound)?;
        let filled_faces = input_face_bound;
        // vertex_corners grows by one element per newly discovered vertex
        // (set_left_most_corner's resize), unlike the other two tables above,
        // which is why it needs its own reservation rather than falling out of
        // try_reserve_faces. self.max_num_vertices already comes from a
        // header count checked against 3 * num_faces, not the stream size --
        // the face bound is what keeps that from being honoured past what
        // the buffer could actually describe, and the corner table already
        // trusts that bound for the same reason. Three vertices per face is
        // the geometric ceiling, not a dial: capping at the face bound alone
        // clipped a strip's V = F + 2 by two vertices and paid a doubling
        // reallocation of the whole vertex table for them.
        let vertex_bound = self
            .max_num_vertices
            .min(input_face_bound.saturating_mul(3));
        self.corner_table.try_reserve_vertices(vertex_bound)?;
        // is_vert_hole is indexed by vertex and grows one at a time from
        // mark_vert_not_hole, the same as vertex_corners above, so it needs
        // the same upfront reservation for the same reason.
        self.is_vert_hole
            .try_reserve(vertex_bound.saturating_sub(self.is_vert_hole.len()))
            .map_err(|_| DracoError::general("Failed to allocate vertex-hole table".to_string()))?;
        traversal_decoder.reserve_traversal_order(input_face_bound);
        traversal_decoder.reserve_vertices(vertex_bound);

        for symbol_id in 0..num_symbols {
            let face = FaceIndex(num_faces as u32);
            num_faces += 1;
            // Faces are created strictly in order, and every corner written
            // below belongs either to this face or to one already built, so
            // growing a face at a time always suffices -- and the fill above
            // has already covered every face the buffer could describe, so
            // this arm is what a face count capped by the stream size leaves
            // over, not the common path.
            if num_faces as usize > filled_faces {
                self.corner_table
                    .try_push_face(self.declared_num_faces.max(0) as usize)?;
            }

            let mut check_topology_split = false;
            let symbol = traversal_decoder.decode_symbol()?;

            // Internal symbol mapping (see `EdgebreakerSymbol`):
            //   Center = 0, Split = 1, Left = 2, Right = 3, End = 4
            if symbol == 0 {
                // TOPOLOGY_C
                if self.active_corner_stack.is_empty() {
                    return Err(DracoError::general(
                        "active_corner_stack empty in TOPOLOGY_C".to_string(),
                    ));
                }

                let corner_a = self.active_corner("TOPOLOGY_C")?;
                let vertex_x = self.corner_table.vertex_after(corner_a);
                let corner_b = self
                    .corner_table
                    .next(self.corner_table.left_most_corner(vertex_x));

                if corner_a == corner_b {
                    return Err(DracoError::general(
                        "corner_a == corner_b in TOPOLOGY_C".to_string(),
                    ));
                }
                if self.corner_table.opposite(corner_a) != INVALID_CORNER_INDEX
                    || self.corner_table.opposite(corner_b) != INVALID_CORNER_INDEX
                {
                    return Err(DracoError::general(
                        "Edge already opposite in TOPOLOGY_C".to_string(),
                    ));
                }

                let corner = CornerIndex(3 * face.0);
                self.set_opposite_corners(corner_a, corner + 1)?;
                self.set_opposite_corners(corner_b, corner + 2)?;

                let vert_a_prev = self.corner_table.vertex_before(corner_a);
                let vert_b_next = self.corner_table.vertex_after(corner_b);
                if vertex_x == vert_a_prev || vertex_x == vert_b_next {
                    return Err(DracoError::general(
                        "Degenerate face in TOPOLOGY_C".to_string(),
                    ));
                }

                self.corner_table.map_corner_to_vertex(corner, vertex_x);
                self.corner_table
                    .map_corner_to_vertex(corner + 1, vert_b_next);
                self.corner_table
                    .map_corner_to_vertex(corner + 2, vert_a_prev);
                self.corner_table
                    .set_left_most_corner(vert_a_prev, corner + 2);

                self.mark_vert_not_hole(vertex_x, "TOPOLOGY_C")?;
                self.replace_active_corner(corner, "TOPOLOGY_C")?;
            } else if symbol == 3 || symbol == 2 {
                // Right or Left
                // Symbol 3 = Right, Symbol 2 = Left.
                if self.active_corner_stack.is_empty() {
                    return Err(DracoError::general(
                        "active_corner_stack empty in TOPOLOGY_R/L".to_string(),
                    ));
                }
                let corner_a = self.active_corner("TOPOLOGY_R/L")?;
                if self.corner_table.opposite(corner_a) != INVALID_CORNER_INDEX {
                    return Err(DracoError::general(
                        "Edge already opposite in TOPOLOGY_R/L".to_string(),
                    ));
                }

                // This matches C++ `MeshEdgebreakerDecoderImpl::DecodeConnectivity()`:
                // - Right: opp_corner = corner + 2, corner_l = corner + 1, corner_r = corner
                // - Left:  opp_corner = corner + 1, corner_l = corner,     corner_r = corner + 2
                let corner = CornerIndex(3 * face.0);
                let (opp_corner, corner_l, corner_r) = if symbol == 3 {
                    // Right
                    (corner + 2, corner + 1, corner)
                } else {
                    // Left
                    (corner + 1, corner, corner + 2)
                };

                self.set_opposite_corners(opp_corner, corner_a)?;
                let new_vert_index = self.corner_table.add_new_vertex();
                traversal_decoder.on_vertex_created(new_vert_index, symbol_id, opp_corner.0 as i32);

                if self.corner_table.num_vertices() as i32 > max_num_vertices {
                    return Err(DracoError::general(
                        "Unexpected number of vertices in TOPOLOGY_R/L".to_string(),
                    ));
                }

                self.corner_table
                    .map_corner_to_vertex(opp_corner, new_vert_index);
                self.corner_table
                    .set_left_most_corner(new_vert_index, opp_corner);

                let vertex_r = self.corner_table.vertex_before(corner_a);
                self.corner_table.map_corner_to_vertex(corner_r, vertex_r);
                self.corner_table.set_left_most_corner(vertex_r, corner_r);

                self.corner_table
                    .map_corner_to_vertex(corner_l, self.corner_table.vertex_after(corner_a));
                self.replace_active_corner(corner, "TOPOLOGY_R/L")?;
                check_topology_split = true;
            } else if symbol == 1 {
                // TOPOLOGY_S
                if self.active_corner_stack.is_empty() {
                    return Err(DracoError::general(
                        "active_corner_stack empty in TOPOLOGY_S".to_string(),
                    ));
                }
                let corner_b = self.pop_active_corner("TOPOLOGY_S")?;

                let decoder_split_symbol_id = symbol_id;
                if let Some(corner_from_map) = self
                    .topology_split_active_corners
                    .get(&decoder_split_symbol_id)
                    .cloned()
                {
                    self.active_corner_stack.push(corner_from_map);
                }

                if self.active_corner_stack.is_empty() {
                    return Err(DracoError::general(
                        "active_corner_stack empty in TOPOLOGY_S after split retrieval".to_string(),
                    ));
                }
                let corner_a = self.active_corner("TOPOLOGY_S")?;

                if corner_a == corner_b {
                    return Err(DracoError::general(
                        "corner_a == corner_b in TOPOLOGY_S".to_string(),
                    ));
                }
                if self.corner_table.opposite(corner_a) != INVALID_CORNER_INDEX
                    || self.corner_table.opposite(corner_b) != INVALID_CORNER_INDEX
                {
                    return Err(DracoError::general(
                        "Edge already opposite in TOPOLOGY_S".to_string(),
                    ));
                }

                let corner = CornerIndex(3 * face.0);
                self.set_opposite_corners(corner_a, corner + 2)?;
                self.set_opposite_corners(corner_b, corner + 1)?;

                let vertex_p = self.corner_table.vertex_before(corner_a);
                self.corner_table.map_corner_to_vertex(corner, vertex_p);
                self.corner_table
                    .map_corner_to_vertex(corner + 1, self.corner_table.vertex_after(corner_a));

                let vert_b_prev = self.corner_table.vertex_before(corner_b);
                self.corner_table
                    .map_corner_to_vertex(corner + 2, vert_b_prev);
                self.corner_table
                    .set_left_most_corner(vert_b_prev, corner + 2);

                let mut corner_n = self.corner_table.next(corner_b);
                let vertex_n = self.corner_table.vertex(corner_n);

                if vertex_n != vertex_p && vertex_n != INVALID_VERTEX_INDEX {
                    traversal_decoder.merge_vertices(vertex_p, vertex_n);
                    self.corner_table.set_left_most_corner(
                        vertex_p,
                        self.corner_table.left_most_corner(vertex_n),
                    );

                    let first_corner = corner_n;
                    while corner_n != INVALID_CORNER_INDEX {
                        self.corner_table.map_corner_to_vertex(corner_n, vertex_p);
                        corner_n = self.corner_table.swing_left(corner_n);
                        if corner_n == first_corner {
                            return Err(DracoError::general(
                                "Cycle detected in vertex merge".to_string(),
                            ));
                        }
                    }

                    self.corner_table.make_vertex_isolated(vertex_n);
                    if remove_invalid_vertices {
                        self.invalid_vertices.push(vertex_n);
                    }
                }
                self.replace_active_corner(corner, "TOPOLOGY_S")?;
                traversal_decoder.on_split_symbol_decoded(corner);
            } else if symbol == 4 {
                // TOPOLOGY_E
                let corner = CornerIndex(3 * face.0);
                let v0 = self.corner_table.add_new_vertex();
                let v1 = self.corner_table.add_new_vertex();
                let v2 = self.corner_table.add_new_vertex();

                traversal_decoder.on_vertex_created(v0, symbol_id, corner.0 as i32);
                traversal_decoder.on_vertex_created(v1, symbol_id, (corner.0 + 1) as i32);
                traversal_decoder.on_vertex_created(v2, symbol_id, (corner.0 + 2) as i32);

                if self.corner_table.num_vertices() as i32 > max_num_vertices {
                    return Err(DracoError::general(
                        "Unexpected number of vertices in TOPOLOGY_E".to_string(),
                    ));
                }

                self.corner_table.map_corner_to_vertex(corner, v0);
                self.corner_table.map_corner_to_vertex(corner + 1, v1);
                self.corner_table.map_corner_to_vertex(corner + 2, v2);

                self.corner_table.set_left_most_corner(v0, corner);
                self.corner_table.set_left_most_corner(v1, corner + 1);
                self.corner_table.set_left_most_corner(v2, corner + 2);

                self.active_corner_stack.push(corner);
                check_topology_split = true;
            } else {
                return Err(DracoError::general(format!("Unknown symbol {}", symbol)));
            }

            if check_topology_split {
                // encoder_symbol_id in C++ is num_symbols - symbol_id - 1
                // Rust loop symbol_id goes 0..num_symbols
                // so this matches.
                let encoder_symbol_id = num_symbols - symbol_id - 1;
                while let Some((split_edge, encoder_split_symbol_id)) =
                    traversal_decoder.is_topology_split(encoder_symbol_id)
                {
                    if encoder_split_symbol_id < 0 {
                        return Err(DracoError::general("Invalid split symbol id".to_string()));
                    }
                    let act_top_corner = self.active_corner("topology split")?;
                    let new_active_corner = match split_edge {
                        EdgeFaceName::RightFaceEdge => self.corner_table.next(act_top_corner),
                        EdgeFaceName::LeftFaceEdge => self.corner_table.previous(act_top_corner),
                    };
                    let decoder_split_symbol_id = num_symbols - encoder_split_symbol_id - 1;
                    self.topology_split_active_corners
                        .insert(decoder_split_symbol_id, new_active_corner);
                }
            }

            // Inform the traversal decoder that a new active corner has been reached.
            // This is the decoder-side equivalent of the encoder's corner visitation order
            // and is used for attribute sequencing.
            if let Some(&active_corner) = self.active_corner_stack.last() {
                traversal_decoder.new_active_corner_reached(active_corner, &self.corner_table);
            } else {
                return Err(DracoError::general(
                    "active_corner_stack empty after decoding symbol".to_string(),
                ));
            }
        }

        if self.corner_table.num_vertices() as i32 > max_num_vertices {
            return Err(DracoError::general(
                "Unexpected number of vertices after first pass".to_string(),
            ));
        }

        // Process component roots in LIFO order (matching C++ pop_back())
        while let Some(corner) = self.active_corner_stack.pop() {
            let interior_face = traversal_decoder.decode_start_face_configuration();
            if interior_face {
                if num_faces >= self.declared_num_faces {
                    return Err(DracoError::general(
                        "More faces than expected in start face config".to_string(),
                    ));
                }
                let corner_a = corner;
                let vert_n = self.corner_table.vertex_after(corner_a);
                if self.corner_table.left_most_corner(vert_n) == INVALID_CORNER_INDEX {
                    return Err(DracoError::general(format!(
                        "Invalid left_most_corner for vert_n={}",
                        vert_n.0
                    )));
                }

                let corner_b = self
                    .corner_table
                    .next(self.corner_table.left_most_corner(vert_n));
                let vert_x = self.corner_table.vertex_after(corner_b);
                if self.corner_table.left_most_corner(vert_x) == INVALID_CORNER_INDEX {
                    return Err(DracoError::general(
                        "Invalid left_most_corner for vert_x".to_string(),
                    ));
                }

                let corner_c = self
                    .corner_table
                    .next(self.corner_table.left_most_corner(vert_x));
                let vert_p = self.corner_table.vertex_after(corner_c);

                let face = FaceIndex(num_faces as u32);
                num_faces += 1;
                // As in the symbol loop: the fill covers this face unless the
                // stream-size cap fell short of the declared count.
                if num_faces as usize > filled_faces {
                    self.corner_table
                        .try_push_face(self.declared_num_faces.max(0) as usize)?;
                }
                let new_corner = CornerIndex(3 * face.0);
                self.set_opposite_corners(new_corner, corner_a)?;
                self.set_opposite_corners(new_corner + 1, corner_b)?;
                self.set_opposite_corners(new_corner + 2, corner_c)?;

                self.corner_table.map_corner_to_vertex(new_corner, vert_x);
                self.corner_table
                    .map_corner_to_vertex(new_corner + 1, vert_p);
                self.corner_table
                    .map_corner_to_vertex(new_corner + 2, vert_n);

                for i in 0..3 {
                    let vertex = self.corner_table.vertex(new_corner + i);
                    self.mark_vert_not_hole(vertex, "start face config")?;
                }
                // Pass new_corner directly, matching C++ init_corners_.push_back(new_corner)
                traversal_decoder.on_start_face_decoded(new_corner);
            } else {
                // Boundary case: Pass corner directly, matching C++ init_corners_.push_back(corner)
                traversal_decoder.on_start_face_decoded(corner);
            }
        }

        if num_faces != self.declared_num_faces {
            return Err(DracoError::general(
                "Unexpected number of faces at end".to_string(),
            ));
        }

        // The fill above ran to a bound that can exceed what was built, so put
        // `num_corners()` back to the faces that exist before anything outside
        // this decode reads the table. In the ordinary case the two are equal
        // and this does nothing: the bound is the declared count capped by the
        // stream size, and the loop just checked that exactly the declared
        // count was built.
        self.corner_table.truncate_to_faces(num_faces.max(0) as usize);

        // Give the table its full extent now that the real vertex count is
        // known. Callers index it by vertex and treat a missing entry as a
        // boundary test rather than as `true`, so it has to cover every vertex
        // the traversal created -- it just no longer has to be sized for the
        // count the bitstream merely claimed.
        let decoded_num_vertices = self.corner_table.num_vertices();
        if self.is_vert_hole.len() < decoded_num_vertices {
            self.is_vert_hole.resize(decoded_num_vertices, true);
        }

        let mut num_vertices = self.corner_table.num_vertices() as i32;

        // Compact vertices (remove isolated/invalid ones)
        // Match C++ logic: iterate invalid_vertices (in order added!)
        for invalid_vert in &self.invalid_vertices {
            let invalid_vert = *invalid_vert;

            // Find the last valid vertex (src_vert)
            let mut src_vert = VertexIndex(num_vertices as u32 - 1);
            while src_vert.0 > 0
                && self.corner_table.left_most_corner(src_vert) == INVALID_CORNER_INDEX
            {
                num_vertices -= 1;
                if num_vertices == 0 {
                    break;
                }
                src_vert = VertexIndex(num_vertices as u32 - 1);
            }
            if src_vert < invalid_vert {
                continue; // No need to swap
            }

            // Remap all corners mapped to src_vert to invalid_vert
            // Use SwingRight traversal (matching C++ VertexCornersIterator)
            let start_corner = self.corner_table.left_most_corner(src_vert);
            if start_corner != INVALID_CORNER_INDEX {
                let mut c = start_corner;
                loop {
                    // Check logic: C++ "if (corner_table_->Vertex(cid) != src_vert) { Error }"
                    if self.corner_table.vertex(c) != src_vert {
                        return Err(DracoError::general(format!(
                            "Vertex mismatch during compaction: corner {} maps to {} expected {}",
                            c.0,
                            self.corner_table.vertex(c).0,
                            src_vert.0
                        )));
                    }
                    self.corner_table.map_corner_to_vertex(c, invalid_vert);
                    c = self.corner_table.swing_right(c);
                    if c == INVALID_CORNER_INDEX || c == start_corner {
                        break;
                    }
                }
            }

            self.corner_table
                .set_left_most_corner(invalid_vert, self.corner_table.left_most_corner(src_vert));
            traversal_decoder.on_vertices_swapped(invalid_vert, src_vert);
            self.corner_table.make_vertex_isolated(src_vert);

            if (invalid_vert.0 as usize) < self.is_vert_hole.len()
                && (src_vert.0 as usize) < self.is_vert_hole.len()
            {
                self.is_vert_hole[invalid_vert.0 as usize] = self.is_vert_hole[src_vert.0 as usize];
                self.is_vert_hole[src_vert.0 as usize] = false;
            }

            num_vertices -= 1;
        }

        // Debug output: show corner table after connectivity decoding
        #[cfg(feature = "debug_logs")]
        if crate::debug_env_enabled("DRACO_VERBOSE") {
            debug_log!("Rust CONN: Corner table after connectivity:");
            let max_corners = 12.min(self.corner_table.num_faces() * 3);
            for c in 0..max_corners {
                debug_log!(
                    "  corner {} -> vertex {}",
                    c,
                    self.corner_table.vertex(CornerIndex(c as u32)).0
                );
            }
            debug_log!(
                "Rust CONN: num_vertices after compaction = {}",
                num_vertices
            );
        }

        Ok(num_vertices)
    }

    fn active_corner(&self, context: &str) -> Result<CornerIndex, DracoError> {
        self.active_corner_stack
            .last()
            .copied()
            .ok_or_else(|| DracoError::general(format!("active_corner_stack empty in {context}")))
    }

    fn replace_active_corner(
        &mut self,
        corner: CornerIndex,
        context: &str,
    ) -> Result<(), DracoError> {
        let active = self.active_corner_stack.last_mut().ok_or_else(|| {
            DracoError::general(format!("active_corner_stack empty in {context}"))
        })?;
        *active = corner;
        Ok(())
    }

    fn pop_active_corner(&mut self, context: &str) -> Result<CornerIndex, DracoError> {
        self.active_corner_stack
            .pop()
            .ok_or_else(|| DracoError::general(format!("active_corner_stack empty in {context}")))
    }

    fn vertex_index(&self, vertex: VertexIndex, context: &str) -> Result<usize, DracoError> {
        if vertex == INVALID_VERTEX_INDEX || vertex.0 as usize >= self.max_num_vertices {
            return Err(DracoError::general(format!(
                "Invalid vertex {} while decoding {context}",
                vertex.0
            )));
        }
        Ok(vertex.0 as usize)
    }

    /// Pairs two corners as each other's opposite, refusing either that is
    /// past the table.
    ///
    /// Each write is bounded by `try_set_opposite` on the map it indexes,
    /// rather than compared here against `num_corners()` and then indexed:
    /// the two maps are the same length by invariant, but not visibly so, so
    /// the comparison here used to leave the indexing's own panic check
    /// standing behind it -- twice per call, three calls a face.
    fn set_opposite_corners(&mut self, c1: CornerIndex, c2: CornerIndex) -> Result<(), DracoError> {
        let invalid = |corner: CornerIndex| {
            DracoError::general(format!("Invalid opposite corner {}", corner.0))
        };
        if c1 != INVALID_CORNER_INDEX && !self.corner_table.try_set_opposite(c1, c2) {
            return Err(invalid(c1));
        }
        if c2 != INVALID_CORNER_INDEX && !self.corner_table.try_set_opposite(c2, c1) {
            return Err(invalid(c2));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticTraversalDecoder {
        symbols: Vec<u32>,
        next_symbol: usize,
    }

    impl StaticTraversalDecoder {
        fn new(symbols: Vec<u32>) -> Self {
            Self {
                symbols,
                next_symbol: 0,
            }
        }
    }

    impl EdgebreakerTraversalDecoder for StaticTraversalDecoder {
        fn decode_symbol(&mut self) -> Result<u32, DracoError> {
            let symbol = *self.symbols.get(self.next_symbol).ok_or_else(|| {
                DracoError::general("Traversal symbol stream exhausted".to_string())
            })?;
            self.next_symbol += 1;
            Ok(symbol)
        }

        fn decode_start_face_configuration(&mut self) -> bool {
            false
        }

        fn merge_vertices(&mut self, _p: VertexIndex, _n: VertexIndex) {}

        fn is_topology_split(&mut self, _encoder_symbol_id: i32) -> Option<(EdgeFaceName, i32)> {
            None
        }

        fn on_vertex_created(&mut self, _vertex: VertexIndex, _symbol_id: i32, _corner_index: i32) {
        }

        fn on_vertices_swapped(&mut self, _v1: VertexIndex, _v2: VertexIndex) {}

        fn on_start_face_decoded(&mut self, _corner: CornerIndex) {}
    }

    #[test]
    fn invalid_opposite_corner_is_rejected_without_indexing() {
        let mut decoder = EdgebreakerConnectivityDecoder::new(1, 3);

        let status = decoder.set_opposite_corners(CornerIndex(3), CornerIndex(0));

        assert!(status.is_err());
    }

    #[test]
    fn topology_symbol_that_requires_active_corner_fails_cleanly() {
        let mut decoder = EdgebreakerConnectivityDecoder::new(1, 3);
        let mut traversal_decoder = StaticTraversalDecoder::new(vec![0]); // TOPOLOGY_C

        let status = decoder.decode_connectivity(1, &mut traversal_decoder, true, 0);

        assert!(status.is_err());
    }

    #[test]
    fn exhausted_traversal_symbol_stream_fails_cleanly() {
        let mut decoder = EdgebreakerConnectivityDecoder::new(1, 3);
        let mut traversal_decoder = StaticTraversalDecoder::new(Vec::new());

        let status = decoder.decode_connectivity(1, &mut traversal_decoder, true, 0);

        assert!(status.is_err());
    }
}
