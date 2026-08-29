//! STL format writer for triangle meshes.
//!
//! Writes both containers: binary, which is the default because it is a fifth
//! the size and exact, and ASCII for the cases that call for readable output.
//!
//! # Example
//!
//! ```no_run
//! use draco_io::{StlWriter, Writer};
//!
//! let mesh = draco_core::mesh::Mesh::new();
//! let mut writer = StlWriter::new();
//! writer.add_mesh(&mesh, Some("Model"))?;
//! writer.write("output.stl")?;
//! # Ok::<(), std::io::Error>(())
//! ```

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;

use crate::traits::{WriteToBytes, Writer};

/// Which STL container to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StlFormat {
    /// The 80-byte header and 50-byte triangle records.
    #[default]
    Binary,
    /// `solid`/`facet`/`vertex` text.
    Ascii,
}

/// STL format writer.
#[derive(Debug, Clone, Default)]
pub struct StlWriter {
    format: StlFormat,
    /// The name written into the ASCII `solid` line; binary has no field for it.
    name: String,
    triangles: Vec<[[f32; 3]; 3]>,
}

impl StlWriter {
    /// Choose the container to write. Binary is the default.
    pub fn with_format(mut self, format: StlFormat) -> Self {
        self.format = format;
        self
    }

    /// Number of triangles collected so far.
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }
}

impl Writer for StlWriter {
    fn new() -> Self {
        Self::default()
    }

    /// Add a mesh's triangles.
    ///
    /// STL holds geometry and nothing else — no vertex identity, no attributes,
    /// one facet normal per triangle — so only positions and the face list are
    /// read. Several meshes may be added; they become one solid, which is all
    /// the format can represent.
    fn add_mesh(&mut self, mesh: &Mesh, name: Option<&str>) -> io::Result<()> {
        crate::traits::ensure_attributes_cover_points(mesh, "STL")?;
        if self.name.is_empty() {
            if let Some(name) = name {
                self.name = name.to_string();
            }
        }
        let position_id = mesh.named_attribute_id(GeometryAttributeType::Position);
        if position_id < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Mesh has no position attribute",
            ));
        }
        // `read_position` reads a fixed twelve bytes per point, so anything but
        // Float32x3 either reads a neighbouring value as a float or runs off
        // the end of the buffer. A decoded `.drc` may legitimately declare its
        // positions as Uint8x3 or Int16x3 - the decoder takes both from the
        // bitstream - so this is reachable from any decode-then-write pipeline.
        // The OBJ writer already refuses the same mesh this way.
        let position = mesh.attribute(position_id);
        if position.data_type() != DataType::Float32 || position.num_components() != 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "STL writer requires position attributes to be Float32x3",
            ));
        }
        let point_count = mesh.num_points();
        for index in 0..mesh.num_faces() as u32 {
            let face = mesh.face(FaceIndex(index));
            if face.iter().any(|point| point.0 as usize >= point_count) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Mesh face references a point outside the attribute",
                ));
            }
            self.triangles.push([
                read_position(mesh, position_id, face[0].0 as usize),
                read_position(mesh, position_id, face[1].0 as usize),
                read_position(mesh, position_id, face[2].0 as usize),
            ]);
        }
        Ok(())
    }

    fn write<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let mut file = BufWriter::new(File::create(path)?);
        file.write_all(&self.write_to_vec()?)?;
        file.flush()
    }

    fn vertex_count(&self) -> usize {
        self.triangles.len() * 3
    }

    fn face_count(&self) -> usize {
        self.triangles.len()
    }
}

impl WriteToBytes for StlWriter {
    fn write_to_vec(&self) -> io::Result<Vec<u8>> {
        match self.format {
            StlFormat::Binary => Ok(self.write_binary()),
            StlFormat::Ascii => Ok(self.write_ascii().into_bytes()),
        }
    }
}

impl StlWriter {
    fn write_binary(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(84 + self.triangles.len() * 50);
        let mut header = [0u8; 80];
        // The header is free-form, and a reader deciding the container by its
        // first word would read this file as ASCII if it began with `solid`.
        let label = format!("Draco {}", self.name);
        let label = label.trim_end().as_bytes();
        let length = label.len().min(80);
        header[..length].copy_from_slice(&label[..length]);
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&(self.triangles.len() as u32).to_le_bytes());
        for triangle in &self.triangles {
            for component in facet_normal(triangle) {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
            for vertex in triangle {
                for component in vertex {
                    bytes.extend_from_slice(&component.to_le_bytes());
                }
            }
            bytes.extend_from_slice(&0u16.to_le_bytes());
        }
        bytes
    }

