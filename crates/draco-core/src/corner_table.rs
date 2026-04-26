use crate::geometry_indices::{
    CornerIndex, FaceIndex, VertexIndex, INVALID_CORNER_INDEX, INVALID_VERTEX_INDEX,
};

#[derive(Debug, Default, Clone)]
pub struct CornerTable {
    pub corner_to_vertex_map: Vec<VertexIndex>,
    pub opposite_corners: Vec<CornerIndex>,
    pub vertex_corners: Vec<CornerIndex>,
    #[allow(dead_code)]
    pub num_original_vertices: usize,
    pub num_degenerated_faces: usize,
    pub num_isolated_vertices: usize,
}

impl CornerTable {
    pub fn new(num_faces: usize) -> Self {
        Self {
            corner_to_vertex_map: vec![INVALID_VERTEX_INDEX; num_faces * 3],
            opposite_corners: vec![INVALID_CORNER_INDEX; num_faces * 3],
            vertex_corners: Vec::new(),
            num_original_vertices: 0,
            num_degenerated_faces: 0,
            num_isolated_vertices: 0,
        }
    }

    pub fn map_corner_to_vertex(&mut self, corner: CornerIndex, vertex: VertexIndex) {
        self.corner_to_vertex_map[corner.0 as usize] = vertex;
    }

    pub fn set_face_vertices(
        &mut self,
        face: FaceIndex,
        v0: crate::geometry_indices::PointIndex,
        v1: crate::geometry_indices::PointIndex,
        v2: crate::geometry_indices::PointIndex,
    ) {
        let c0 = self.first_corner(face);
        let c1 = self.next(c0);
        let c2 = self.previous(c0);

        self.map_corner_to_vertex(c0, VertexIndex(v0.0));
        self.map_corner_to_vertex(c1, VertexIndex(v1.0));
        self.map_corner_to_vertex(c2, VertexIndex(v2.0));

        // Ensure vertex_corners is large enough and set deterministically
        let max_v = usize::max(v0.0 as usize, usize::max(v1.0 as usize, v2.0 as usize));
        if self.vertex_corners.len() <= max_v {
            self.vertex_corners.resize(max_v + 1, INVALID_CORNER_INDEX);
        }
        self.vertex_corners[v0.0 as usize] = c0;
        self.vertex_corners[v1.0 as usize] = c1;
        self.vertex_corners[v2.0 as usize] = c2;
    }

    pub fn set_opposite(&mut self, corner: CornerIndex, opposite: CornerIndex) {
        // Debug logging removed to avoid noisy output during tests.
        self.opposite_corners[corner.0 as usize] = opposite;
    }

    pub fn init(&mut self, faces: &[[VertexIndex; 3]]) -> bool {
        self.corner_to_vertex_map
            .resize(faces.len() * 3, INVALID_VERTEX_INDEX);
        for (fi, face) in faces.iter().enumerate() {
            for i in 0..3 {
                self.corner_to_vertex_map[fi * 3 + i] = face[i];
            }
        }

        let mut num_vertices = 0;
        if !self.compute_opposite_corners(&mut num_vertices) {
            return false;
        }

        if !self.break_non_manifold_edges() {
            return false;
        }

        if !self.compute_vertex_corners(num_vertices) {
            return false;
        }

        self.num_degenerated_faces = 0;
        for f in 0..self.num_faces() {
            if self.is_degenerated(FaceIndex(f as u32)) {
                self.num_degenerated_faces += 1;
            }
        }

        self.num_isolated_vertices = 0;
        for v in 0..self.num_vertices() {
            if self.vertex_corners[v] == INVALID_CORNER_INDEX {
                self.num_isolated_vertices += 1;
            }
        }

        // In debug builds perform an invariant check to catch subtle topology bugs early.
        debug_assert!(self.validate_invariants());

        true
    }

    pub fn num_vertices(&self) -> usize {
        self.vertex_corners.len()
    }

