//! OBJ format reader for meshes and point clouds.
//!
//! Provides both a struct-based API (`ObjReader`) and convenience functions.

use std::fs;
use std::io::{self, BufRead, BufReader, Cursor};
use std::path::Path;

use crate::mesh_weld::CornerWeld;
use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::mesh::Mesh;

use crate::traits::{PointCloudReader, ReadFromBytes, Reader};

/// OBJ format reader.
///
/// Reads vertex positions, texture coordinates, normals, and faces from OBJ files.
#[derive(Debug)]
pub struct ObjReader {
    source: ObjReaderSource,
}

#[derive(Debug, Clone)]
enum ObjReaderSource {
    Path(std::path::PathBuf),
    Bytes(Vec<u8>),
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
        Ok(Self {
            source: ObjReaderSource::Path(path),
        })
    }

    /// Create an OBJ reader from in-memory bytes.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            source: ObjReaderSource::Bytes(bytes.into()),
        }
    }

    /// Read a mesh directly from in-memory bytes.
    pub fn read_from_bytes(bytes: &[u8]) -> io::Result<Mesh> {
        let mut reader = Self::from_bytes(bytes.to_vec());
        reader.read_mesh()
    }

    /// Read all positions from the OBJ file.
    pub fn read_positions(&mut self) -> io::Result<Vec<[f32; 3]>> {
        match &self.source {
            ObjReaderSource::Path(path) => read_obj_positions(path),
            ObjReaderSource::Bytes(bytes) => {
                read_obj_positions_from_reader(BufReader::new(Cursor::new(bytes.as_slice())))
            }
        }
    }

    /// Read mesh geometry from the OBJ file.
    fn read_mesh_data(&self) -> io::Result<ParsedObjMesh> {
        match &self.source {
            ObjReaderSource::Path(path) => {
                let file = fs::File::open(path)?;
                read_obj_mesh_from_reader(BufReader::new(file))
            }
            ObjReaderSource::Bytes(bytes) => {
                read_obj_mesh_from_reader(BufReader::new(Cursor::new(bytes.as_slice())))
            }
        }
    }

    /// Read a mesh with positions and faces (if present).
    ///
    /// The mesh is automatically processed to match C++ Draco OBJ loader behavior:
    /// point IDs are deduplicated in face-traversal order, which ensures binary
    /// compatibility when encoding with sequential encoding (speed 10).
    pub fn read_mesh(&mut self) -> io::Result<Mesh> {
        let parsed = self.read_mesh_data()?;
        let mut mesh = Mesh::new();

        if parsed.positions.is_empty() {
            return Ok(mesh);
        }

        mesh.set_num_points(parsed.positions.len());
        mesh.set_num_faces(parsed.faces.len());

        mesh.add_attribute(make_f32x3_attribute(
            GeometryAttributeType::Position,
            &parsed.positions,
        ));

        if let Some(normals) = parsed.normals.as_ref() {
            mesh.add_attribute(make_f32x3_attribute(GeometryAttributeType::Normal, normals));
        }

        if let Some(texcoords) = parsed.texcoords.as_ref() {
            mesh.add_attribute(make_f32x2_attribute(
                GeometryAttributeType::TexCoord,
                texcoords,
            ));
        }

        // Set faces
        use draco_core::geometry_indices::{FaceIndex, PointIndex};
        for (i, face) in parsed.faces.iter().enumerate() {
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

impl ReadFromBytes for ObjReader {
    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        Ok(Self::from_bytes(bytes.to_vec()))
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
    read_obj_positions_from_reader(reader)
}

fn read_obj_positions_from_reader<R: BufRead>(reader: R) -> io::Result<Vec<[f32; 3]>> {
    let mut positions = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = strip_obj_comment(&line);

        if trimmed.starts_with("vn ") || trimmed.starts_with("vt ") {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        if parts.next() != Some("v") {
            continue;
        }

        let x = parts.next().and_then(|s| s.parse().ok());
        let y = parts.next().and_then(|s| s.parse().ok());
        let z = parts.next().and_then(|s| s.parse().ok());

        // As above: a vertex line that does not parse is refused, because the
        // caller counts on the position order matching the file's numbering.
        match (x, y, z) {
            (Some(x), Some(y), Some(z)) => positions.push([x, y, z]),
            _ => return Err(invalid_obj("OBJ vertex line must have three numbers")),
        }
    }

    Ok(positions)
}

#[derive(Debug)]
struct ParsedObjMesh {
    positions: Vec<[f32; 3]>,
    texcoords: Option<Vec<[f32; 2]>>,
    normals: Option<Vec<[f32; 3]>>,
    faces: Vec<[u32; 3]>,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct ObjVertexRef {
    position: usize,
    texcoord: Option<usize>,
    normal: Option<usize>,
}

fn strip_obj_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or("").trim()
}

fn make_f32x3_attribute(
    attribute_type: GeometryAttributeType,
    values: &[[f32; 3]],
) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(attribute_type, 3, DataType::Float32, false, values.len());

    let buffer = attribute.buffer_mut();
    for (i, value) in values.iter().enumerate() {
        let bytes: Vec<u8> = value
            .iter()
            .flat_map(|component| component.to_le_bytes())
            .collect();
        buffer.write(i * 12, &bytes);
    }

    attribute
}

fn make_f32x2_attribute(
    attribute_type: GeometryAttributeType,
    values: &[[f32; 2]],
) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(attribute_type, 2, DataType::Float32, false, values.len());

    let buffer = attribute.buffer_mut();
    for (i, value) in values.iter().enumerate() {
        let bytes: Vec<u8> = value
            .iter()
            .flat_map(|component| component.to_le_bytes())
            .collect();
        buffer.write(i * 8, &bytes);
    }

    attribute
}