    fn write_ascii(&self) -> String {
        let name = if self.name.is_empty() {
            "mesh"
        } else {
            &self.name
        };
        // Spelled element by element into one String -- four `format!`s and
        // drops per facet was most of this writer's cost -- with zmij for the
        // floats: shortest round-trip like `Display`, into a stack buffer, no
        // `fmt` machinery per value. One thing to know: `Display` spells only
        // positional decimals, zmij switches to exponent notation for very
        // large and very small magnitudes, which every `strtod`-family STL
        // reader parses but the most naive ones may not.
        //
        // `format`, not `format_finite`: a vertex coordinate is whatever the
        // position attribute holds, and nothing upstream of here promises it
        // is finite. `format_finite` documents its answer for a non-finite
        // input as "correctly formatted but unspecified", and it is -- a NaN
        // comes out `2.696539702293474e+308`, a number a reader accepts
        // without complaint. `format` spells the three non-finite cases the
        // way `Display` did, so the file keeps saying `NaN` where the mesh
        // said `NaN`. The branch costs nothing measurable against the
        // formatting it guards. (`facet_normal` already answers `[0, 0, 0]`
        // for a degenerate triangle, so only the vertices need this.)
        let mut buffer = zmij::Buffer::new();
        let mut float = |text: &mut String, value: f32| {
            text.push_str(buffer.format(value));
        };

        let mut text = String::with_capacity(self.triangles.len() * 180 + name.len() * 2 + 32);
        text.push_str("solid ");
        text.push_str(name);
        text.push('\n');
        for triangle in &self.triangles {
            let [nx, ny, nz] = facet_normal(triangle);
            text.push_str("  facet normal ");
            float(&mut text, nx);
            text.push(' ');
            float(&mut text, ny);
            text.push(' ');
            float(&mut text, nz);
            text.push_str("\n    outer loop\n");
            for [x, y, z] in triangle {
                text.push_str("      vertex ");
                float(&mut text, *x);
                text.push(' ');
                float(&mut text, *y);
                text.push(' ');
                float(&mut text, *z);
                text.push('\n');
            }
            text.push_str("    endloop\n  endfacet\n");
        }
        text.push_str("endsolid ");
        text.push_str(name);
        text.push('\n');
        text
    }
}

/// The facet normal, from the winding rather than from any stored normal.
///
/// STL states one normal per facet and readers take it as the surface's own, so
/// a smooth-shading normal averaged from the corners would contradict the
/// geometry it is attached to. A degenerate triangle has no normal to state, and
/// zero is the format's way of saying the reader should derive one.
fn facet_normal(triangle: &[[f32; 3]; 3]) -> [f32; 3] {
    let [a, b, c] = triangle;
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let normal = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if length > 0.0 && length.is_finite() {
        [normal[0] / length, normal[1] / length, normal[2] / length]
    } else {
        [0.0, 0.0, 0.0]
    }
}