    pub fn num_isolated_vertices(&self) -> usize {
        self.num_isolated_vertices
    }

    pub fn num_degenerated_faces(&self) -> usize {
        self.num_degenerated_faces
    }

    pub fn is_degenerated(&self, face: FaceIndex) -> bool {
        if face == crate::geometry_indices::INVALID_FACE_INDEX {
            return true;
        }
        let c0 = self.first_corner(face);
        let v0 = self.vertex(c0);
        let v1 = self.vertex(self.next(c0));
        let v2 = self.vertex(self.previous(c0));
        v0 == v1 || v0 == v2 || v1 == v2
    }

    pub fn num_corners(&self) -> usize {
        self.corner_to_vertex_map.len()
    }

    pub fn num_faces(&self) -> usize {
        self.corner_to_vertex_map.len() / 3
    }

    pub fn add_new_vertex(&mut self) -> VertexIndex {
        let new_idx = self.vertex_corners.len();
        self.vertex_corners.push(INVALID_CORNER_INDEX);
        VertexIndex(new_idx as u32)
    }

    pub fn left_most_corner(&self, v: VertexIndex) -> CornerIndex {
        if v.0 < self.vertex_corners.len() as u32 {
            self.vertex_corners[v.0 as usize]
        } else {
            INVALID_CORNER_INDEX
        }
    }

    pub fn set_left_most_corner(&mut self, v: VertexIndex, c: CornerIndex) {
        let idx = v.0 as usize;
        if idx >= self.vertex_corners.len() {
            self.vertex_corners.resize(idx + 1, INVALID_CORNER_INDEX);
        }
        self.vertex_corners[idx] = c;
    }

    pub fn make_vertex_isolated(&mut self, v: VertexIndex) {
        if v.0 < self.vertex_corners.len() as u32 {
            self.vertex_corners[v.0 as usize] = INVALID_CORNER_INDEX;
        }
    }

    pub fn opposite(&self, corner: CornerIndex) -> CornerIndex {
        if corner == INVALID_CORNER_INDEX {
            return corner;
        }
        self.opposite_corners[corner.0 as usize]
    }

    pub fn next(&self, corner: CornerIndex) -> CornerIndex {
        if corner == INVALID_CORNER_INDEX {
            return corner;
        }
        if !(corner.0 + 1).is_multiple_of(3) {
            CornerIndex(corner.0 + 1)
        } else {
            CornerIndex(corner.0 - 2)
        }
    }

    pub fn previous(&self, corner: CornerIndex) -> CornerIndex {
        if corner == INVALID_CORNER_INDEX {
            return corner;
        }
        if !corner.0.is_multiple_of(3) {
            CornerIndex(corner.0 - 1)
        } else {
            CornerIndex(corner.0 + 2)
        }
    }

    pub fn vertex(&self, corner: CornerIndex) -> VertexIndex {
        if corner == INVALID_CORNER_INDEX {
            return INVALID_VERTEX_INDEX;
        }
        self.corner_to_vertex_map[corner.0 as usize]
    }

    pub fn face(&self, corner: CornerIndex) -> FaceIndex {
        if corner == INVALID_CORNER_INDEX {
            return crate::geometry_indices::INVALID_FACE_INDEX;
        }
        FaceIndex(corner.0 / 3)
    }

    pub fn first_corner(&self, face: FaceIndex) -> CornerIndex {
        CornerIndex(face.0 * 3)
    }

    /// Swings from a corner to the next corner sharing the same vertex in a counter-clockwise direction.
    /// Validate that every opposite corner pair corresponds to the same undirected edge.
    /// Returns true when all assigned opposites match edge endpoints.
    pub fn validate_opposite_edge_consistency(&self) -> bool {
        for c_idx in 0..self.num_corners() {
            let c = CornerIndex(c_idx as u32);
            let opp = self.opposite(c);
            if opp == INVALID_CORNER_INDEX {
                continue;
            }
            let a = self.vertex(self.next(c)).0;
            let b = self.vertex(self.previous(c)).0;
            let oa = self.vertex(self.next(opp)).0;
            let ob = self.vertex(self.previous(opp)).0;
            if !((a == oa && b == ob) || (a == ob && b == oa)) {
                return false;
            }
        }
        true
    }

