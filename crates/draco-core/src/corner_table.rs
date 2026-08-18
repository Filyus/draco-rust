//! Corner-table mesh connectivity.
//!
//! [`CornerTable`] is Draco's corner-based connectivity structure: each triangle
//! contributes three corners, and `next`/`previous`/`opposite` plus the
//! corner→vertex map let traversal walk the mesh in O(1) per step. It underpins
//! EdgeBreaker connectivity and every mesh prediction scheme. Port of Draco's
//! `corner_table.h`.

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

/// The next corner within the same face, sentinel test omitted.
///
/// Total on all of `u32` -- the arithmetic wraps rather than overflowing, so
/// there is no precondition on the caller and no way to reach a panic. What the
/// caller does owe is a reading of where the sentinel comes out, and that is
/// where this differs from [`prev_in_face`]:
///
/// This one **absorbs** the sentinel. `u32::MAX + 1` wraps to `0`, which is a
/// multiple of three, so the other arm answers `u32::MAX - 2` -- past the end of
/// any table this crate can build (a corner map that long would be sixteen
/// gigabytes), so the `get` it feeds answers `None` and the sentinel reappears.
/// That holds only while the result goes straight into a bounds-checked index:
/// **use it in the inner position only.** Returned to a caller it would be
/// `u32::MAX - 2`, which is not equal to [`INVALID_CORNER_INDEX`], and every fan
/// loop in this crate tests for the sentinel by equality.
#[inline(always)]
fn next_in_face(corner: u32) -> u32 {
    if !corner.wrapping_add(1).is_multiple_of(3) {
        corner.wrapping_add(1)
    } else {
        corner.wrapping_sub(2)
    }
}