fn read_position(mesh: &Mesh, attribute_id: i32, point: usize) -> [f32; 3] {
    let attribute = mesh.attribute(attribute_id);
    let stride = attribute.byte_stride() as usize;
    let mut bytes = [0u8; 12];
    attribute.buffer().read(point * stride, &mut bytes);
    [
        f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::PointAttribute;
    use draco_core::geometry_indices::PointIndex;

    fn triangle_mesh() -> Mesh {
        mesh_from(&[[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]])
    }

    fn mesh_from(vertices: &[[f32; 3]; 3]) -> Mesh {
        let mut mesh = Mesh::new();
        mesh.set_num_points(3);
        mesh.set_num_faces(1);
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            3,
        );
        for (index, vertex) in vertices.iter().enumerate() {
            let bytes: Vec<u8> = vertex
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            attribute.buffer_mut().write(index * 12, &bytes);
        }
        mesh.add_attribute(attribute);
        mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
        mesh
    }

    #[test]
    fn test_write_binary_stl() {
        let mut writer = StlWriter::new();
        writer.add_mesh(&triangle_mesh(), Some("Tri")).unwrap();
        let bytes = writer.write_to_vec().unwrap();

        assert_eq!(bytes.len(), 84 + 50);
        assert_eq!(u32::from_le_bytes(bytes[80..84].try_into().unwrap()), 1);
        // Counter-clockwise in the XY plane, so the facet normal is +Z.
        let normal: Vec<f32> = (0..3)
            .map(|index| {
                let start = 84 + index * 4;
                f32::from_le_bytes(bytes[start..start + 4].try_into().unwrap())
            })
            .collect();
        assert_eq!(normal, vec![0.0, 0.0, 1.0]);
        // And the header must not start a binary file with the ASCII keyword.
        assert!(!bytes.starts_with(b"solid"));
    }

    #[test]
    fn test_write_ascii_stl() {
        let mut writer = StlWriter::new().with_format(StlFormat::Ascii);
        writer.add_mesh(&triangle_mesh(), Some("Tri")).unwrap();
        let text = String::from_utf8(writer.write_to_vec().unwrap()).unwrap();

        assert!(text.starts_with("solid Tri\n"));
        // zmij keeps a decimal point on whole values, so the +Z normal is
        // spelled `0.0 0.0 1.0` rather than `Display`'s bare `0 0 1`.
        assert!(text.contains("facet normal 0.0 0.0 1.0"));
        assert_eq!(text.matches("vertex ").count(), 3);
        assert!(text.trim_end().ends_with("endsolid Tri"));
    }

    /// A coordinate the mesh cannot express as a number must not be written as
    /// one.
    ///
    /// Nothing upstream promises a position attribute holds finite floats, and
    /// zmij's `format_finite` answers a non-finite input with an unspecified
    /// but well-formed number -- a NaN spells `2.696539702293474e+308`, which
    /// every reader takes for a real coordinate. The file has to keep saying
    /// what the mesh said.
    #[test]
    fn ascii_stl_spells_a_non_finite_coordinate_as_itself() {
        let mesh = mesh_from(&[
            [f32::NAN, 0.0, 0.0],
            [1.0, f32::INFINITY, 0.0],
            [0.0, 1.0, f32::NEG_INFINITY],
        ]);
        let mut writer = StlWriter::new().with_format(StlFormat::Ascii);
        writer.add_mesh(&mesh, Some("Tri")).unwrap();
        let text = String::from_utf8(writer.write_to_vec().unwrap()).unwrap();

        assert!(text.contains("vertex NaN 0.0 0.0"), "{text}");
        assert!(text.contains("vertex 1.0 inf 0.0"), "{text}");
        assert!(text.contains("vertex 0.0 1.0 -inf"), "{text}");
        assert!(!text.contains("e+308"), "{text}");
    }

    /// Both containers have to come back as the same geometry, and the binary
    /// one exactly: it stores the float bits the mesh held.
    #[cfg(feature = "stl-reader")]
    #[test]
    fn test_roundtrip_through_both_containers() {
        use crate::stl_reader::StlReader;

        for format in [StlFormat::Binary, StlFormat::Ascii] {
            let mut writer = StlWriter::new().with_format(format);
            writer.add_mesh(&triangle_mesh(), Some("Tri")).unwrap();
            let mesh = StlReader::read_from_bytes(&writer.write_to_vec().unwrap()).unwrap();

            assert_eq!(mesh.num_faces(), 1, "{format:?}");
            assert_eq!(mesh.num_points(), 3, "{format:?}");
            let position_id = mesh.named_attribute_id(GeometryAttributeType::Position);
            assert_eq!(
                read_position(&mesh, position_id, 1),
                [1.0, 0.0, 0.0],
                "{format:?}"
            );
            assert_eq!(
                read_position(&mesh, position_id, 2),
                [0.0, 1.0, 0.0],
                "{format:?}"
            );
        }
    }

    #[test]
    fn test_write_rejects_a_face_pointing_past_its_vertices() {
        let mut mesh = triangle_mesh();
        mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(9)]);
        let mut writer = StlWriter::new();
        let error = writer.add_mesh(&mesh, None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
