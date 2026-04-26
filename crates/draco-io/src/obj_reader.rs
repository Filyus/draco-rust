//! OBJ format reader for meshes and point clouds.
//!
//! Provides both a struct-based API (`ObjReader`) and convenience functions.

use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::mesh::Mesh;

use crate::traits::{PointCloudReader, Reader};

/// OBJ format reader.
///
/// Reads vertex positions and faces from OBJ files.
#[derive(Debug)]
pub struct ObjReader {
    path: std::path::PathBuf,
}

impl ObjReader {
    /// Open an OBJ file for reading.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found: {}", path.display()),
            ));
        }
        Ok(Self { path })
    }

    /// Read all positions from the OBJ file.
    pub fn read_positions(&mut self) -> io::Result<Vec<[f32; 3]>> {
        read_obj_positions(&self.path)
    }

    /// Read positions and faces from the OBJ file.
    fn read_positions_and_faces(&self) -> io::Result<(Vec<[f32; 3]>, Vec<[u32; 3]>)> {
        let file = fs::File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut positions = Vec::new();
        let mut faces = Vec::new();

        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();

            if trimmed.starts_with("vn ") || trimmed.starts_with("vt ") {
                continue;
            }

            if trimmed.starts_with("v ") {
                let mut parts = trimmed.split_whitespace();
                parts.next(); // skip 'v'

                let x = parts.next().and_then(|s| s.parse().ok());
                let y = parts.next().and_then(|s| s.parse().ok());
                let z = parts.next().and_then(|s| s.parse().ok());

                if let (Some(x), Some(y), Some(z)) = (x, y, z) {
                    positions.push([x, y, z]);
                }
            } else if trimmed.starts_with("f ") {
                let mut parts = trimmed.split_whitespace();
                parts.next(); // skip 'f'

                // Parse face indices (format: v or v/vt or v/vt/vn or v//vn)
                let parse_vertex = |s: &str| -> Option<u32> {
                    // Take only the first number (vertex index), ignore texture/normal indices
                    let idx_str = s.split('/').next()?;
                    idx_str.parse::<u32>().ok()
                };

                let v0 = parts.next().and_then(parse_vertex);
                let v1 = parts.next().and_then(parse_vertex);
                let v2 = parts.next().and_then(parse_vertex);

                // OBJ uses 1-based indices, convert to 0-based
                if let (Some(v0), Some(v1), Some(v2)) = (v0, v1, v2) {
                    faces.push([v0 - 1, v1 - 1, v2 - 1]);
                }
            }
        }

        Ok((positions, faces))
    }

    /// Read a mesh with positions and faces (if present).
    ///
    /// The mesh is automatically processed to match C++ Draco OBJ loader behavior:
    /// point IDs are deduplicated in face-traversal order, which ensures binary
    /// compatibility when encoding with sequential encoding (speed 10).
    pub fn read_mesh(&mut self) -> io::Result<Mesh> {
        let (positions, faces) = self.read_positions_and_faces()?;
        let mut mesh = Mesh::new();

        if positions.is_empty() {
            return Ok(mesh);
        }

        mesh.set_num_points(positions.len());
        mesh.set_num_faces(faces.len());

        // Create position attribute
        let mut pos_att = PointAttribute::new();
        pos_att.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            positions.len(),
        );

        let buffer = pos_att.buffer_mut();
        for (i, pos) in positions.iter().enumerate() {
            let bytes: Vec<u8> = pos.iter().flat_map(|v| v.to_le_bytes()).collect();
            buffer.write(i * 12, &bytes);
        }

        mesh.add_attribute(pos_att);

        // Set faces
        use draco_core::geometry_indices::{FaceIndex, PointIndex};
        for (i, face) in faces.iter().enumerate() {
            mesh.set_face(
                FaceIndex(i as u32),
                [
                    PointIndex(face[0]),
                    PointIndex(face[1]),
                    PointIndex(face[2]),
                ],
            );
        }

        // Match C++ OBJ loader behavior: deduplicate point IDs in face-traversal order.
        // C++ creates separate points for each face corner then deduplicates them,
        // assigning new IDs in the order points are first encountered in faces.
        mesh.deduplicate_point_ids();

        Ok(mesh)
    }
}

impl Reader for ObjReader {
    fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        ObjReader::open(path)
    }

    fn read_meshes(&mut self) -> io::Result<Vec<Mesh>> {
        let m = self.read_mesh()?;
        Ok(vec![m])
    }
}

impl crate::traits::SceneReader for ObjReader {
    fn read_scene(&mut self) -> io::Result<crate::traits::Scene> {
        let meshes = self.read_meshes()?;
        let mut parts = Vec::with_capacity(meshes.len());
        let mut root = crate::traits::SceneNode::new(
            self.path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string()),
        );
        for mesh in meshes {
            let part = crate::traits::SceneObject {
                name: None,
                mesh: mesh.clone(),
                transform: None,
            };
            root.parts.push(part.clone());
            parts.push(part);
        }
        Ok(crate::traits::Scene {
            name: root.name.clone(),
            parts,
            root_nodes: vec![root],
        })
    }
}

impl PointCloudReader for ObjReader {
    fn read_points(&mut self) -> io::Result<Vec<[f32; 3]>> {
        self.read_positions()
    }
}

// ============================================================================
// Convenience Functions (for backward compatibility)
// ============================================================================

/// Parse vertex positions from an OBJ file.
/// Returns a vec of [x, y, z] positions.
pub fn read_obj_positions<P: AsRef<Path>>(path: P) -> io::Result<Vec<[f32; 3]>> {
    let file = fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut positions = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();

        if trimmed.starts_with("vn ") || trimmed.starts_with("vt ") {
            continue;
        }

        if !trimmed.starts_with('v') {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        if parts.next() != Some("v") {
            continue;
        }

        let x = parts.next().and_then(|s| s.parse().ok());
        let y = parts.next().and_then(|s| s.parse().ok());
        let z = parts.next().and_then(|s| s.parse().ok());

        if let (Some(x), Some(y), Some(z)) = (x, y, z) {
            positions.push([x, y, z]);
        }
    }

    Ok(positions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_obj_positions() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "# comment").unwrap();
        writeln!(file, "v 1.0 2.0 3.0").unwrap();
        writeln!(file, "v 4.5 5.5 6.5").unwrap();
        writeln!(file, "vn 0 1 0").unwrap();
        writeln!(file, "vt 0.5 0.5").unwrap();
        writeln!(file, "v -1.0 -2.0 -3.0").unwrap();
        file.flush().unwrap();

        let positions = read_obj_positions(file.path()).unwrap();
        assert_eq!(positions.len(), 3);
        assert_eq!(positions[0], [1.0, 2.0, 3.0]);
        assert_eq!(positions[1], [4.5, 5.5, 6.5]);
        assert_eq!(positions[2], [-1.0, -2.0, -3.0]);
    }
}