fn parse_obj_index(token: &str, len: usize, label: &str) -> io::Result<usize> {
    let raw = token
        .parse::<i32>()
        .map_err(|_| invalid_obj(format!("Bad {label} index: {token}")))?;
    if raw == 0 {
        return Err(invalid_obj(format!("OBJ {label} indices are 1-based")));
    }

    let index = if raw > 0 { raw - 1 } else { len as i32 + raw };

    if index < 0 || index as usize >= len {
        return Err(invalid_obj(format!(
            "OBJ {label} index {raw} is out of range for {len} values"
        )));
    }

    Ok(index as usize)
}

fn parse_face_vertex(
    token: &str,
    position_count: usize,
    texcoord_count: usize,
    normal_count: usize,
) -> io::Result<ObjVertexRef> {
    let parts: Vec<&str> = token.split('/').collect();
    if parts.is_empty() || parts.len() > 3 || parts[0].is_empty() {
        return Err(invalid_obj(format!("Bad OBJ face vertex: {token}")));
    }

    let position = parse_obj_index(parts[0], position_count, "position")?;
    let texcoord = if parts.get(1).is_some_and(|part| !part.is_empty()) {
        Some(parse_obj_index(parts[1], texcoord_count, "texcoord")?)
    } else {
        None
    };
    let normal = if parts.get(2).is_some_and(|part| !part.is_empty()) {
        Some(parse_obj_index(parts[2], normal_count, "normal")?)
    } else {
        None
    };

    Ok(ObjVertexRef {
        position,
        texcoord,
        normal,
    })
}

/// The point id for a face corner, creating one when the `v/vt/vn` triple is
/// new.
///
/// The triple is the key because the file supplies identity through it: two
/// corners naming the same indices are the same vertex, whatever the values
/// behind them turn out to be. `vertices` keeps them in the order the ids were
/// handed out, which is what the attributes are emitted from later.
fn push_obj_vertex(
    vertex_ref: ObjVertexRef,
    weld: &mut CornerWeld<ObjVertexRef>,
    vertices: &mut Vec<ObjVertexRef>,
) -> u32 {
    let (point_id, is_new) = weld.intern(vertex_ref);
    if is_new {
        vertices.push(vertex_ref);
    }
    point_id
}

