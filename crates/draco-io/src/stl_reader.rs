//! STL format reader for triangle meshes.
//!
//! Reads both containers the format is written in — binary and ASCII — and
//! produces a mesh in the shape STL states it: three vertices per triangle,
//! shared by nothing, with the facet normal replicated onto each of them. STL
//! carries no vertex identity, so welding would be this reader inventing one.

use std::fs;
use std::io::{self, Cursor, Read};
use std::path::Path;

use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;

use crate::raw_attribute::make_f32x3_attribute;
use crate::traits::{ReadFromBytes, Reader};

/// The fixed part of a binary STL: an 80-byte header and the triangle count.
const BINARY_HEADER_LENGTH: usize = 84;
/// Three float triples, a normal, and the two-byte attribute count.
const BINARY_TRIANGLE_LENGTH: usize = 50;

/// STL format reader.
#[derive(Debug)]
pub struct StlReader {
    source: StlReaderSource,
}

#[derive(Debug, Clone)]
enum StlReaderSource {
    Path(std::path::PathBuf),
    Bytes(Vec<u8>),
}

/// One triangle as the file states it: a facet normal and three corners.
#[derive(Debug, Clone, Copy, PartialEq)]
struct StlTriangle {
    normal: [f32; 3],
    vertices: [[f32; 3]; 3],
}

impl StlReader {
    /// Open an STL file for reading.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found: {}", path.display()),
            ));
        }
        Ok(Self {
            source: StlReaderSource::Path(path),
        })
    }

    /// Create an STL reader from in-memory bytes.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            source: StlReaderSource::Bytes(bytes.into()),
        }
    }

    /// Read a mesh directly from in-memory bytes.
    pub fn read_from_bytes(bytes: &[u8]) -> io::Result<Mesh> {
        let mut reader = Self::from_bytes(bytes.to_vec());
        reader.read_mesh()
    }

    /// Read a mesh from the STL file.
    pub fn read_mesh(&mut self) -> io::Result<Mesh> {
        let bytes = match &self.source {
            StlReaderSource::Path(path) => fs::read(path)?,
            StlReaderSource::Bytes(bytes) => bytes.clone(),
        };
        Ok(triangles_to_mesh(&read_stl_bytes(&bytes)?))
    }
}

impl Reader for StlReader {
    fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        StlReader::open(path)
    }

    fn read_meshes(&mut self) -> io::Result<Vec<Mesh>> {
        Ok(vec![self.read_mesh()?])
    }
}

impl ReadFromBytes for StlReader {
    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        Ok(Self::from_bytes(bytes.to_vec()))
    }
}