    /// Returns the corner on the left-adjacent face.
    /// C++: Opposite(Previous(corner))
    pub fn left_corner(&self, corner: CornerIndex) -> CornerIndex {
        self.opposite(self.previous(corner))
    }

    /// Returns the corner on the right-adjacent face.
    /// C++: Opposite(Next(corner))
    pub fn right_corner(&self, corner: CornerIndex) -> CornerIndex {
        self.opposite(self.next(corner))
    }

    /// Returns the corner on the adjacent face on the right that maps to
    /// the same vertex as the given corner.
    /// C++: Previous(Opposite(Previous(corner)))
    pub fn swing_right(&self, corner: CornerIndex) -> CornerIndex {
        self.previous(self.opposite(self.previous(corner)))
    }

    /// Returns the corner on the left face that maps to the same vertex as the
    /// given corner.
    /// C++: Next(Opposite(Next(corner)))
    pub fn swing_left(&self, corner: CornerIndex) -> CornerIndex {
        self.next(self.opposite(self.next(corner)))
    }

    fn break_non_manifold_edges(&mut self) -> bool {
        // This function detects and breaks non-manifold edges that are caused by
        // folds in 1-ring neighborhood around a vertex. Non-manifold edges can occur
        // when the 1-ring surface around a vertex self-intersects in a common edge.
        // For example imagine a surface around a pivot vertex 0, where the 1-ring
        // is defined by vertices |1, 2, 3, 1, 4|. The surface passes edge <0, 1>
        // twice which would result in a non-manifold edge that needs to be broken.
        // For now all faces connected to these non-manifold edges are disconnected
        // resulting in open boundaries on the mesh. New vertices will be created
        // automatically for each new disjoint patch in the ComputeVertexCorners()
        // method.
        // Note that all other non-manifold edges are implicitly handled by the
        // function ComputeVertexCorners() that automatically creates new vertices
        // on disjoint 1-ring surface patches.

        let mut visited_corners = vec![false; self.num_corners()];
        let mut sink_vertices: Vec<(VertexIndex, CornerIndex)> = Vec::new();

        loop {
            let mut mesh_connectivity_updated = false;
            for c in 0..self.num_corners() {
                let c_idx = CornerIndex(c as u32);
                if visited_corners[c] {
                    continue;
                }

                sink_vertices.clear();

                // First swing all the way to find the left-most corner connected to the
                // corner's vertex.
                let mut first_c = c_idx;
                let mut current_c = c_idx;

                loop {
                    let next_c = self.swing_left(current_c);
                    if next_c == first_c
                        || next_c == INVALID_CORNER_INDEX
                        || visited_corners[next_c.0 as usize]
                    {
                        break;
                    }
                    current_c = next_c;
                }

                first_c = current_c;

                // Swing right from the first corner and check if all visited edges
                // are unique.
                loop {
                    visited_corners[current_c.0 as usize] = true;

                    // Each new edge is defined by the pivot vertex (that is the same for
                    // all faces) and by the sink vertex (that is the |next| vertex from the
                    // currently processed pivot corner. I.e., each edge is uniquely defined
                    // by the sink vertex index.
                    let sink_c = self.next(current_c);
                    let sink_v = self.corner_to_vertex_map[sink_c.0 as usize];

                    // Corner that defines the edge on the face.
                    let edge_corner = self.previous(current_c);
                    let mut vertex_connectivity_updated = false;

                    // Go over all processed edges (sink vertices). If the current sink
                    // vertex has been already encountered before it may indicate a
                    // non-manifold edge that needs to be broken.
                    for attached_sink_vertex in &sink_vertices {
                        if attached_sink_vertex.0 == sink_v {
                            // Sink vertex has been already processed.
                            let other_edge_corner = attached_sink_vertex.1;
                            let opp_edge_corner = self.opposite(edge_corner);

                            if opp_edge_corner == other_edge_corner {
                                // We are closing the loop so no need to change the connectivity.
                                continue;
                            }

                            // Break the connectivity on the non-manifold edge.
                            let opp_other_edge_corner = self.opposite(other_edge_corner);
                            if opp_edge_corner != INVALID_CORNER_INDEX {
                                self.opposite_corners[opp_edge_corner.0 as usize] =
                                    INVALID_CORNER_INDEX;
                            }
                            if opp_other_edge_corner != INVALID_CORNER_INDEX {
                                self.opposite_corners[opp_other_edge_corner.0 as usize] =
                                    INVALID_CORNER_INDEX;
                            }

                            self.opposite_corners[edge_corner.0 as usize] = INVALID_CORNER_INDEX;
                            self.opposite_corners[other_edge_corner.0 as usize] =
                                INVALID_CORNER_INDEX;

                            vertex_connectivity_updated = true;
                            break;
                        }
                    }

                    if vertex_connectivity_updated {
                        // Because of the updated connectivity, not all corners connected to
                        // this vertex have been processed and we need to go over them again.
                        mesh_connectivity_updated = true;
                        break;
                    }

                    // Insert new sink vertex information <sink vertex index, edge corner>.
                    let new_sink_vert = (
                        self.corner_to_vertex_map[self.previous(current_c).0 as usize],
                        sink_c,
                    );
                    sink_vertices.push(new_sink_vert);

                    current_c = self.swing_right(current_c);
                    if current_c == first_c || current_c == INVALID_CORNER_INDEX {
                        break;
                    }
                }
            }

            if !mesh_connectivity_updated {
                break;
            }
        }

        true
    }