fn invalid_obj(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn read_obj_mesh_from_reader<R: BufRead>(reader: R) -> io::Result<ParsedObjMesh> {
    let mut source_positions = Vec::new();
    let mut source_texcoords = Vec::new();
    let mut source_normals = Vec::new();
    let mut faces = Vec::new();
    let mut vertices = Vec::new();
    // Sized on nothing better than a guess: the corner count is not known
    // until the faces are parsed, which is the same pass that interns them.
    let mut weld: CornerWeld<ObjVertexRef> = CornerWeld::with_capacity(0);

    for line in reader.lines() {
        let line = line?;
        let trimmed = strip_obj_comment(&line);

        // The keyword is split off rather than matched with a trailing space:
        // OBJ separates fields with any whitespace, and requiring `"v "` made a
        // tab-delimited file - which is valid - decode to an empty mesh with
        // `Ok`.
        let mut parts = trimmed.split_whitespace();
        let keyword = parts.next();

        if keyword == Some("v") {
            // A vertex line that does not parse is refused rather than skipped.
            // Dropping it shifts every later 1-based index by one, so the faces
            // silently name different vertices and the reader returns `Ok` with
            // the wrong geometry; upstream `ObjDecoder::ParseVertexPosition`
            // fails the file on the same input.
            let x = parts.next().and_then(|s| s.parse().ok());
            let y = parts.next().and_then(|s| s.parse().ok());
            let z = parts.next().and_then(|s| s.parse().ok());

            match (x, y, z) {
                (Some(x), Some(y), Some(z)) => source_positions.push([x, y, z]),
                _ => return Err(invalid_obj("OBJ vertex line must have three numbers")),
            }
        } else if keyword == Some("vt") {
            let u = parts.next().and_then(|s| s.parse().ok());
            let v = parts.next().and_then(|s| s.parse().ok());

            match (u, v) {
                (Some(u), Some(v)) => source_texcoords.push([u, v]),
                _ => {
                    return Err(invalid_obj(
                        "OBJ texture coordinate line must have two numbers",
                    ))
                }
            }
        } else if keyword == Some("vn") {
            let x = parts.next().and_then(|s| s.parse().ok());
            let y = parts.next().and_then(|s| s.parse().ok());
            let z = parts.next().and_then(|s| s.parse().ok());

            match (x, y, z) {
                (Some(x), Some(y), Some(z)) => source_normals.push([x, y, z]),
                _ => return Err(invalid_obj("OBJ normal line must have three numbers")),
            }
        } else if keyword == Some("f") {
            let face_vertices = parts
                .map(|part| {
                    parse_face_vertex(
                        part,
                        source_positions.len(),
                        source_texcoords.len(),
                        source_normals.len(),
                    )
                })
                .collect::<io::Result<Vec<_>>>()?;
            if face_vertices.len() < 3 {
                continue;
            }

            for i in 1..face_vertices.len() - 1 {
                let triangle = [face_vertices[0], face_vertices[i], face_vertices[i + 1]];
                faces.push([
                    push_obj_vertex(triangle[0], &mut weld, &mut vertices),
                    push_obj_vertex(triangle[1], &mut weld, &mut vertices),
                    push_obj_vertex(triangle[2], &mut weld, &mut vertices),
                ]);
            }
        }
    }

    if faces.is_empty() {
        return Ok(ParsedObjMesh {
            positions: source_positions,
            texcoords: None,
            normals: None,
            faces,
        });
    }

    let uses_texcoords = vertices.iter().any(|vertex| vertex.texcoord.is_some());
    let uses_normals = vertices.iter().any(|vertex| vertex.normal.is_some());
    if uses_texcoords && vertices.iter().any(|vertex| vertex.texcoord.is_none()) {
        return Err(invalid_obj(
            "OBJ texture coordinate indices must be present on every face vertex",
        ));
    }
    if uses_normals && vertices.iter().any(|vertex| vertex.normal.is_none()) {
        return Err(invalid_obj(
            "OBJ normal indices must be present on every face vertex",
        ));
    }

    let positions = vertices
        .iter()
        .map(|vertex| source_positions[vertex.position])
        .collect();
    let texcoords = uses_texcoords.then(|| {
        vertices
            .iter()
            .map(|vertex| source_texcoords[vertex.texcoord.unwrap()])
            .collect()
    });
    let normals = uses_normals.then(|| {
        vertices
            .iter()
            .map(|vertex| source_normals[vertex.normal.unwrap()])
            .collect()
    });

    Ok(ParsedObjMesh {
        positions,
        texcoords,
        normals,
        faces,
    })
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

    #[test]
    fn test_read_obj_mesh_preserves_normals_and_texcoords() {
        let obj = br#"
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 1.0 1.0 0.0
v 0.0 1.0 0.0
vt 0.0 0.0
vt 1.0 0.0
vt 1.0 1.0
vt 0.0 1.0
vn 0.0 0.0 1.0
f 1/1/1 2/2/1 3/3/1 4/4/1
"#;

        let mut reader = ObjReader::from_bytes(obj.as_slice());
        let mesh = reader.read_mesh().unwrap();

        assert_eq!(mesh.num_points(), 4);
        assert_eq!(mesh.num_faces(), 2);

        let normal_att = mesh.named_attribute(GeometryAttributeType::Normal).unwrap();
        assert_eq!(normal_att.num_components(), 3);
        assert_eq!(normal_att.data_type(), DataType::Float32);

        let texcoord_att = mesh
            .named_attribute(GeometryAttributeType::TexCoord)
            .unwrap();
        assert_eq!(texcoord_att.num_components(), 2);
        assert_eq!(texcoord_att.data_type(), DataType::Float32);
        assert_eq!(texcoord_att.buffer().data().len(), 4 * 2 * 4);
    }
}
