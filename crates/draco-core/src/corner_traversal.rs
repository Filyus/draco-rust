//! Depth-first traversal of a corner table.
//!
//! Port of Draco's `DepthFirstTraverser::TraverseFromCorner`. Both decode paths
//! that order attribute values by connectivity -- the generic mesh decoder and
//! the EdgeBreaker decoder's point assignment -- walk faces in exactly this
//! order, and for a long time each carried its own copy of the walk. The copies
//! were identical apart from what they recorded per newly visited vertex, which
//! is what the observer argument is for.

use crate::corner_table::CornerTable;
use crate::geometry_indices::{
    CornerIndex, VertexIndex, INVALID_CORNER_INDEX, INVALID_FACE_INDEX, INVALID_VERTEX_INDEX,
};

/// Walks the connected component reachable from `start_corner`, marking faces
/// and vertices visited and calling `on_new_vertex` once per vertex, in
/// discovery order, with the corner it was discovered through.
///
/// Returns immediately if the start face is invalid or already visited, so a
/// caller can drive this over every face in order and let it skip the ones it
/// already covered.
///
/// The observer is a closure rather than a trait object on purpose: it runs once
/// per vertex -- two hundred thousand times on a mesh the size of the Stanford
/// Bunny -- and a monomorphised closure inlines into the walk where a vtable
/// would put an indirect call in the middle of it.
pub(crate) fn traverse_from_corner(
    corner_table: &CornerTable,
    start_corner: CornerIndex,
    visited_faces: &mut [bool],
    visited_vertices: &mut [bool],
    on_new_vertex: &mut impl FnMut(VertexIndex, CornerIndex),
) {
    let start_face = corner_table.face(start_corner);
    if start_face == INVALID_FACE_INDEX || visited_faces[start_face.0 as usize] {
        return;
    }

    let mut corner_stack = vec![start_corner];

    // The first face's other two corners may never be reached by the walk
    // itself, so C++ visits Next and then Previous before the main loop -- and
    // not the tip vertex, which the loop covers.
    let next_corner = corner_table.next(start_corner);
    let prev_corner = corner_table.previous(start_corner);
    let next_vert = corner_table.vertex(next_corner);
    let prev_vert = corner_table.vertex(prev_corner);
    if next_vert == INVALID_VERTEX_INDEX || prev_vert == INVALID_VERTEX_INDEX {
        return;
    }

    for (vertex, corner) in [(next_vert, next_corner), (prev_vert, prev_corner)] {
        if !visited_vertices[vertex.0 as usize] {
            visited_vertices[vertex.0 as usize] = true;
            on_new_vertex(vertex, corner);
        }
    }

    while let Some(mut corner_id) = corner_stack.pop() {
        let mut face_id = corner_table.face(corner_id);
        if corner_id == INVALID_CORNER_INDEX || visited_faces[face_id.0 as usize] {
            continue;
        }

        loop {
            visited_faces[face_id.0 as usize] = true;

            let vert_id = corner_table.vertex(corner_id);
            if vert_id == INVALID_VERTEX_INDEX {
                break;
            }

            if !visited_vertices[vert_id.0 as usize] {
                let on_boundary = corner_table.is_vertex_on_boundary(vert_id);
                visited_vertices[vert_id.0 as usize] = true;
                on_new_vertex(vert_id, corner_id);

                if !on_boundary {
                    corner_id = corner_table.right_corner(corner_id);
                    if corner_id == INVALID_CORNER_INDEX {
                        break;
                    }
                    face_id = corner_table.face(corner_id);
                    continue;
                }
            }

            // The vertex was already visited, or it sits on a boundary. Either
            // way the walk cannot continue through it, so look at the two
            // neighbouring faces instead.
            let right_corner_id = corner_table.right_corner(corner_id);
            let left_corner_id = corner_table.left_corner(corner_id);

            let right_face_id = if right_corner_id == INVALID_CORNER_INDEX {
                INVALID_FACE_INDEX
            } else {
                corner_table.face(right_corner_id)
            };
            let left_face_id = if left_corner_id == INVALID_CORNER_INDEX {
                INVALID_FACE_INDEX
            } else {
                corner_table.face(left_corner_id)
            };

            let right_visited =
                right_face_id == INVALID_FACE_INDEX || visited_faces[right_face_id.0 as usize];
            let left_visited =
                left_face_id == INVALID_FACE_INDEX || visited_faces[left_face_id.0 as usize];

            if right_visited {
                if left_visited {
                    // Both neighbours are done: this end of the walk is finished.
                    break;
                }
                corner_id = left_corner_id;
                face_id = left_face_id;
            } else if left_visited {
                corner_id = right_corner_id;
                face_id = right_face_id;
            } else {
                // Both are unvisited, so the walk splits. The right face is
                // pushed last and therefore taken first; the left one waits.
                corner_stack.push(left_corner_id);
                corner_stack.push(right_corner_id);
                break;
            }
        }
    }
}