fn invalid_stl(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// Whether these bytes are a binary STL rather than an ASCII one.
///
/// The leading keyword decides nothing: exporters exist that write `solid` into
/// the 80-byte header of a binary file, so a reader trusting it reads a binary
/// mesh as text and finds no facets at all. The length does decide — a binary
/// file is exactly 84 bytes plus 50 per triangle, and that is checked first.
fn is_binary_stl(bytes: &[u8]) -> bool {
    if bytes.len() < BINARY_HEADER_LENGTH {
        return false;
    }
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    if let Some(expected) = count
        .checked_mul(BINARY_TRIANGLE_LENGTH)
        .and_then(|body| body.checked_add(BINARY_HEADER_LENGTH))
    {
        if bytes.len() == expected {
            return true;
        }
    }
    // Length disagreed, so the file is truncated, padded, or text. Only then is
    // the keyword worth asking about, and only to choose which parser reports it.
    !bytes.starts_with(b"solid")
}

fn read_stl_bytes(bytes: &[u8]) -> io::Result<Vec<StlTriangle>> {
    if is_binary_stl(bytes) {
        read_binary_stl(bytes)
    } else {
        read_ascii_stl(
            std::str::from_utf8(bytes)
                .map_err(|_| invalid_stl("STL is neither a valid binary file nor UTF-8 text"))?,
        )
    }
}

fn read_binary_stl(bytes: &[u8]) -> io::Result<Vec<StlTriangle>> {
    if bytes.len() < BINARY_HEADER_LENGTH {
        return Err(invalid_stl("Binary STL is shorter than its header"));
    }
    let declared = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    // The count is read from the file, so it is a claim rather than a fact: it
    // sizes nothing until the bytes to back it have been counted.
    let available = (bytes.len() - BINARY_HEADER_LENGTH) / BINARY_TRIANGLE_LENGTH;
    if declared > available {
        return Err(invalid_stl(
            "Binary STL declares more triangles than it contains",
        ));
    }

    let mut cursor = Cursor::new(&bytes[BINARY_HEADER_LENGTH..]);
    let mut triangles = Vec::with_capacity(declared);
    let mut record = [0u8; BINARY_TRIANGLE_LENGTH];
    for _ in 0..declared {
        cursor.read_exact(&mut record)?;
        let value =
            |index: usize| f32::from_le_bytes(record[index * 4..index * 4 + 4].try_into().unwrap());
        triangles.push(StlTriangle {
            normal: [value(0), value(1), value(2)],
            vertices: [
                [value(3), value(4), value(5)],
                [value(6), value(7), value(8)],
                [value(9), value(10), value(11)],
            ],
        });
    }
    Ok(triangles)
}

fn read_ascii_stl(text: &str) -> io::Result<Vec<StlTriangle>> {
    let mut triangles = Vec::new();
    let mut normal = [0.0f32; 3];
    let mut corners: Vec<[f32; 3]> = Vec::with_capacity(3);

    for line in text.lines() {
        let mut tokens = line.split_whitespace();
        let keyword = match tokens.next() {
            Some(keyword) => keyword,
            None => continue,
        };
        match keyword {
            // `facet normal nx ny nz`, and a facet whose normal is stated as
            // anything else is taken as unstated rather than refused.
            "facet" => {
                corners.clear();
                normal = read_ascii_triple(tokens.skip(1)).unwrap_or([0.0; 3]);
            }
            "vertex" => {
                let vertex = read_ascii_triple(tokens)
                    .ok_or_else(|| invalid_stl("ASCII STL vertex needs three coordinates"))?;
                corners.push(vertex);
            }
            "endfacet" => {
                // A polygon larger than a triangle is not STL, but a facet cut
                // short is a truncated file and is worth naming as one.
                if corners.len() < 3 {
                    return Err(invalid_stl("ASCII STL facet has fewer than three vertices"));
                }
                for corner in 1..corners.len() - 1 {
                    triangles.push(StlTriangle {
                        normal,
                        vertices: [corners[0], corners[corner], corners[corner + 1]],
                    });
                }
                corners.clear();
            }
            _ => {}
        }
    }

    Ok(triangles)
}

fn read_ascii_triple<'a>(tokens: impl Iterator<Item = &'a str>) -> Option<[f32; 3]> {
    let values: Vec<f32> = tokens
        .take(3)
        .filter_map(|token| token.parse().ok())
        .collect();
    match values.as_slice() {
        [x, y, z] => Some([*x, *y, *z]),
        _ => None,
    }
}

