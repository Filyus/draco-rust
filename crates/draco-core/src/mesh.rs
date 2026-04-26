use crate::geometry_indices::{FaceIndex, PointIndex};
use crate::point_cloud::PointCloud;
use crate::status::{DracoError, Status};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

pub type Face = [PointIndex; 3];

#[derive(Debug, Default, Clone)]
pub struct Mesh {
    point_cloud: PointCloud,
    faces: Vec<Face>,
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_face(&mut self, face: Face) {
        self.faces.push(face);
    }

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
        for (face, chunk) in self.faces.iter_mut().zip(bytes.chunks_exact(3)) {
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
        for (face, chunk) in self.faces.iter_mut().zip(bytes.chunks_exact(6)) {
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
        for (face, chunk) in self.faces.iter_mut().zip(bytes.chunks_exact(12)) {
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

    pub fn face(&self, face_id: FaceIndex) -> Face {
        self.faces[face_id.0 as usize]
    }

    pub fn num_faces(&self) -> usize {
        self.faces.len()
    }

    pub fn set_num_faces(&mut self, num_faces: usize) {
        self.faces.resize(num_faces, [PointIndex(0); 3]);
    }

    pub fn try_set_num_faces(&mut self, num_faces: usize) -> Status {
        if num_faces > self.faces.len() {
            self.faces
                .try_reserve_exact(num_faces - self.faces.len())
                .map_err(|_| DracoError::DracoError("Failed to allocate mesh faces".to_string()))?;
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
                if !old_to_new.contains_key(&point_idx.0) {
                    old_to_new.insert(point_idx.0, new_id);
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

            // Update attribute buffer - resize and write the new data
            let att_mut = self.attribute_mut(att_idx);
            att_mut.buffer_mut().resize(new_buffer.len());
            att_mut.buffer_mut().write(0, &new_buffer);
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
