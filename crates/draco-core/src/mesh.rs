use crate::geometry_indices::{FaceIndex, PointIndex, VertexIndex};
use crate::point_cloud::PointCloud;
use crate::status::{DracoError, Status};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

/// Triangle face represented by three point indices.
pub type Face = [PointIndex; 3];

/// Triangle mesh geometry decoded from, or prepared for, a Draco bitstream.
///
/// A mesh owns triangle topology and dereferences to its underlying
/// [`PointCloud`], where attributes and metadata are stored.
#[derive(Debug, Default, Clone)]
pub struct Mesh {
    point_cloud: PointCloud,
    faces: Vec<Face>,
}

impl Mesh {
    /// Creates an empty mesh with no faces, points, attributes, or metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one triangle face.
    pub fn add_face(&mut self, face: Face) {
        self.faces.push(face);
    }

    /// Sets a face, growing the face list with zeroed faces when needed.
    pub fn set_face(&mut self, face_id: FaceIndex, face: Face) {
        if face_id.0 as usize >= self.faces.len() {
            self.faces
                .resize(face_id.0 as usize + 1, [PointIndex(0); 3]);
        }
        self.faces[face_id.0 as usize] = face;
    }

    /// Bulk-set all faces from a flat u32 index array (3 indices per face).
    /// Assumes `set_num_faces` has already been called with the right count.
    #[inline]
    /// Fills every face from a corner table's corner-to-vertex map.
    ///
    /// The map lays the three corners of face `f` at `3f..3f + 3`, in the
    /// order a face stores them, so an edgebreaker decode without attribute
    /// seams -- where a corner-table vertex index *is* a point index -- is a
    /// straight copy. Reading it back through `vertex`/`vertex_after`/
    /// `vertex_before` instead costs three bounds-checked `Option` lookups
    /// and two modular corner computations per face for indices already
    /// known to be consecutive: 51 instructions per face against upstream's
    /// 23, on a table whose bounds the caller's consistency scan has just
    /// proved.
    pub fn set_faces_from_corner_vertices(&mut self, corner_to_vertex_map: &[VertexIndex]) {
        let (corners_per_face, _) = corner_to_vertex_map.as_chunks::<3>();
        // Matches `set_face`, which grows rather than refusing a face past
        // the end; the edgebreaker caller has already sized the mesh to the
        // table, so this is a fallback and not the path taken.
        if self.faces.len() < corners_per_face.len() {
            self.faces
                .resize(corners_per_face.len(), [PointIndex(0); 3]);
        }
        for (face, corners) in self.faces.iter_mut().zip(corners_per_face) {
            *face = [
                PointIndex(corners[0].0),
                PointIndex(corners[1].0),
                PointIndex(corners[2].0),
            ];
        }
    }

    pub fn set_faces_from_flat_indices(&mut self, indices: &[u32]) {
        debug_assert_eq!(indices.len(), self.faces.len() * 3);
        for (i, face) in self.faces.iter_mut().enumerate() {
            let base = i * 3;
            *face = [
                PointIndex(indices[base]),
                PointIndex(indices[base + 1]),
                PointIndex(indices[base + 2]),
            ];
        }
    }

    /// Bulk-set all faces from tightly packed u8 indices.
    /// Assumes `set_num_faces` has already been called with the right count.
    #[inline]
    pub fn set_faces_from_u8_indices(&mut self, bytes: &[u8]) {
        debug_assert_eq!(bytes.len(), self.faces.len() * 3);
        for (face, chunk) in self.faces.iter_mut().zip(bytes.as_chunks::<3>().0) {
            *face = [
                PointIndex(chunk[0] as u32),
                PointIndex(chunk[1] as u32),
                PointIndex(chunk[2] as u32),
            ];
        }
    }

