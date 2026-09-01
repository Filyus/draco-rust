//! Deriving an attribute's own corner table from the position corner table.
//!
//! An interior seam is a base-connectivity edge where the attribute's value
//! differs on its two sides -- the encoder finds these by comparing attribute
//! values around each edge, the decoder by reading the seam bits it just
//! decoded. Either way, turning "these edges are seams" into "here is the
//! per-attribute corner table, with its own vertex ids split at every seam" is
//! one computation, and [`cut_seam_edges_and_recompute_vertices`] is the only
//! place it happens: `mesh_encoder.rs` and `mesh_decoder.rs` both call it
//! rather than each recomputing vertices its own way. Upstream keeps the same
//! logic in one class, `MeshAttributeCornerTable`; this keeps it in one
//! function instead of porting the class.

use crate::corner_table::CornerTable;
use crate::geometry_indices::{
    CornerIndex, VertexIndex, INVALID_CORNER_INDEX, INVALID_VERTEX_INDEX,
};
use crate::status::DracoError;

/// Cuts every marked seam edge out of a clone of `base_ct` and gives each
/// resulting fan piece its own vertex id.
///
/// `is_edge_on_seam` is indexed by corner and must mark both sides of a cut
/// edge: a seam corner and, when it has one, its opposite. The caller decides
/// how it learns that -- from attribute-value comparisons on the encode side,
/// from bitstream bits on the decode side -- this function only needs the
/// result.
///
/// Returns the attribute's corner table together with, per *base* vertex,
/// whether it sits on a seam (so more than one attribute-vertex can answer for
/// it). A malformed seam pattern that made a fan cycle without ever reaching a
/// cut is refused rather than looped on forever: only the decoder can be
/// handed such a pattern, since the encoder's own seam detection cannot
/// produce one, but the guard costs nothing on that side either.
pub fn cut_seam_edges_and_recompute_vertices(
    base_ct: &CornerTable,
    is_edge_on_seam: &[bool],
) -> Result<(CornerTable, Vec<bool>), DracoError> {
    let mut ct = base_ct.clone();
    for c_idx in 0..base_ct.num_corners() {
        if is_edge_on_seam[c_idx] {
            ct.set_opposite(CornerIndex(c_idx as u32), INVALID_CORNER_INDEX);
        }
    }

    let mut is_vertex_on_seam = vec![false; base_ct.num_vertices()];
    for c_idx in 0..base_ct.num_corners() {
        if !is_edge_on_seam[c_idx] {
            continue;
        }
        let c = CornerIndex(c_idx as u32);
        let next_vertex = base_ct.vertex_after(c);
        if next_vertex != INVALID_VERTEX_INDEX {
            is_vertex_on_seam[next_vertex.0 as usize] = true;
        }
        let previous_vertex = base_ct.vertex_before(c);
        if previous_vertex != INVALID_VERTEX_INDEX {
            is_vertex_on_seam[previous_vertex.0 as usize] = true;
        }
    }

    // A seam-aware swing, taken over the *uncut* base table: it walks past a
    // seam like any other edge, and the caller renumbers at the crossing
    // instead of stopping there. One continuous pass then covers a vertex
    // with any number of seam edges around it, not just the common one-cut
    // case -- `ct.swing_left`, which stops dead at the first cut, cannot.
    let seam_swing_left = |corner: CornerIndex| -> CornerIndex {
        let opposite = base_ct.next(corner);
        let opposite = if is_edge_on_seam
            .get(opposite.0 as usize)
            .copied()
            .unwrap_or(false)
        {
            INVALID_CORNER_INDEX
        } else {
            base_ct.opposite(opposite)
        };
        base_ct.next(opposite)
    };

    ct.corner_to_vertex_map.fill(INVALID_VERTEX_INDEX);
    ct.vertex_corners.clear();

    let max_swing_steps = base_ct.num_corners().saturating_add(1);
    let mut num_new_vertices = 0usize;
    for v in 0..base_ct.num_vertices() {
        let c = base_ct.left_most_corner(VertexIndex(v as u32));
        if c == INVALID_CORNER_INDEX {
            continue;
        }

        let mut first_vertex_id = VertexIndex(num_new_vertices as u32);
        num_new_vertices += 1;

        let mut first_c = c;
        if is_vertex_on_seam[v] {
            let mut act_c = seam_swing_left(first_c);
            let mut swing_steps = 0usize;
            while act_c != INVALID_CORNER_INDEX {
                swing_steps += 1;
                if swing_steps > max_swing_steps {
                    return Err(DracoError::general(
                        "Attribute seam left-swing traversal did not terminate".to_string(),
                    ));
                }
                first_c = act_c;
                act_c = seam_swing_left(act_c);
            }
        }

        ct.corner_to_vertex_map[first_c.0 as usize] = first_vertex_id;
        ct.vertex_corners.push(first_c);

        let mut act_c = base_ct.swing_right(first_c);
        let mut swing_steps = 0usize;
        while act_c != INVALID_CORNER_INDEX && act_c != first_c {
            swing_steps += 1;
            if swing_steps > max_swing_steps {
                return Err(DracoError::general(
                    "Attribute seam right-swing traversal did not terminate".to_string(),
                ));
            }
            if is_edge_on_seam[base_ct.next(act_c).0 as usize] {
                first_vertex_id = VertexIndex(num_new_vertices as u32);
                num_new_vertices += 1;
                ct.vertex_corners.push(act_c);
            }
            ct.corner_to_vertex_map[act_c.0 as usize] = first_vertex_id;
            act_c = base_ct.swing_right(act_c);
        }
    }

    ct.num_original_vertices = ct.vertex_corners.len();
    ct.num_isolated_vertices = 0;
    ct.num_degenerated_faces = base_ct.num_degenerated_faces;

    Ok((ct, is_vertex_on_seam))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Six triangles fanned around a shared centre vertex 0, ring vertices
    /// 1..=6 in order. `spoke(j)` is the edge `(0, 1+j)`, shared by `face(j-1)`
    /// and `face(j)` (indices mod 6) -- the only edge those two faces have in
    /// common, so cutting it also splits the ring vertex `1+j`, not only the
    /// centre.
    fn hexagon_fan() -> CornerTable {
        let faces: Vec<[VertexIndex; 3]> = (0..6u32)
            .map(|j| {
                let a = 1 + j;
                let b = 1 + (j + 1) % 6;
                [VertexIndex(0), VertexIndex(a), VertexIndex(b)]
            })
            .collect();
        let mut ct = CornerTable::new(faces.len());
        assert!(ct.init(&faces));
        ct
    }

    /// The corner in `face(j)` that is *not* on `spoke(j)` -- marking it seam
    /// cuts that spoke from `face(j)`'s side.
    fn spoke_opposite_corner(j: u32) -> CornerIndex {
        CornerIndex(3 * j + 2)
    }

    /// Cutting three of the six spokes, spaced one apart, must split the
    /// centre vertex into exactly the three arcs the cuts leave behind -- not
    /// two (an algorithm that only handles a single cut correctly) and not
    /// six (one that always isolates every corner it touches).
    ///
    /// This is the case `ct.swing_left`/`ct.swing_right` on a once-cut clone
    /// cannot answer directly: a vertex can have any number of seam edges
    /// around it, and [`cut_seam_edges_and_recompute_vertices`] handles that
    /// by walking the uncut fan once and renumbering at every crossing, rather
    /// than cutting and re-swinging per piece.
    #[test]
    fn a_vertex_with_three_seam_edges_splits_into_three_groups() {
        let base_ct = hexagon_fan();
        let mut is_edge_on_seam = vec![false; base_ct.num_corners()];
        for &j in &[0u32, 2, 4] {
            let c = spoke_opposite_corner(j);
            is_edge_on_seam[c.0 as usize] = true;
            let opp = base_ct.opposite(c);
            assert_ne!(opp, INVALID_CORNER_INDEX, "spoke {j} should be interior");
            is_edge_on_seam[opp.0 as usize] = true;
        }

        let (attr_ct, is_vertex_on_seam) =
            cut_seam_edges_and_recompute_vertices(&base_ct, &is_edge_on_seam)
                .expect("a well-formed cut must not fail");

        // The centre corner of each face, grouped by the uncut spokes between
        // them: (face0,face1), (face2,face3), (face4,face5).
        let centre = |face: u32| attr_ct.vertex(CornerIndex(3 * face));
        let (g01, g23, g45) = (centre(0), centre(2), centre(4));
        assert_eq!(centre(0), centre(1), "face0/face1 share an uncut spoke");
        assert_eq!(centre(2), centre(3), "face2/face3 share an uncut spoke");
        assert_eq!(centre(4), centre(5), "face4/face5 share an uncut spoke");
        assert_ne!(g01, g23, "the cut at spoke2 must separate these arcs");
        assert_ne!(g23, g45, "the cut at spoke4 must separate these arcs");
        assert_ne!(g01, g45, "the cut at spoke0 must separate these arcs");

        // A ring vertex touches exactly two faces through exactly one edge
        // (its spoke), so it splits in two precisely when that spoke is cut.
        let ring_pair = |j: u32| {
            let prev_face = (j + 5) % 6; // j - 1, mod 6
            (
                attr_ct.vertex(CornerIndex(3 * prev_face + 2)),
                attr_ct.vertex(CornerIndex(3 * j + 1)),
            )
        };
        for &j in &[0u32, 2, 4] {
            let (a, b) = ring_pair(j);
            assert_ne!(a, b, "ring vertex {} sits on the cut spoke {j}", 1 + j);
        }
        for &j in &[1u32, 3, 5] {
            let (a, b) = ring_pair(j);
            assert_eq!(a, b, "ring vertex {} sits on an uncut spoke {j}", 1 + j);
        }

        assert!(is_vertex_on_seam[0], "the centre sits on three cuts");
        for j in [0u32, 2, 4] {
            assert!(
                is_vertex_on_seam[(1 + j) as usize],
                "ring vertex {} sits on cut spoke {j}",
                1 + j
            );
        }
        for j in [1u32, 3, 5] {
            assert!(
                !is_vertex_on_seam[(1 + j) as usize],
                "ring vertex {} touches only uncut spokes",
                1 + j
            );
        }
    }

    /// A one-sided seam mark -- the kind a corrupted or adversarial bitstream
    /// can produce, since the decoder trusts the seam corners it reads rather
    /// than deriving them from real attribute values -- can flag a vertex as
    /// seam-bound without actually cutting anything on its fan. Two
    /// disconnected faces share vertex id 0 here (a non-manifold corner table,
    /// which the position corner table can itself produce): face A's own
    /// opposite pointers are corrupted into a closed 3-cycle that never
    /// reaches a real cut or an invalid corner, while the seam mark lives on
    /// face B, unreachable from face A's fan walk. Left unguarded, the
    /// left-swing loops forever; this checks it is refused instead.
    #[test]
    fn a_seam_mark_unreachable_from_a_corrupted_fan_is_refused_not_looped_on() {
        let base_ct = CornerTable {
            // Face A: corners 0,1,2 at vertices 0,1,2. Face B: corners 3,4,5
            // at vertices 0,3,4 -- sharing vertex 0 with face A, but not
            // linked to it by any opposite pointer.
            corner_to_vertex_map: [0, 1, 2, 0, 3, 4].map(VertexIndex).to_vec(),
            // Face A's pointers form a closed cycle among themselves instead
            // of the real (all-INVALID) topology of two disjoint triangles --
            // `next` never sees an invalid corner and never revisits corner 0
            // by construction. Face B's are irrelevant to the walk and left
            // invalid.
            opposite_corners: vec![
                CornerIndex(1),
                CornerIndex(2),
                CornerIndex(0),
                INVALID_CORNER_INDEX,
                INVALID_CORNER_INDEX,
                INVALID_CORNER_INDEX,
            ],
            vertex_corners: vec![
                CornerIndex(0),
                CornerIndex(1),
                CornerIndex(2),
                CornerIndex(4),
                CornerIndex(5),
            ],
            num_original_vertices: 5,
            num_degenerated_faces: 0,
            num_isolated_vertices: 0,
        };

        // Corner 5 (face B, vertex 4) is opposite the edge (vertex 0, vertex
        // 3) -- marking it seam flags vertex 0 as seam-bound without cutting
        // anything face A's own fan walk will ever reach.
        let mut is_edge_on_seam = vec![false; base_ct.num_corners()];
        is_edge_on_seam[5] = true;

        let result = cut_seam_edges_and_recompute_vertices(&base_ct, &is_edge_on_seam);
        assert!(
            result.is_err(),
            "a fan that never reaches a cut or an invalid corner must be refused"
        );
    }
}
