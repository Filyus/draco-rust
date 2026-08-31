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
use crate::geometry_indices::{CornerIndex, VertexIndex, INVALID_CORNER_INDEX, INVALID_VERTEX_INDEX};
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