    /// Bulk-set all faces from tightly packed little-endian u16 indices.
    /// Assumes `set_num_faces` has already been called with the right count.
    #[inline]
    pub fn set_faces_from_le_u16_indices(&mut self, bytes: &[u8]) {
        debug_assert_eq!(bytes.len(), self.faces.len() * 3 * 2);
        for (face, chunk) in self.faces.iter_mut().zip(bytes.as_chunks::<6>().0) {
            *face = [
                PointIndex(u16::from_le_bytes([chunk[0], chunk[1]]) as u32),
                PointIndex(u16::from_le_bytes([chunk[2], chunk[3]]) as u32),
                PointIndex(u16::from_le_bytes([chunk[4], chunk[5]]) as u32),
            ];
        }
    }

    /// Bulk-set all faces from tightly packed little-endian u32 indices.
    /// Assumes `set_num_faces` has already been called with the right count.
    #[inline]
    pub fn set_faces_from_le_u32_indices(&mut self, bytes: &[u8]) {
        debug_assert_eq!(bytes.len(), self.faces.len() * 3 * 4);
        for (face, chunk) in self.faces.iter_mut().zip(bytes.as_chunks::<12>().0) {
            *face = [
                PointIndex(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])),
                PointIndex(u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]])),
                PointIndex(u32::from_le_bytes([
                    chunk[8], chunk[9], chunk[10], chunk[11],
                ])),
            ];
        }
    }

    /// Sets one face from raw u32 point ids.
    #[inline]
    pub fn set_face_from_indices(&mut self, face_id: usize, indices: [u32; 3]) {
        self.faces[face_id] = [
            PointIndex(indices[0]),
            PointIndex(indices[1]),
            PointIndex(indices[2]),
        ];
    }

    /// Returns the point indices for a face.
    pub fn face(&self, face_id: FaceIndex) -> Face {
        self.faces[face_id.0 as usize]
    }

    /// Returns the number of triangle faces.
    pub fn num_faces(&self) -> usize {
        self.faces.len()
    }

    /// Resizes the face list, filling new faces with point index zero.
    pub fn set_num_faces(&mut self, num_faces: usize) {
        self.faces.resize(num_faces, [PointIndex(0); 3]);
    }

    /// Fallibly resizes the face list.
    pub fn try_set_num_faces(&mut self, num_faces: usize) -> Status {
        if num_faces > self.faces.len() {
            self.faces
                .try_reserve_exact(num_faces - self.faces.len())
                .map_err(|_| DracoError::general("Failed to allocate mesh faces".to_string()))?;
        }
        self.faces.resize(num_faces, [PointIndex(0); 3]);
        Ok(())
    }

    /// Deduplicate point IDs to match C++ Draco behavior.
    ///
    /// This function remaps point indices such that:
    /// 1. Points are assigned new IDs in the order they're first encountered in faces
    /// 2. Face indices are updated to use the new point IDs
    /// 3. Attribute point mappings are updated accordingly
    ///
    /// This is needed for binary compatibility with C++ Draco, which internally
    /// creates separate points for each face corner during OBJ loading and then
    /// deduplicates them in face-traversal order.
    pub fn deduplicate_point_ids(&mut self) {
        if self.faces.is_empty() || self.num_points() == 0 {
            return;
        }

        // Build mapping from old point ID to new point ID
        // Points are assigned new IDs in the order they're first seen in faces
        let mut old_to_new: HashMap<u32, u32> = HashMap::new();
        let mut new_id = 0u32;

        // First pass: determine the mapping
        for face in &self.faces {
            for &point_idx in face.iter() {
                if let std::collections::hash_map::Entry::Vacant(e) = old_to_new.entry(point_idx.0)
                {
                    e.insert(new_id);
                    new_id += 1;
                }
            }
        }

        // If no remapping needed (already in correct order), skip
        let needs_remap = old_to_new.iter().any(|(&old, &new)| old != new);
        if !needs_remap {
            return;
        }

        // Build reverse mapping for reordering attributes
        let num_unique = new_id as usize;
        let mut new_to_old = vec![0u32; num_unique];
        for (&old, &new) in &old_to_new {
            new_to_old[new as usize] = old;
        }

        // Second pass: update face indices
        for face in &mut self.faces {
            for point_idx in face.iter_mut() {
                point_idx.0 = old_to_new[&point_idx.0];
            }
        }

        // Third pass: reorder attribute data
        // For each attribute, create new buffer with data in new order
        for att_idx in 0..self.num_attributes() {
            let att = self.attribute(att_idx);
            let stride = att.byte_stride() as usize;
            let old_buffer = att.buffer().data().to_vec();

            // Create new buffer with reordered data
            let mut new_buffer = vec![0u8; num_unique * stride];
            for new_idx in 0..num_unique {
                let old_idx = new_to_old[new_idx] as usize;
                if old_idx * stride + stride <= old_buffer.len() {
                    new_buffer[new_idx * stride..new_idx * stride + stride]
                        .copy_from_slice(&old_buffer[old_idx * stride..old_idx * stride + stride]);
                }
            }

            // Update the attribute through `resize_unique_entries` rather than
            // resizing its buffer directly: the buffer is only half of an
            // attribute's size, and leaving `size()` at the pre-dedup count
            // makes the attribute claim entries its buffer no longer holds.
            // Anything that walks the attribute by `size()` then reads past the
            // end -- reachable from any mesh with vertices no face references,
            // which is ordinary in scanned geometry.
            let att_mut = self.attribute_mut(att_idx);
            if att_mut.resize_unique_entries(num_unique).is_ok() {
                att_mut.buffer_mut().write(0, &new_buffer);
            }
        }

        // Update point count
        self.set_num_points(num_unique);
    }
}

