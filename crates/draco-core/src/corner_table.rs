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

/// Exact counts of the corner table's array loads, per accessor, for comparing
/// this port against the C++ it was ported from.
///
/// Loads rather than calls, because the two sides do not agree on what a call
/// is: this port fuses `Opposite(Next(c))` into one bounds-checked lookup
/// where C++ makes three calls, so a per-method comparison of `next` or
/// `opposite` would report a difference the fusion created rather than work
/// either side does or skips. Every accessor here performs exactly one load,
/// and a load is the same event on both sides -- the metric round three of the
/// decode campaign used to put the two within 156 of each other.
///
/// Only loads made through the accessors are counted, on both sides; the
/// table's own build also indexes these arrays directly and is excluded.
///
/// Behind the off-by-default `count_table_loads` feature: the counters sit in
/// the hottest accessors in the crate, so a build carrying them is for counts
/// and never for timing.
#[cfg(feature = "count_table_loads")]
pub mod table_loads {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    /// Which array an accessor reads.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Array {
        Opposite,
        CornerToVertex,
        VertexCorners,
    }

    macro_rules! accessors {
        ($($variant:ident => ($name:literal, $array:ident),)+) => {
            /// One counted accessor.
            #[derive(Clone, Copy)]
            pub enum Accessor { $($variant,)+ }

            impl Accessor {
                pub const ALL: &'static [Accessor] = &[$(Accessor::$variant,)+];

                pub fn name(self) -> &'static str {
                    match self { $(Accessor::$variant => $name,)+ }
                }

                pub fn array(self) -> Array {
                    match self { $(Accessor::$variant => Array::$array,)+ }
                }
            }

            pub static COUNTS: [AtomicU64; Accessor::ALL.len()] =
                [$({ let _ = stringify!($variant); AtomicU64::new(0) },)+];
        };
    }

    accessors! {
        Opposite => ("opposite", Opposite),
        OppositeInternal => ("opposite (table build)", Opposite),
        LeftCorner => ("left_corner", Opposite),
        RightCorner => ("right_corner", Opposite),
        SwingLeft => ("swing_left", Opposite),
        SwingRight => ("swing_right", Opposite),
        Vertex => ("vertex", CornerToVertex),
        VertexAfter => ("vertex_after", CornerToVertex),
        VertexBefore => ("vertex_before", CornerToVertex),
        LeftMostCorner => ("left_most_corner", VertexCorners),
    }

    pub fn reset() {
        for counter in COUNTS.iter() {
            counter.store(0, Relaxed);
        }
    }

    pub fn count(accessor: Accessor) -> u64 {
        COUNTS[accessor as usize].load(Relaxed)
    }

    /// Loads of one array, summed over the accessors that read it.
    pub fn array_total(array: Array) -> u64 {
        Accessor::ALL
            .iter()
            .filter(|a| a.array() == array)
            .map(|a| count(*a))
            .sum()
    }
}

macro_rules! count_load {
    ($which:ident) => {
        #[cfg(feature = "count_table_loads")]
        {
            table_loads::COUNTS[table_loads::Accessor::$which as usize]
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    };
}

/// The stages [`CornerTable::init_with_stage_timings`] reports, in the order
/// it reports them.
#[derive(Clone, Copy, Debug)]
pub enum InitStage {
    OppositeCorners = 0,
    BreakNonManifoldEdges = 1,
    VertexCorners = 2,
}

impl InitStage {
    pub const COUNT: usize = 3;
    pub const ALL: [InitStage; Self::COUNT] = [
        InitStage::OppositeCorners,
        InitStage::BreakNonManifoldEdges,
        InitStage::VertexCorners,
    ];

    pub fn name(self) -> &'static str {
        match self {
            InitStage::OppositeCorners => "opposite_corners",
            InitStage::BreakNonManifoldEdges => "break_non_manifold",
            InitStage::VertexCorners => "vertex_corners",
        }
    }
}

/// One sink-vertex key's edges within a single pivot walk of
/// [`CornerTable::break_non_manifold_edges`]: the first two corners recorded
/// for it, and the walk they belong to.
#[derive(Clone, Copy)]
struct SinkSlot {
    stamp: u64,
    corners: [CornerIndex; 2],
    len: u8,
}