    fn compute_opposite_corners(&mut self, num_vertices: &mut usize) -> bool {
        self.opposite_corners
            .resize(self.num_corners(), INVALID_CORNER_INDEX);

        // 1. Count outgoing half-edges per vertex
        let mut num_corners_on_vertices = Vec::new();
        for c in 0..self.num_corners() {
            let v1 = self.vertex(CornerIndex(c as u32));
            if v1 == INVALID_VERTEX_INDEX {
                continue;
            }
            let v1_val = v1.0 as usize;
            if v1_val >= num_corners_on_vertices.len() {
                num_corners_on_vertices.resize(v1_val + 1, 0);
            }
            num_corners_on_vertices[v1_val] += 1;
        }

        // 2. Create storage for half-edges
        #[derive(Clone, Copy, Debug)]
        struct VertexEdgePair {
            sink_vert: VertexIndex,
            edge_corner: CornerIndex,
        }
        let mut vertex_edges = vec![
            VertexEdgePair {
                sink_vert: INVALID_VERTEX_INDEX,
                edge_corner: INVALID_CORNER_INDEX
            };
            self.num_corners()
        ];

        // 3. Compute offsets
        let mut vertex_offset = vec![0; num_corners_on_vertices.len()];
        let mut offset = 0;
        for i in 0..num_corners_on_vertices.len() {
            vertex_offset[i] = offset;
            offset += num_corners_on_vertices[i];
        }

        // 4. Connect half-edges
        for c in 0..self.num_corners() {
            let c_idx = CornerIndex(c as u32);
            let tip_v = self.vertex(c_idx);
            let source_v = self.vertex(self.next(c_idx));
            let sink_v = self.vertex(self.previous(c_idx));

            // Check for degenerated faces
            let f_first = self.first_corner(self.face(c_idx));
            let v0 = self.vertex(f_first);
            let v1 = self.vertex(self.next(f_first));
            let v2 = self.vertex(self.previous(f_first));
            if v0 == v1 || v1 == v2 || v2 == v0 {
                continue;
            }

            let mut opposite_c = INVALID_CORNER_INDEX;
            let num_corners_on_vert = num_corners_on_vertices[sink_v.0 as usize];
            let mut offset = vertex_offset[sink_v.0 as usize];

            let mut found_match = false;
            let mut match_pos_found: Option<usize> = None;

            // Search for matching half-edge on sink vertex.
            // Match C++ behavior: take the first match we find (early break).
            for i in 0..num_corners_on_vert {
                let other_v = vertex_edges[offset].sink_vert;
                if other_v == INVALID_VERTEX_INDEX {
                    break;
                }
                if other_v == source_v {
                    // Check for mirrored faces
                    if tip_v == self.vertex(vertex_edges[offset].edge_corner) {
                        offset += 1;
                        continue;
                    }
                    // Take first match (matches C++ behavior)
                    match_pos_found = Some(vertex_offset[sink_v.0 as usize] + i);
                    break;
                }
                offset += 1;
            }

            if let Some(match_pos) = match_pos_found {
                let start = vertex_offset[sink_v.0 as usize];
                let count = num_corners_on_vertices[sink_v.0 as usize];
                opposite_c = vertex_edges[match_pos].edge_corner;

                // Shift elements left to remove the matched entry
                if match_pos + 1 < start + count {
                    vertex_edges.copy_within(match_pos + 1..start + count, match_pos);
                }
                if count > 0 {
                    vertex_edges[start + count - 1].sink_vert = INVALID_VERTEX_INDEX;
                    vertex_edges[start + count - 1].edge_corner = INVALID_CORNER_INDEX;
                }
                found_match = true;
            }

            // Debug logging removed to avoid noisy output during tests.

            if !found_match {
                // No opposite found, add to source vertex list
                let num_corners_on_source = num_corners_on_vertices[source_v.0 as usize];
                let mut offset = vertex_offset[source_v.0 as usize];
                for _ in 0..num_corners_on_source {
                    if vertex_edges[offset].sink_vert == INVALID_VERTEX_INDEX {
                        vertex_edges[offset].sink_vert = sink_v;
                        vertex_edges[offset].edge_corner = c_idx;
                        break;
                    }
                    offset += 1;
                }
            } else {
                self.opposite_corners[c] = opposite_c;
                self.opposite_corners[opposite_c.0 as usize] = c_idx;
            }
        }

        *num_vertices = num_corners_on_vertices.len();
        true
    }