fn triangles_to_mesh(triangles: &[StlTriangle]) -> Mesh {
    let mut mesh = Mesh::new();
    if triangles.is_empty() {
        return mesh;
    }

    let point_count = triangles.len() * 3;
    mesh.set_num_points(point_count);
    mesh.set_num_faces(triangles.len());

    let mut positions = Vec::with_capacity(point_count);
    let mut normals = Vec::with_capacity(point_count);
    for triangle in triangles {
        for vertex in triangle.vertices {
            positions.push(vertex);
            // STL states one normal per facet; the mesh model has none but the
            // per-point kind, so it is written onto all three corners. Flat
            // shading is what the format means, and this is how it survives.
            normals.push(triangle.normal);
        }
    }

    mesh.add_attribute(make_f32x3_attribute(
        GeometryAttributeType::Position,
        &positions,
    ));
    if normals.iter().any(|normal| normal != &[0.0, 0.0, 0.0]) {
        mesh.add_attribute(make_f32x3_attribute(
            GeometryAttributeType::Normal,
            &normals,
        ));
    }

    for (index, _) in triangles.iter().enumerate() {
        let base = (index * 3) as u32;
        mesh.set_face(
            FaceIndex(index as u32),
            [PointIndex(base), PointIndex(base + 1), PointIndex(base + 2)],
        );
    }

    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_stl(header: &[u8], triangles: &[StlTriangle]) -> Vec<u8> {
        let mut bytes = vec![0u8; 80];
        bytes[..header.len().min(80)].copy_from_slice(&header[..header.len().min(80)]);
        bytes.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
        for triangle in triangles {
            for component in triangle.normal {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
            for vertex in triangle.vertices {
                for component in vertex {
                    bytes.extend_from_slice(&component.to_le_bytes());
                }
            }
            bytes.extend_from_slice(&0u16.to_le_bytes());
        }
        bytes
    }

    fn one_triangle() -> StlTriangle {
        StlTriangle {
            normal: [0.0, 0.0, 1.0],
            vertices: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        }
    }

    #[test]
    fn test_read_binary_stl() {
        let data = binary_stl(b"binary", &[one_triangle()]);
        let triangles = read_stl_bytes(&data).unwrap();
        assert_eq!(triangles, vec![one_triangle()]);

        let mesh = StlReader::read_from_bytes(&data).unwrap();
        assert_eq!(mesh.num_points(), 3);
        assert_eq!(mesh.num_faces(), 1);
        assert_eq!(
            mesh.face(FaceIndex(0)),
            [PointIndex(0), PointIndex(1), PointIndex(2)]
        );
        assert!(mesh.named_attribute_id(GeometryAttributeType::Normal) >= 0);
    }

    /// The header is 80 free bytes, and exporters have put `solid` in them. A
    /// reader that decides on the keyword reads such a file as text and finds
    /// nothing; the length is what actually separates the two containers.
    #[test]
    fn test_binary_stl_whose_header_says_solid() {
        let data = binary_stl(
            b"solid created by an exporter that means binary",
            &[one_triangle()],
        );
        assert!(is_binary_stl(&data));
        assert_eq!(read_stl_bytes(&data).unwrap(), vec![one_triangle()]);
    }

    #[test]
    fn test_read_ascii_stl() {
        let text = "solid demo\n\
             facet normal 0 0 1\n\
             outer loop\n\
             vertex 0 0 0\n\
             vertex 1 0 0\n\
             vertex 0 1 0\n\
             endloop\n\
             endfacet\n\
             endsolid demo\n";
        let triangles = read_stl_bytes(text.as_bytes()).unwrap();
        assert_eq!(triangles, vec![one_triangle()]);
    }

    #[test]
    fn test_ascii_stl_rejects_a_truncated_facet() {
        let text = "solid demo\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nendloop\nendfacet\n";
        let error = read_stl_bytes(text.as_bytes()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    /// A declared count larger than the file is the one field an attacker or a
    /// truncating transfer controls, and it used to size the allocation.
    #[test]
    fn test_binary_stl_rejects_an_overstated_count() {
        let mut data = binary_stl(b"binary", &[one_triangle()]);
        data[80..84].copy_from_slice(&1_000_000u32.to_le_bytes());
        let error = read_stl_bytes(&data).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_empty_stl_reads_as_an_empty_mesh() {
        let mesh = StlReader::read_from_bytes(&binary_stl(b"binary", &[])).unwrap();
        assert_eq!(mesh.num_points(), 0);
        assert_eq!(mesh.num_faces(), 0);
    }
}