/// The previous corner within the same face, sentinel test omitted.
///
/// Mirror of [`next_in_face`], and total for the same reason -- but this one
/// **preserves** the sentinel, which makes it safe in the outer position too.
/// `u32::MAX` is a multiple of three (its digits sum to 57), so the `+ 2` arm is
/// the one taken, and the saturation holds the answer at `u32::MAX`: the
/// sentinel in, the sentinel out, exactly as [`CornerTable::previous`] answers.
///
/// The saturation is the whole of it. Wrapping would answer `1` -- a perfectly
/// ordinary corner of the first face -- and a fan walk handed that instead of a
/// termination would keep going through unrelated geometry.
#[inline(always)]
fn prev_in_face(corner: u32) -> u32 {
    if !corner.is_multiple_of(3) {
        corner - 1
    } else {
        corner.saturating_add(2)
    }
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

    /// Fallible constructor that reserves the corner storage through
    /// `try_reserve` so a bitstream-controlled `num_faces` cannot abort the
    /// process on a failed allocation; oversized counts return a `DracoError`
    /// instead. Behaviorally identical to [`CornerTable::new`] on success.
    pub fn try_new(num_faces: usize) -> Result<Self, crate::status::DracoError> {
        let num_corners = num_faces.checked_mul(3).ok_or_else(|| {
            crate::status::DracoError::general("Corner table size overflow".to_string())
        })?;

        let mut corner_to_vertex_map = Vec::new();
        corner_to_vertex_map
            .try_reserve_exact(num_corners)
            .map_err(|_| {
                crate::status::DracoError::general("Failed to allocate corner table".to_string())
            })?;
        corner_to_vertex_map.resize(num_corners, INVALID_VERTEX_INDEX);

        let mut opposite_corners = Vec::new();
        opposite_corners
            .try_reserve_exact(num_corners)
            .map_err(|_| {
                crate::status::DracoError::general("Failed to allocate corner table".to_string())
            })?;
        opposite_corners.resize(num_corners, INVALID_CORNER_INDEX);

        Ok(Self {
            corner_to_vertex_map,
            opposite_corners,
            vertex_corners: Vec::new(),
            num_original_vertices: 0,
            num_degenerated_faces: 0,
            num_isolated_vertices: 0,
        })
    }

    /// Grows the corner storage so `face` and its three corners are
    /// addressable, leaving `num_faces()` equal to the number of faces that
    /// have actually been created.
    ///
    /// Edgebreaker decoding creates faces strictly in order and only ever
    /// writes corners of the face it just created or of faces already built,
    /// so growing one face at a time is always sufficient. Sizing the table
    /// from the face count the bitstream *claims* instead lets a few hundred
    /// bytes of malformed input ask for gigabytes before any of the checks
    /// that would reject it have run.
    /// Reserves corner storage for `faces` without changing `num_faces()`.
    ///
    /// Growing a face at a time keeps a claimed face count from allocating
    /// anything, but it also reallocates: decoding a 69k-face mesh moved
    /// through ten reallocations totalling 3.9 MB to reach a 1.7 MB table, and
    /// every one of them copied what was already there. Reserving up front
    /// costs one allocation instead, so this takes a ceiling the caller has
    /// already bounded against the input rather than the count the header
    /// claims. Capacity only: a caller that over-reserves still sees the same
    /// `num_faces()` as it grows.
    pub fn try_reserve_faces(&mut self, faces: usize) -> Result<(), crate::status::DracoError> {
        let Some(num_corners) = faces.checked_mul(3) else {
            return Ok(());
        };
        for (vec_len, reserve) in [
            (self.corner_to_vertex_map.len(), true),
            (self.opposite_corners.len(), false),
        ] {
            let extra = num_corners.saturating_sub(vec_len);
            if extra == 0 {
                continue;
            }
            let result = if reserve {
                self.corner_to_vertex_map.try_reserve(extra)
            } else {
                self.opposite_corners.try_reserve(extra)
            };
            result.map_err(|_| {
                crate::status::DracoError::general("Failed to allocate corner table".to_string())
            })?;
        }
        Ok(())
    }

    pub fn try_grow_to_face(&mut self, face: usize) -> Result<(), crate::status::DracoError> {
        let num_corners = face
            .checked_add(1)
            .and_then(|faces| faces.checked_mul(3))
            .ok_or_else(|| {
                crate::status::DracoError::general("Corner table size overflow".to_string())
            })?;
        if num_corners <= self.corner_to_vertex_map.len() {
            return Ok(());
        }

        let extra = num_corners - self.corner_to_vertex_map.len();
        self.corner_to_vertex_map.try_reserve(extra).map_err(|_| {
            crate::status::DracoError::general("Failed to allocate corner table".to_string())
        })?;
        self.corner_to_vertex_map
            .resize(num_corners, INVALID_VERTEX_INDEX);

        let extra = num_corners - self.opposite_corners.len();
        self.opposite_corners.try_reserve(extra).map_err(|_| {
            crate::status::DracoError::general("Failed to allocate corner table".to_string())
        })?;
        self.opposite_corners
            .resize(num_corners, INVALID_CORNER_INDEX);
        Ok(())
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
        let v1 = self.vertex_after(c0);
        let v2 = self.vertex_before(c0);
        v0 == v1 || v0 == v2 || v1 == v2
    }

    pub fn num_corners(&self) -> usize {
        self.corner_to_vertex_map.len()
    }

    /// Validate that the corner maps are internally consistent so a traversal can
    /// index per-vertex / per-face arrays sized from the table counts without
    /// risking an out-of-bounds panic. Every vertex index must be `INVALID` or
    /// `< num_vertices`, every opposite-corner index must be `INVALID` or
    /// `< num_corners`, and the opposite map must match the corner count.
    /// Malformed (e.g. attribute-seam-modified) tables can violate these and
    /// would otherwise panic on direct indexing in debug and release builds.
    /// O(num_corners); intended for once-per-traversal use off the hot path.
    pub fn is_index_consistent(&self) -> bool {
        if self.opposite_corners.len() != self.corner_to_vertex_map.len() {
            return false;
        }
        let num_vertices = self.vertex_corners.len();
        let num_corners = self.corner_to_vertex_map.len();

        // "sentinel, or below the bound" is one comparison, not two: adding one
        // wraps the sentinel (u32::MAX) to 0, which is below every bound, and
        // shifts every real value so that `>` catches exactly those at or past
        // the bound. Reducing with max instead of short-circuiting on `any`
        // keeps the loop branch-free, so it vectorises -- this walks both maps
        // of a 200k-corner table on the way into each attribute traversal.
        fn exceeds(values: impl Iterator<Item = u32>, bound: usize) -> bool {
            // A bound past u32 cannot be exceeded by a u32 value anyway.
            let bound = u32::try_from(bound).unwrap_or(u32::MAX);
            values.fold(0u32, |worst, v| worst.max(v.wrapping_add(1))) > bound
        }

        if exceeds(self.corner_to_vertex_map.iter().map(|v| v.0), num_vertices) {
            return false;
        }
        if exceeds(self.opposite_corners.iter().map(|c| c.0), num_corners) {
            return false;
        }
        true
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

    /// The corner across the edge from `corner`, or the invalid sentinel.
    ///
    /// Total, like [`vertex`](Self::vertex): the sentinel already meant "no
    /// such corner", and a corner past the table now returns it too rather
    /// than panicking. Roughly twenty-five index expressions across this file
    /// funnel through these two, and the corners reaching them come from
    /// decoded connectivity -- `is_index_consistent` exists to check a whole
    /// table up front, but only three sites call it.
    ///
    /// This and [`vertex`](Self::vertex) sit under every fan walk, so the
    /// bounds check here has been measured rather than argued about (Stanford
    /// Bunny decode, 30 interleaved runs per binary):
    ///
    /// - Spelling the same check as an explicit range test and a direct index
    ///   instead of `.get().copied().unwrap_or(..)` changes nothing (+0.2%,
    ///   inside the 0.4% that code layout moves between builds of identical
    ///   source). It is not the `Option` that costs.
    /// - Dropping the check via `get_unchecked` is worth **2.0%** of decode.
    ///   That is the standing price of the no-`unsafe` promise in SECURITY.md
    ///   on this path, and it is not recoverable by writing the check more
    ///   cleverly -- only by removing it, or by a table where the sentinel is
    ///   in range by construction.
    pub fn opposite(&self, corner: CornerIndex) -> CornerIndex {
        // No sentinel test: the sentinel is `u32::MAX`, so it indexes past any
        // table this crate can build and `get` answers `None` for it exactly as
        // it does for a corner past the end. The test that used to stand here
        // ran on every one of the half-million calls a 69k-face decode makes.
        self.opposite_corners
            .get(corner.0 as usize)
            .copied()
            .unwrap_or(INVALID_CORNER_INDEX)
    }

    /// The next corner within the same face.
    ///
    /// The wrap is a branch on purpose. Replacing it with `c + 1 - 3 * wraps`
    /// measured **8.8% slower** end-to-end on the Bunny: the wrap lands on one
    /// corner in three in a fixed pattern the branch predictor learns, and the
    /// arithmetic form puts a multiply and a subtract on the dependency chain
    /// that every `swing_left`/`swing_right` waits on.
    pub fn next(&self, corner: CornerIndex) -> CornerIndex {
        if corner == INVALID_CORNER_INDEX {
            return corner;
        }
        CornerIndex(next_in_face(corner.0))
    }

    /// The previous corner within the same face. Mirror of [`next`](Self::next),
    /// branch and all.
    pub fn previous(&self, corner: CornerIndex) -> CornerIndex {
        if corner == INVALID_CORNER_INDEX {
            return corner;
        }
        CornerIndex(prev_in_face(corner.0))
    }

    /// The vertex at `corner`, or the invalid sentinel.
    ///
    /// Total for the same reason [`opposite`](Self::opposite) is: callers
    /// already handle the sentinel, so widening "invalid corner" to include
    /// "corner past the table" costs them nothing and removes a panic that a
    /// header count can reach.
    pub fn vertex(&self, corner: CornerIndex) -> VertexIndex {
        // As in `opposite`: the sentinel indexes past the map, so the lookup
        // already answers with the invalid vertex and the explicit test was
        // work on every call -- nine hundred thousand of them per decode here.
        self.corner_to_vertex_map
            .get(corner.0 as usize)
            .copied()
            .unwrap_or(INVALID_VERTEX_INDEX)
    }

    /// The vertex at the next corner of the same face: `vertex(next(corner))`
    /// in one lookup.
    ///
    /// The pair of this and [`vertex_before`](Self::vertex_before) is the
    /// commonest idiom in the crate -- the parallelogram predictors, the
    /// EdgeBreaker symbol arms and the degeneracy test all want two of the three
    /// vertices of a face given the third corner -- and spelled as a composition
    /// it costs a sentinel test on the way in that the bounds check on the way
    /// out already covers. Upstream carries the same consolidation as a standing
    /// TODO in `mesh_prediction_scheme_parallelogram_shared.h`.
    ///
    /// This is not the fusion that was tried and rejected at
    /// [`prediction_scheme_parallelogram`](crate::prediction_scheme_parallelogram):
    /// that one read the whole face triple as an array and handed it back
    /// through memory, and measured 1.4% slower. This stays a scalar load.
    pub fn vertex_after(&self, corner: CornerIndex) -> VertexIndex {
        // `next_in_face` in the inner position: the sentinel lands past the map
        // and `get` answers `None`, exactly as `vertex(next(sentinel))` does.
        self.corner_to_vertex_map
            .get(next_in_face(corner.0) as usize)
            .copied()
            .unwrap_or(INVALID_VERTEX_INDEX)
    }

    /// The vertex at the previous corner of the same face:
    /// `vertex(previous(corner))` in one lookup. Mirror of
    /// [`vertex_after`](Self::vertex_after).
    pub fn vertex_before(&self, corner: CornerIndex) -> VertexIndex {
        self.corner_to_vertex_map
            .get(prev_in_face(corner.0) as usize)
            .copied()
            .unwrap_or(INVALID_VERTEX_INDEX)
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
            let a = self.vertex_after(c).0;
            let b = self.vertex_before(c).0;
            let oa = self.vertex_after(opp).0;
            let ob = self.vertex_before(opp).0;
            if !((a == oa && b == ob) || (a == ob && b == oa)) {
                return false;
            }
        }
        true
    }

    /// Returns the corner on the left-adjacent face.
    /// C++: Opposite(Previous(corner))
    ///
    /// Written as one lookup rather than `opposite(previous(c))` for the reason
    /// [`vertex_after`](Self::vertex_after) is: the sentinel test inside
    /// `previous` asks a question the bounds check of the lookup answers anyway.
    pub fn left_corner(&self, corner: CornerIndex) -> CornerIndex {
        self.opposite_corners
            .get(prev_in_face(corner.0) as usize)
            .copied()
            .unwrap_or(INVALID_CORNER_INDEX)
    }

    /// Returns the corner on the right-adjacent face.
    /// C++: Opposite(Next(corner))
    pub fn right_corner(&self, corner: CornerIndex) -> CornerIndex {
        self.opposite_corners
            .get(next_in_face(corner.0) as usize)
            .copied()
            .unwrap_or(INVALID_CORNER_INDEX)
    }

    /// Returns the corner on the adjacent face on the right that maps to
    /// the same vertex as the given corner.
    /// C++: Previous(Opposite(Previous(corner)))
    ///
    /// Both in-face steps drop their sentinel test here, and this is the one
    /// swing where that is free on the way out as well as on the way in:
    /// [`prev_in_face`] preserves the sentinel, so an opposite that came back
    /// invalid stays invalid through the final step. Three comparisons become
    /// one lookup, on a walk that runs for as long as a vertex has neighbours.
    pub fn swing_right(&self, corner: CornerIndex) -> CornerIndex {
        let opposite = self
            .opposite_corners
            .get(prev_in_face(corner.0) as usize)
            .copied()
            .unwrap_or(INVALID_CORNER_INDEX);
        CornerIndex(prev_in_face(opposite.0))
    }

    /// Returns the corner on the left face that maps to the same vertex as the
    /// given corner.
    /// C++: Next(Opposite(Next(corner)))
    ///
    /// The mirror of [`swing_right`](Self::swing_right) cannot drop its outer
    /// test, and the asymmetry is load-bearing rather than an oversight:
    /// [`next_in_face`] answers `u32::MAX - 2` for the sentinel, which is fine
    /// as an index -- the lookup rejects it -- but wrong as an answer, because
    /// every fan walk in this crate ends on `== INVALID_CORNER_INDEX` and
    /// `u32::MAX - 2` is not that. So the inner step fuses and the outer one
    /// keeps its branch.
    pub fn swing_left(&self, corner: CornerIndex) -> CornerIndex {
        let opposite = self
            .opposite_corners
            .get(next_in_face(corner.0) as usize)
            .copied()
            .unwrap_or(INVALID_CORNER_INDEX);
        if opposite == INVALID_CORNER_INDEX {
            return INVALID_CORNER_INDEX;
        }
        CornerIndex(next_in_face(opposite.0))
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

        // 1. Count outgoing half-edges per vertex.
        let mut num_vertices_seen = 0;
        for &v1 in &self.corner_to_vertex_map {
            if v1 == INVALID_VERTEX_INDEX {
                continue;
            }
            num_vertices_seen = num_vertices_seen.max(v1.0 as usize + 1);
        }

        let mut num_corners_on_vertices = vec![0; num_vertices_seen];
        for &v1 in &self.corner_to_vertex_map {
            if v1 == INVALID_VERTEX_INDEX {
                continue;
            }
            let v1_val = v1.0 as usize;
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
        //
        // Within a face, the corner after `local` and the one before it, as
        // tables, so the loop below needs no wrap arithmetic.
        const NEXT_LOCAL: [usize; 3] = [1, 2, 0];
        const PREV_LOCAL: [usize; 3] = [2, 0, 1];

        // Walked a face at a time: the three vertices each corner needs are its
        // own face's, so reading the triple once replaces, per corner, the wrap
        // arithmetic in `next`/`previous` and three checked map lookups. Same
        // corners in the same order. `chunks_exact` also declines to invent a
        // face out of a map whose length is not a multiple of three, which the
        // per-corner form would have walked into.
        for (face, face_base) in self
            .corner_to_vertex_map
            .chunks_exact(3)
            .map(<[VertexIndex; 3]>::try_from)
            .map(|face| face.expect("chunks_exact(3) yields three vertices"))
            .zip((0..).step_by(3))
        {
            for local in 0..3 {
                let c = face_base + local;
                let c_idx = CornerIndex(c as u32);
                let tip_v = face[local];
                let source_v = face[NEXT_LOCAL[local]];
                let sink_v = face[PREV_LOCAL[local]];

                if tip_v == source_v || source_v == sink_v || sink_v == tip_v {
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
                    let base = vertex_offset[source_v.0 as usize];
                    for offset in base..base + num_corners_on_source {
                        if vertex_edges[offset].sink_vert == INVALID_VERTEX_INDEX {
                            vertex_edges[offset].sink_vert = sink_v;
                            vertex_edges[offset].edge_corner = c_idx;
                            break;
                        }
                    }
                } else {
                    self.opposite_corners[c] = opposite_c;
                    self.opposite_corners[opposite_c.0 as usize] = c_idx;
                }
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
                debug_log!(
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
                || self.vertex_after(c0) == INVALID_VERTEX_INDEX
                || self.vertex_before(c0) == INVALID_VERTEX_INDEX
            {
                debug_log!(
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
                debug_log!("CornerTable invariant failed: vertex_corners[{}] -> corner {} maps to vertex {}", v, c.0, self.vertex(c).0);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A corner past the end of the table answers with the invalid sentinel
    /// instead of panicking.
    ///
    /// Every other accessor here funnels through these two, and the corners
    /// reaching them come from decoded connectivity: the traversal walks
    /// wherever the stream tells it to, and `is_index_consistent` -- the
    /// whole-table check that would rule this out -- is called at three sites,
    /// not at every entry point.
    #[test]
    fn a_corner_past_the_table_is_invalid_rather_than_a_panic() {
        let table = CornerTable {
            corner_to_vertex_map: vec![VertexIndex(0), VertexIndex(1), VertexIndex(2)],
            opposite_corners: vec![INVALID_CORNER_INDEX; 3],
            vertex_corners: vec![CornerIndex(0), CornerIndex(1), CornerIndex(2)],
            ..Default::default()
        };

        // In range, for contrast: these are real answers.
        assert_eq!(table.vertex(CornerIndex(2)), VertexIndex(2));
        assert_eq!(table.opposite(CornerIndex(2)), INVALID_CORNER_INDEX);

        // One past the end, and far past it.
        for corner in [CornerIndex(3), CornerIndex(u32::MAX - 1)] {
            assert_eq!(table.vertex(corner), INVALID_VERTEX_INDEX, "{corner:?}");
            assert_eq!(table.opposite(corner), INVALID_CORNER_INDEX, "{corner:?}");
        }
    }

    /// The two in-face helpers answer the public accessors on every corner, and
    /// part on the sentinel in the two different ways their callers rely on.
    #[test]
    fn in_face_helpers_agree_with_the_accessors_and_part_on_the_sentinel() {
        let table = CornerTable::default();
        for corner in 0..3000u32 {
            let c = CornerIndex(corner);
            assert_eq!(next_in_face(corner), table.next(c).0, "next {corner}");
            assert_eq!(prev_in_face(corner), table.previous(c).0, "prev {corner}");
        }

        // `previous` preserves the sentinel, so `prev_in_face` may stand in the
        // outer position of a swing.
        assert_eq!(prev_in_face(INVALID_CORNER_INDEX.0), INVALID_CORNER_INDEX.0);

        // `next_in_face` does not: it lands past any table instead, which the
        // bounds check of an inner-position `get` turns back into the sentinel.
        assert_ne!(next_in_face(INVALID_CORNER_INDEX.0), INVALID_CORNER_INDEX.0);
        assert_eq!(next_in_face(INVALID_CORNER_INDEX.0), u32::MAX - 2);
    }

    /// The six fused accessors answer exactly what the compositions they
    /// replaced answered, on every input class that reaches them: a corner in
    /// range, a corner past the end, the sentinel, and -- because a table is
    /// grown a face at a time and a malformed stream can stop it mid-face -- a
    /// table whose corner count is not a multiple of three.
    #[test]
    fn the_fused_accessors_answer_what_the_compositions_answered() {
        for corner_count in [3usize, 6, 7, 8] {
            let table = CornerTable {
                corner_to_vertex_map: (0..corner_count)
                    .map(|i| VertexIndex(i as u32 % 5))
                    .collect(),
                opposite_corners: (0..corner_count)
                    .map(|i| {
                        if i % 3 == 0 {
                            INVALID_CORNER_INDEX
                        } else {
                            CornerIndex((corner_count - i) as u32 - 1)
                        }
                    })
                    .collect(),
                vertex_corners: vec![CornerIndex(0); 5],
                ..Default::default()
            };

            let mut corners: Vec<CornerIndex> =
                (0..corner_count as u32 + 3).map(CornerIndex).collect();
            corners.push(INVALID_CORNER_INDEX);
            corners.push(CornerIndex(u32::MAX - 1));

            for c in corners {
                let what = format!("corner {c:?} of {corner_count}");
                assert_eq!(
                    table.vertex_after(c),
                    table.vertex(table.next(c)),
                    "vertex_after, {what}"
                );
                assert_eq!(
                    table.vertex_before(c),
                    table.vertex(table.previous(c)),
                    "vertex_before, {what}"
                );
                assert_eq!(
                    table.right_corner(c),
                    table.opposite(table.next(c)),
                    "right_corner, {what}"
                );
                assert_eq!(
                    table.left_corner(c),
                    table.opposite(table.previous(c)),
                    "left_corner, {what}"
                );
                assert_eq!(
                    table.swing_right(c),
                    table.previous(table.opposite(table.previous(c))),
                    "swing_right, {what}"
                );
                assert_eq!(
                    table.swing_left(c),
                    table.next(table.opposite(table.next(c))),
                    "swing_left, {what}"
                );
            }
        }
    }

    #[test]
    fn is_index_consistent_accepts_valid_and_rejects_malformed() {
        // A single triangle: 3 corners, 3 vertices, no opposites.
        let ct = CornerTable {
            corner_to_vertex_map: vec![VertexIndex(0), VertexIndex(1), VertexIndex(2)],
            opposite_corners: vec![INVALID_CORNER_INDEX; 3],
            vertex_corners: vec![CornerIndex(0), CornerIndex(1), CornerIndex(2)],
            ..Default::default()
        };
        assert!(ct.is_index_consistent());

        // Vertex index beyond num_vertices (vertex_corners.len()).
        let mut bad_vertex = ct.clone();
        bad_vertex.corner_to_vertex_map[2] = VertexIndex(3);
        assert!(!bad_vertex.is_index_consistent());

        // The sentinel is not an out-of-range index; a corner may carry it.
        let mut sentinel_vertex = ct.clone();
        sentinel_vertex.corner_to_vertex_map[1] = INVALID_VERTEX_INDEX;
        assert!(sentinel_vertex.is_index_consistent());

        // Opposite-corner index beyond num_corners.
        let mut bad_opposite = ct.clone();
        bad_opposite.opposite_corners[0] = CornerIndex(99);
        assert!(!bad_opposite.is_index_consistent());

        // Opposite map length not matching the corner count.
        let mut bad_len = ct.clone();
        bad_len.opposite_corners.pop();
        assert!(!bad_len.is_index_consistent());
    }
}