    pub fn compute_vertex_corners(&mut self, mut num_vertices: usize) -> bool {
        self.num_original_vertices = num_vertices;
        self.vertex_corners
            .resize(num_vertices, INVALID_CORNER_INDEX);

        // Arrays for marking visited vertices and corners that allow us to detect
        // non-manifold vertices.
        let mut visited_vertices = vec![false; num_vertices];
        let mut visited_corners = vec![false; self.num_corners()];

        for f in 0..self.num_faces() {
            let first_face_corner = self.first_corner(FaceIndex(f as u32));

            // Check whether the face is degenerated. If so ignore it.
            if self.is_degenerated(FaceIndex(f as u32)) {
                continue;
            }

            for k in 0..3 {
                let c = CornerIndex(first_face_corner.0 + k);
                if visited_corners[c.0 as usize] {
                    continue;
                }

                let mut v = self.corner_to_vertex_map[c.0 as usize];

                // Note that one vertex maps to many corners; if the vertex was already
                // visited on another corner of the same original vertex, we must
                // create a new vertex (non-manifold handling).
                let mut is_non_manifold_vertex = false;
                if v.0 as usize >= visited_vertices.len() {
                    // Defensive: grow visited_vertices if corner table had larger index.
                    visited_vertices.resize(v.0 as usize + 1, false);
                }
                if visited_vertices[v.0 as usize] {
                    self.vertex_corners.push(INVALID_CORNER_INDEX);
                    visited_vertices.push(false);
                    v = VertexIndex(num_vertices as u32);
                    num_vertices += 1;
                    is_non_manifold_vertex = true;
                }

                // Mark the vertex as visited.
                visited_vertices[v.0 as usize] = true;

                // First swing all the way to the left and mark all corners on the way.
                // Vertex will eventually point to the left most corner (the corner from
                // which SwingLeft returns invalid - i.e., boundary corner).
                let mut act_c = c;
                while act_c != INVALID_CORNER_INDEX {
                    visited_corners[act_c.0 as usize] = true;
                    // Vertex will eventually point to the left most corner.
                    self.vertex_corners[v.0 as usize] = act_c;
                    if is_non_manifold_vertex {
                        // Update vertex index in the corresponding face.
                        self.corner_to_vertex_map[act_c.0 as usize] = v;
                    }
                    act_c = self.swing_left(act_c);
                    if act_c == c {
                        break; // Full circle reached.
                    }
                }

                if act_c == INVALID_CORNER_INDEX {
                    // If we have reached an open boundary we need to swing right from the
                    // initial corner to mark all corners in the opposite direction.
                    act_c = self.swing_right(c);
                    while act_c != INVALID_CORNER_INDEX {
                        visited_corners[act_c.0 as usize] = true;
                        if is_non_manifold_vertex {
                            // Update vertex index in the corresponding face.
                            self.corner_to_vertex_map[act_c.0 as usize] = v;
                        }
                        act_c = self.swing_right(act_c);
                    }
                }
            }
        }

        // Count the number of isolated (unprocessed) vertices.
        self.num_isolated_vertices = 0;
        for visited in visited_vertices {
            if !visited {
                self.num_isolated_vertices += 1;
            }
        }

        true
    }