impl Default for SinkSlot {
    fn default() -> Self {
        Self {
            stamp: 0,
            corners: [INVALID_CORNER_INDEX; 2],
            len: 0,
        }
    }
}

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

    /// Reserves `vertex_corners` for `vertices`, the same way
    /// [`try_reserve_faces`](Self::try_reserve_faces) reserves the other two
    /// tables -- and for the same reason: [`set_left_most_corner`](Self::set_left_most_corner)
    /// grows this one vertex at a time, from five different symbol arms
    /// during EdgeBreaker connectivity decode, and a claimed vertex count
    /// left unreserved reallocates every time the table crosses a new power
    /// of two.
    ///
    /// `vertices` must already be bounded against the input the same way
    /// [`try_reserve_faces`](Self::try_reserve_faces)'s `faces` is -- this
    /// takes no header count on faith.
    pub fn try_reserve_vertices(
        &mut self,
        vertices: usize,
    ) -> Result<(), crate::status::DracoError> {
        let extra = vertices.saturating_sub(self.vertex_corners.len());
        if extra == 0 {
            return Ok(());
        }
        self.vertex_corners.try_reserve(extra).map_err(|_| {
            crate::status::DracoError::general("Failed to allocate corner table".to_string())
        })
    }

    /// `declared_faces` is the face count the bitstream claims. It is only a
    /// growth target, never an allocation: when growth past the current
    /// capacity is needed, the new capacity is the smaller of doubling and the
    /// declared total (but never less than what this face needs). A truthful
    /// header therefore lands the table exactly at its final size instead of
    /// overshooting by up to 2x, while a header that lies large cannot reserve
    /// more than doubling already would, and one that lies small degrades to
    /// plain doubling once the table passes it -- amortized growth either way.
    pub fn try_grow_to_face(
        &mut self,
        face: usize,
        declared_faces: usize,
    ) -> Result<(), crate::status::DracoError> {
        let num_corners = face
            .checked_add(1)
            .and_then(|faces| faces.checked_mul(3))
            .ok_or_else(|| {
                crate::status::DracoError::general("Corner table size overflow".to_string())
            })?;
        let len = self.corner_to_vertex_map.len();
        if num_corners <= len {
            return Ok(());
        }

        // The step the symbol loop actually takes: one face on, with the
        // capacity for it already reserved before the loop. `resize` reaches
        // that through `extend_with`'s element-at-a-time loop and its
        // set-length-on-drop guard, which together cost far more than the
        // twelve bytes being written -- 130 instructions per face, 13.8% of a
        // grid decode, against upstream, which fills both tables once from
        // the declared count and pays nothing per face. Extending from a
        // fixed-size array is one copy and one length update instead. The
        // general path below still handles any other step.
        if num_corners == len + 3
            && self.opposite_corners.len() == len
            && self.corner_to_vertex_map.capacity() >= num_corners
            && self.opposite_corners.capacity() >= num_corners
        {
            self.corner_to_vertex_map
                .extend_from_slice(&[INVALID_VERTEX_INDEX; 3]);
            self.opposite_corners
                .extend_from_slice(&[INVALID_CORNER_INDEX; 3]);
            return Ok(());
        }

        let declared_corners = declared_faces.saturating_mul(3);

        fn grow<T: Clone>(
            vec: &mut Vec<T>,
            num_corners: usize,
            declared_corners: usize,
            fill: T,
        ) -> Result<(), crate::status::DracoError> {
            if num_corners > vec.capacity() {
                let doubled = vec.capacity().saturating_mul(2).max(num_corners);
                let target = if vec.capacity() < declared_corners {
                    doubled.min(declared_corners).max(num_corners)
                } else {
                    doubled
                };
                vec.try_reserve_exact(target - vec.len()).map_err(|_| {
                    crate::status::DracoError::general(
                        "Failed to allocate corner table".to_string(),
                    )
                })?;
            }
            vec.resize(num_corners, fill);
            Ok(())
        }

        grow(
            &mut self.corner_to_vertex_map,
            num_corners,
            declared_corners,
            INVALID_VERTEX_INDEX,
        )?;
        grow(
            &mut self.opposite_corners,
            num_corners,
            declared_corners,
            INVALID_CORNER_INDEX,
        )
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
        self.opposite_corners[corner.0 as usize] = opposite;
    }

    pub fn init(&mut self, faces: &[[VertexIndex; 3]]) -> bool {
        self.init_inner(faces, None)
    }

    /// [`CornerTable::init`], with each stage's wall time written to `stages`
    /// in the order [`InitStage`] gives.
    ///
    /// Profiling tooling only, and deliberately the same body as `init` rather
    /// than a copy of it: a split that drifts from the path it claims to
    /// describe is worse than no split, and this stage division has already
    /// overturned one hypothesis about where the table's time goes.
    pub fn init_with_stage_timings(
        &mut self,
        faces: &[[VertexIndex; 3]],
        stages: &mut [f64; InitStage::COUNT],
    ) -> bool {
        self.init_inner(faces, Some(stages))
    }

    fn init_inner(
        &mut self,
        faces: &[[VertexIndex; 3]],
        mut stages: Option<&mut [f64; InitStage::COUNT]>,
    ) -> bool {
        // Corner `3f + i` is face `f`'s vertex `i`, which is exactly the face
        // array read as one flat run -- so the map is that run, copied once,
        // rather than a bounds-checked store per corner over a fill that every
        // one of those stores would overwrite.
        self.corner_to_vertex_map.clear();
        self.corner_to_vertex_map
            .extend_from_slice(faces.as_flattened());

        // Reading the clock costs nothing here: `stages` is `None` on every
        // path but the profiler's, and `init` runs once per encode.
        let mut started = std::time::Instant::now();
        let mut mark = |stages: &mut Option<&mut [f64; InitStage::COUNT]>, stage: InitStage| {
            if let Some(stages) = stages.as_deref_mut() {
                let now = std::time::Instant::now();
                stages[stage as usize] = now.duration_since(started).as_secs_f64() * 1e6;
                started = now;
            }
        };

        let mut num_vertices = 0;
        if !self.compute_opposite_corners(&mut num_vertices) {
            return false;
        }
        mark(&mut stages, InitStage::OppositeCorners);

        if !self.break_non_manifold_edges(num_vertices) {
            return false;
        }
        mark(&mut stages, InitStage::BreakNonManifoldEdges);

        if !self.compute_vertex_corners(num_vertices) {
            return false;
        }
        mark(&mut stages, InitStage::VertexCorners);

        // num_degenerated_faces is counted inside compute_opposite_corners,
        // against the pre-split vertex map, matching where C++ counts it.

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
        count_load!(LeftMostCorner);
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
    fn opposite_internal(&self, corner: CornerIndex) -> CornerIndex {
        count_load!(OppositeInternal);
        self.opposite_corners
            .get(corner.0 as usize)
            .copied()
            .unwrap_or(INVALID_CORNER_INDEX)
    }

    pub fn opposite(&self, corner: CornerIndex) -> CornerIndex {
        count_load!(Opposite);
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
        count_load!(Vertex);
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
        count_load!(VertexAfter);
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
        count_load!(VertexBefore);
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
            let opp = self.opposite_internal(c);
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
        count_load!(LeftCorner);
        self.opposite_corners
            .get(prev_in_face(corner.0) as usize)
            .copied()
            .unwrap_or(INVALID_CORNER_INDEX)
    }

    /// Returns the corner on the right-adjacent face.
    /// C++: Opposite(Next(corner))
    pub fn right_corner(&self, corner: CornerIndex) -> CornerIndex {
        count_load!(RightCorner);
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
        count_load!(SwingRight);
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
        count_load!(SwingLeft);
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

    /// Whether the fan around `v` fails to close: an isolated vertex, or one
    /// whose left-most corner cannot swing left. Port of C++
    /// `CornerTable::IsOnBoundary`.
    ///
    /// The definition is deliberately the cheap one rather than a walk of the
    /// whole fan looking for a missing opposite: `vertex_corners` holds the
    /// left-most corner, so a vertex is interior exactly when there is still a
    /// face further left. Three copies of this used to live in the decoder and
    /// the encoder, one of them open-coded inside a traversal; the question is
    /// about connectivity, so it belongs to the table that holds it.
    pub fn is_vertex_on_boundary(&self, v: VertexIndex) -> bool {
        let corner = self.left_most_corner(v);
        corner == INVALID_CORNER_INDEX || self.swing_left(corner) == INVALID_CORNER_INDEX
    }

    fn break_non_manifold_edges(&mut self, num_vertices: usize) -> bool {
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

        // The edges seen so far around the current pivot, keyed by sink vertex
        // rather than held as a list scanned per edge: the scan below is linear
        // in the edges already seen, so on a mesh with a high-valence hub --
        // a fan, a cone, a stitched pole -- it is quadratic in that vertex's
        // valence, which dominates the whole table build.
        //
        // Only the first two entries per key are ever needed. The scan takes
        // the first entry whose corner differs from `opp_edge_corner`, and the
        // stored corners are distinct (one per corner visited around the
        // pivot), so at most the first can be the one that gets skipped.
        //
        // `stamp` scopes the table to one pivot walk without clearing it: an
        // entry belongs to the current walk only if its stamp matches. The
        // extra slot past `num_vertices` holds the invalid vertex, which the
        // list form compared like any other key.
        // Scratch for the left-swinging pass, reused across pivots.
        let mut left_walk: Vec<CornerIndex> = Vec::new();
        let invalid_slot = num_vertices;
        let mut sink_slots = vec![SinkSlot::default(); num_vertices + 1];
        let mut walk = 0_u64;
        let slot_of = |v: VertexIndex| {
            if v == INVALID_VERTEX_INDEX {
                invalid_slot
            } else {
                v.0 as usize
            }
        };

        loop {
            let mut mesh_connectivity_updated = false;
            for c in 0..self.num_corners() {
                let c_idx = CornerIndex(c as u32);
                if visited_corners[c] {
                    continue;
                }

                walk += 1;

                // First swing all the way to find the left-most corner connected to the
                // corner's vertex.
                //
                // The corners this pass walks are exactly the ones the pass
                // below re-walks in the opposite direction, so they are
                // recorded here and replayed there rather than swung twice.
                // `swing_right` is the exact inverse of `swing_left`:
                // `previous(next(o))` is `o`, `opposite` is an involution --
                // established by `compute_opposite_corners` and preserved
                // here, which only ever clears both directions of an edge --
                // and `previous(next(x))` is `x`. On a closed 1-ring the left
                // pass covers the whole ring, so this halves the swings; on a
                // boundary one it stops at the boundary and the replay is
                // short, which is why a ribbon was already the cheapest family
                // in this function.
                let mut first_c = c_idx;
                let mut current_c = c_idx;
                left_walk.clear();
                left_walk.push(c_idx);

                loop {
                    let next_c = self.swing_left(current_c);
                    if next_c == first_c
                        || next_c == INVALID_CORNER_INDEX
                        || visited_corners[next_c.0 as usize]
                    {
                        break;
                    }
                    current_c = next_c;
                    left_walk.push(next_c);
                }

                first_c = current_c;
                // Consumed from the end: `left_walk` holds the ring from the
                // pivot corner outwards to `first_c`, which is the order the
                // right-swinging pass visits it in, reversed.
                let mut replay = left_walk.len();

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

                    // Look up the edges already seen around this pivot with the
                    // same sink vertex. Closing the loop needs no connectivity
                    // change, so an entry whose corner is `opp_edge_corner` is
                    // skipped and the next one with that sink vertex is taken.
                    let slot = &sink_slots[slot_of(sink_v)];
                    // The opposite lookup stays inside this branch, as it is in
                    // the list form: hoisting it out costs one load of
                    // `opposite_corners` per corner of the mesh, and the branch
                    // is taken only when a sink vertex repeats around the pivot.
                    let mut opp_edge_corner = INVALID_CORNER_INDEX;
                    let other_edge_corner = if slot.stamp == walk {
                        opp_edge_corner = self.opposite_internal(edge_corner);
                        if slot.corners[0] != opp_edge_corner {
                            Some(slot.corners[0])
                        } else if slot.len > 1 {
                            Some(slot.corners[1])
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(other_edge_corner) = other_edge_corner {
                        // A non-manifold edge: break the connectivity on it.
                        let opp_other_edge_corner = self.opposite_internal(other_edge_corner);
                        if opp_edge_corner != INVALID_CORNER_INDEX {
                            self.opposite_corners[opp_edge_corner.0 as usize] =
                                INVALID_CORNER_INDEX;
                        }
                        if opp_other_edge_corner != INVALID_CORNER_INDEX {
                            self.opposite_corners[opp_other_edge_corner.0 as usize] =
                                INVALID_CORNER_INDEX;
                        }

                        self.opposite_corners[edge_corner.0 as usize] = INVALID_CORNER_INDEX;
                        self.opposite_corners[other_edge_corner.0 as usize] = INVALID_CORNER_INDEX;

                        // Because of the updated connectivity, not all corners connected to
                        // this vertex have been processed and we need to go over them again.
                        //
                        // The sweep restarts from corner zero, which bounds the
                        // function at O(corners * breaks) -- but the bound is
                        // not what it costs. A sweep does not stop at the first
                        // break, it goes on breaking everything it meets, so
                        // only the corners an aborted walk left unvisited need
                        // another sweep. `visited_corners` outlives the sweep,
                        // so those restarts skip rather than redo. A manifold
                        // mesh takes one sweep; the densest folded input this
                        // has been run against takes three, and the count does
                        // not grow with the mesh. Starting a repeated sweep past
                        // the visited prefix is therefore worth two scans of a
                        // bool array, which is below what a table build can
                        // measure.
                        mesh_connectivity_updated = true;
                        break;
                    }

                    // Insert new sink vertex information <sink vertex index, edge corner>.
                    let key = self.corner_to_vertex_map[self.previous(current_c).0 as usize];
                    let slot = &mut sink_slots[slot_of(key)];
                    if slot.stamp != walk {
                        slot.stamp = walk;
                        slot.len = 0;
                    }
                    if slot.len < 2 {
                        slot.corners[slot.len as usize] = sink_c;
                        slot.len += 1;
                    }

                    if replay > 1 {
                        replay -= 1;
                        current_c = left_walk[replay - 1];
                    } else {
                        replay = 0;
                        current_c = self.swing_right(current_c);
                        if current_c == first_c || current_c == INVALID_CORNER_INDEX {
                            break;
                        }
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
        self.num_degenerated_faces = 0;
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
            .as_chunks::<3>()
            .0
            .iter()
            .copied()
            .zip((0..).step_by(3))
        {
            // A face with any repeated vertex is degenerated, and the condition
            // is symmetric in the three vertices, so one check covers all three
            // corners. Counted here, while the map still holds the vertices the
            // faces were built with -- compute_vertex_corners can split
            // non-manifold vertices afterwards, and the count written to the
            // stream header must be the pre-split one.
            if face[0] == face[1] || face[1] == face[2] || face[2] == face[0] {
                self.num_degenerated_faces += 1;
                continue;
            }
            for local in 0..3 {
                let c = face_base + local;
                let c_idx = CornerIndex(c as u32);
                let tip_v = face[local];
                let source_v = face[NEXT_LOCAL[local]];
                let sink_v = face[PREV_LOCAL[local]];

                // A vertex's half-edges occupy one contiguous run, so the search
                // and the removal below both work on that run sliced once
                // rather than indexing `vertex_edges` per entry.
                let sink_start = vertex_offset[sink_v.0 as usize];
                let sink_end = sink_start + num_corners_on_vertices[sink_v.0 as usize];

                // Search for the matching half-edge on the sink vertex, taking
                // the first match, as C++ does.
                let mut match_pos = None;
                for (i, entry) in vertex_edges[sink_start..sink_end].iter().enumerate() {
                    if entry.sink_vert == INVALID_VERTEX_INDEX {
                        break; // No matching half-edge on the sink vertex.
                    }
                    if entry.sink_vert == source_v {
                        if tip_v == self.vertex(entry.edge_corner) {
                            continue; // Don't connect mirrored faces.
                        }
                        match_pos = Some(sink_start + i);
                        break;
                    }
                }

                let Some(match_pos) = match_pos else {
                    // No opposite corner found; insert this half-edge into the
                    // first free slot on the source vertex.
                    let source_start = vertex_offset[source_v.0 as usize];
                    let source_end = source_start + num_corners_on_vertices[source_v.0 as usize];
                    for entry in vertex_edges[source_start..source_end].iter_mut() {
                        if entry.sink_vert == INVALID_VERTEX_INDEX {
                            entry.sink_vert = sink_v;
                            entry.edge_corner = c_idx;
                            break;
                        }
                    }
                    continue;
                };

                let opposite_c = vertex_edges[match_pos].edge_corner;

                // Remove the matched half-edge by shifting the entries after it
                // one slot down, stopping at the first unused one -- everything
                // past that is already unused, so shifting it would move
                // invalid over invalid. The trip count is a vertex valence, so
                // this stays a few inlined moves rather than a memmove call.
                let tail = &mut vertex_edges[match_pos..sink_end];
                let mut last = 0;
                for j in 1..tail.len() {
                    if tail[j].sink_vert == INVALID_VERTEX_INDEX {
                        break;
                    }
                    tail[last] = tail[j];
                    last = j;
                }
                tail[last].sink_vert = INVALID_VERTEX_INDEX;
                tail[last].edge_corner = INVALID_CORNER_INDEX;

                self.opposite_corners[c] = opposite_c;
                self.opposite_corners[opposite_c.0 as usize] = c_idx;
            }
        }

        *num_vertices = num_corners_on_vertices.len();
        true
    }

    /// The list-scanning form of [`CornerTable::break_non_manifold_edges`],
    /// kept as the differential oracle for the indexed one: it is upstream's
    /// shape, quadratic in vertex valence, and the two must agree on every
    /// opposite corner.
    #[cfg(test)]
    fn break_non_manifold_edges_by_list(&mut self) {
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

                loop {
                    visited_corners[current_c.0 as usize] = true;
                    let sink_c = self.next(current_c);
                    let sink_v = self.corner_to_vertex_map[sink_c.0 as usize];
                    let edge_corner = self.previous(current_c);
                    let mut vertex_connectivity_updated = false;

                    for attached_sink_vertex in &sink_vertices {
                        if attached_sink_vertex.0 == sink_v {
                            let other_edge_corner = attached_sink_vertex.1;
                            let opp_edge_corner = self.opposite_internal(edge_corner);
                            if opp_edge_corner == other_edge_corner {
                                continue;
                            }
                            let opp_other_edge_corner = self.opposite_internal(other_edge_corner);
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
                        mesh_connectivity_updated = true;
                        break;
                    }

                    sink_vertices.push((
                        self.corner_to_vertex_map[self.previous(current_c).0 as usize],
                        sink_c,
                    ));

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
            let o = self.opposite_internal(ci);
            if o != INVALID_CORNER_INDEX && self.opposite_internal(o) != ci {
                debug_log!(
                    "CornerTable invariant failed: opposite(opposite({})) != {} (got {})",
                    c,
                    c,
                    self.opposite_internal(o).0
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

    /// Random triangle soups over a handful of vertices produce exactly the
    /// folded 1-rings `break_non_manifold_edges` exists for, so the indexed
    /// form and upstream's list form are compared on every one of them.
    #[test]
    fn break_non_manifold_edges_matches_the_list_form() {
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        let mut cases_that_broke_an_edge = 0;
        for num_vertices in [4_u32, 5, 6, 8] {
            for _ in 0..250 {
                let faces: Vec<[VertexIndex; 3]> = (0..40)
                    .map(|_| {
                        [
                            VertexIndex((next() % num_vertices as u64) as u32),
                            VertexIndex((next() % num_vertices as u64) as u32),
                            VertexIndex((next() % num_vertices as u64) as u32),
                        ]
                    })
                    .collect();

                let mut indexed = CornerTable::new(faces.len());
                indexed
                    .corner_to_vertex_map
                    .extend_from_slice(faces.as_flattened());
                let mut num_vertices_seen = 0;
                assert!(indexed.compute_opposite_corners(&mut num_vertices_seen));

                let mut by_list = indexed.clone();
                let before = indexed.opposite_corners.clone();

                assert!(indexed.break_non_manifold_edges(num_vertices_seen));
                by_list.break_non_manifold_edges_by_list();

                assert_eq!(indexed.opposite_corners, by_list.opposite_corners);
                if indexed.opposite_corners != before {
                    cases_that_broke_an_edge += 1;
                }
            }
        }

        assert!(
            cases_that_broke_an_edge > 100,
            "the soups exercised the breaking path only {cases_that_broke_an_edge} times"
        );
    }

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