impl Deref for Mesh {
    type Target = PointCloud;

    fn deref(&self) -> &Self::Target {
        &self.point_cloud
    }
}

impl DerefMut for Mesh {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.point_cloud
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draco_types::DataType;
    use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};

    /// A mesh whose vertices are not all referenced by faces -- ordinary in
    /// scanned geometry, where the raw point set outlives the triangulation.
    ///
    /// Deduplication drops the unreferenced ones, and every attribute has to
    /// come away describing the points that are left. It used to rewrite the
    /// buffer but leave `size()` at the old count, so the attribute claimed
    /// entries whose bytes were gone and readers walked off the end.
    #[test]
    fn deduplicate_point_ids_shrinks_attribute_size_with_its_buffer() {
        let mut mesh = Mesh::new();
        let num_points = 5;
        mesh.set_num_points(num_points);
        mesh.set_num_faces(1);

        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            num_points,
        );
        for point in 0..num_points {
            for component in 0..3 {
                let value = (point * 3 + component) as f32;
                attribute
                    .buffer_mut()
                    .update(&value.to_le_bytes(), Some((point * 3 + component) * 4));
            }
        }
        mesh.add_attribute(attribute);

        // Only three of the five points are reachable through a face.
        mesh.set_face(FaceIndex(0), [PointIndex(4), PointIndex(2), PointIndex(0)]);

        mesh.deduplicate_point_ids();

        assert_eq!(
            mesh.num_points(),
            3,
            "unreferenced points should be dropped"
        );
        let attribute = mesh.attribute(0);
        assert_eq!(
            attribute.size(),
            3,
            "attribute still claims entries it no longer stores"
        );
        assert_eq!(
            attribute.buffer().data().len(),
            3 * attribute.byte_stride() as usize,
            "buffer and size disagree"
        );

        // The surviving values must be the ones the faces pointed at, in the
        // order the faces first reach them.
        let read = |entry: usize| -> f32 {
            let offset = entry * attribute.byte_stride() as usize;
            f32::from_le_bytes(
                attribute.buffer().data()[offset..offset + 4]
                    .try_into()
                    .unwrap(),
            )
        };
        assert_eq!(read(0), 12.0, "first face corner was old point 4");
        assert_eq!(read(1), 6.0, "second face corner was old point 2");
        assert_eq!(read(2), 0.0, "third face corner was old point 0");
    }
}