    /// Validates basic corner-table invariants. Returns true if all checks pass.
    /// Note: this is an O(N) check intended for debug builds only (invoked via debug_assert!).
    pub fn validate_invariants(&self) -> bool {
        // Opposite corner symmetry: opposite(opposite(c)) == c or INVALID
        for c in 0..self.num_corners() {
            let ci = CornerIndex(c as u32);
            let o = self.opposite(ci);
            if o != INVALID_CORNER_INDEX && self.opposite(o) != ci {
                eprintln!(
                    "CornerTable invariant failed: opposite(opposite({})) != {} (got {})",
                    c,
                    c,
                    self.opposite(o).0
                );
                return false;
            }
        }

        // Non-degenerated faces must have valid vertex mappings
        for f in 0..self.num_faces() {
            let face = FaceIndex(f as u32);
            if self.is_degenerated(face) {
                continue;
            }
            let c0 = self.first_corner(face);
            if self.vertex(c0) == INVALID_VERTEX_INDEX
                || self.vertex(self.next(c0)) == INVALID_VERTEX_INDEX
                || self.vertex(self.previous(c0)) == INVALID_VERTEX_INDEX
            {
                eprintln!(
                    "CornerTable invariant failed: face {} contains INVALID vertex mapping",
                    f
                );
                return false;
            }
        }

        // vertex_corners must reference the expected vertex
        for v in 0..self.vertex_corners.len() {
            let c = self.vertex_corners[v];
            if c == INVALID_CORNER_INDEX {
                continue;
            }
            if self.vertex(c) != VertexIndex(v as u32) {
                eprintln!("CornerTable invariant failed: vertex_corners[{}] -> corner {} maps to vertex {}", v, c.0, self.vertex(c).0);
                return false;
            }
        }

        true
    }
    pub fn valence(&self, v: VertexIndex) -> i32 {
        let mut valence = 0;
        let start_corner = self.left_most_corner(v);
        if start_corner == INVALID_CORNER_INDEX {
            return 0;
        }
        let mut act_c = start_corner;
        loop {
            valence += 1;
            act_c = self.swing_right(act_c);
            if act_c == start_corner {
                break;
            }
            if act_c == INVALID_CORNER_INDEX {
                // If we are on a boundary we need to add 1 to the valence (one extra edge).
                valence += 1;
                break;
            }
        }
        valence
    }
}
