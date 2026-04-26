//! PLY format writer for meshes and point clouds.
//!
//! Supports writing:
//! - ASCII PLY format
//! - Binary little-endian PLY format
//! - Vertex positions
//! - Vertex normals (if present)
//! - Vertex colors (if present)
//! - Triangle faces (for meshes)
//!
//! # Example
//!
//! ```ignore
//! use draco_io::PlyWriter;
//! use draco_core::mesh::Mesh;
//!
//! let mesh: Mesh = /* ... */;
//! let mut writer = PlyWriter::new();
//! writer.add_mesh(&mesh);
//! writer.write("output.ply")?;
//!
//! // Or write point cloud
//! let mut writer = PlyWriter::new();
//! writer.add_points(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
//! writer.write("points.ply")?;
//! ```

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;

pub use crate::ply_format::PlyFormat;
use crate::traits::{PointCloudWriter, Writer};

/// PLY format writer.
///
/// This struct provides a builder-style API for writing PLY files.
/// Meshes or points are added, then written with `write()`.
///
/// # Example
///
/// ```ignore
/// use draco_io::PlyWriter;
///
/// let mut writer = PlyWriter::new();
/// writer.add_mesh(&mesh);
/// writer.write("cube.ply")?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct PlyWriter {
    /// Output format
    format: PlyFormat,
    /// Collected vertex positions
    positions: PlyPositionData,
    /// Collected vertex normals
    normals: Vec<[f32; 3]>,
    /// Collected vertex colors (RGBA 0-255)
    colors: Vec<[u8; 4]>,
    color_components: u8,
    /// Collected vertex texture coordinates
    texcoords: Vec<[f32; 2]>,
    /// Collected faces (0-based indices)
    faces: Vec<[u32; 3]>,
}

#[derive(Debug, Clone)]
enum PlyPositionData {
    Float32(Vec<[f32; 3]>),
    Float64(Vec<[f64; 3]>),
    Int32(Vec<[i32; 3]>),
    Uint32(Vec<[u32; 3]>),
}

impl Default for PlyPositionData {
    fn default() -> Self {
        PlyPositionData::Float32(Vec::new())
    }
}

impl PlyPositionData {
    fn len(&self) -> usize {
        match self {
            PlyPositionData::Float32(values) => values.len(),
            PlyPositionData::Float64(values) => values.len(),
            PlyPositionData::Int32(values) => values.len(),
            PlyPositionData::Uint32(values) => values.len(),
        }
    }

    fn data_type(&self) -> draco_core::draco_types::DataType {
        match self {
            PlyPositionData::Float32(_) => draco_core::draco_types::DataType::Float32,
            PlyPositionData::Float64(_) => draco_core::draco_types::DataType::Float64,
            PlyPositionData::Int32(_) => draco_core::draco_types::DataType::Int32,
            PlyPositionData::Uint32(_) => draco_core::draco_types::DataType::Uint32,
        }
    }

    fn type_name(&self) -> &'static str {
        match self.data_type() {
            draco_core::draco_types::DataType::Float64 => "double",
            draco_core::draco_types::DataType::Int32 => "int",
            draco_core::draco_types::DataType::Uint32 => "uint",
            _ => "float",
        }
    }

    fn push_f32_slice(&mut self, points: &[[f32; 3]]) {
        self.ensure_float32();
        if let PlyPositionData::Float32(values) = self {
            values.extend_from_slice(points);
        }
    }

    fn ensure_float32(&mut self) {
        if matches!(self, PlyPositionData::Float32(_)) {
            return;
        }
        let converted = self.iter_as_f32().collect();
        *self = PlyPositionData::Float32(converted);
    }

    fn iter_as_f32(&self) -> Box<dyn Iterator<Item = [f32; 3]> + '_> {
        match self {
            PlyPositionData::Float32(values) => Box::new(values.iter().copied()),
            PlyPositionData::Float64(values) => Box::new(
                values
                    .iter()
                    .map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]),
            ),
            PlyPositionData::Int32(values) => Box::new(
                values
                    .iter()
                    .map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]),
            ),
            PlyPositionData::Uint32(values) => Box::new(
                values
                    .iter()
                    .map(|v| [v[0] as f32, v[1] as f32, v[2] as f32]),
            ),
        }
    }
}

impl PlyWriter {
    /// Create a new PLY writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure the writer to emit binary little-endian PLY.
    pub fn with_binary_little_endian(mut self) -> Self {
        self.format = PlyFormat::BinaryLittleEndian;
        self
    }

    /// Configure the PLY storage format.
    pub fn with_format(mut self, format: PlyFormat) -> Self {
        self.format = format;
        self
    }

    /// Set the PLY storage format.
    pub fn set_format(&mut self, format: PlyFormat) -> &mut Self {
        self.format = format;
        self
    }

    /// Get the configured PLY storage format.
    pub fn format(&self) -> PlyFormat {
        self.format
    }

    /// Enable or disable binary little-endian output.
    pub fn set_binary_little_endian(&mut self, enabled: bool) -> &mut Self {
        self.format = if enabled {
            PlyFormat::BinaryLittleEndian
        } else {
            PlyFormat::Ascii
        };
        self
    }

    /// Returns true when the writer is configured for binary little-endian output.
    pub fn is_binary_little_endian(&self) -> bool {
        self.format == PlyFormat::BinaryLittleEndian
    }

    /// Add raw point positions (for point cloud output).
    pub fn add_points(&mut self, points: &[[f32; 3]]) {
        self.positions.push_f32_slice(points);
    }

    /// Add a single point.
    pub fn add_point(&mut self, point: [f32; 3]) {
        self.add_points(&[point]);
    }

    /// Add points with colors.
    pub fn add_points_with_colors(&mut self, points: &[[f32; 3]], colors: &[[u8; 4]]) {
        // Pad colors if needed
        while self.colors.len() < self.positions.len() {
            self.colors.push([255, 255, 255, 255]);
        }
        self.positions.push_f32_slice(points);
        self.color_components = self.color_components.max(4);
        self.colors.extend_from_slice(colors);
    }

    /// Get the number of vertices added.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Get the number of faces added.
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Check if the writer has normals.
    pub fn has_normals(&self) -> bool {
        !self.normals.is_empty()
    }

    /// Check if the writer has colors.
    pub fn has_colors(&self) -> bool {
        !self.colors.is_empty()
    }

    /// Write the PLY file to the given path.
    pub fn write<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        self.write_to(&mut writer)
    }

    /// Write the PLY data into a byte vector.
    pub fn write_to_vec(&self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        self.write_to(&mut out)?;
        Ok(out)
    }

    /// Write the PLY data to a writer.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let has_normals = self.normals.len() == self.positions.len();
        let has_colors = self.colors.len() == self.positions.len() && self.color_components > 0;

        let has_texcoords = self.texcoords.len() == self.positions.len() && !self.faces.is_empty();
        self.write_header(writer, has_normals, has_colors, has_texcoords)?;

        match self.format {
            PlyFormat::Ascii => {
                self.write_ascii_body(writer, has_normals, has_colors, has_texcoords)
            }
            PlyFormat::BinaryLittleEndian => {
                self.write_binary_body(writer, has_normals, has_colors, has_texcoords, false)
            }
            PlyFormat::BinaryBigEndian => {
                self.write_binary_body(writer, has_normals, has_colors, has_texcoords, true)
            }
        }
    }

    fn write_header<W: Write>(
        &self,
        writer: &mut W,
        has_normals: bool,
        has_colors: bool,
        has_texcoords: bool,
    ) -> io::Result<()> {
        writeln!(writer, "ply")?;
        match self.format {
            PlyFormat::Ascii => writeln!(writer, "format ascii 1.0")?,
            PlyFormat::BinaryLittleEndian => writeln!(writer, "format binary_little_endian 1.0")?,
            PlyFormat::BinaryBigEndian => writeln!(writer, "format binary_big_endian 1.0")?,
        }
        writeln!(writer, "comment Generated by draco-io")?;
        writeln!(writer, "element vertex {}", self.positions.len())?;
        writeln!(writer, "property {} x", self.positions.type_name())?;
        writeln!(writer, "property {} y", self.positions.type_name())?;
        writeln!(writer, "property {} z", self.positions.type_name())?;

        if has_normals {
            writeln!(writer, "property float nx")?;
            writeln!(writer, "property float ny")?;
            writeln!(writer, "property float nz")?;
        }

        if has_colors {
            writeln!(writer, "property uchar red")?;
            writeln!(writer, "property uchar green")?;
            writeln!(writer, "property uchar blue")?;
            if self.color_components > 3 {
                writeln!(writer, "property uchar alpha")?;
            }
        }

        if !self.faces.is_empty() {
            writeln!(writer, "element face {}", self.faces.len())?;
            writeln!(writer, "property list uchar int vertex_indices")?;
            if has_texcoords {
                writeln!(writer, "property list uchar float texcoord")?;
            }
        }

        writeln!(writer, "end_header")?;
        Ok(())
    }

    fn write_ascii_body<W: Write>(
        &self,
        writer: &mut W,
        has_normals: bool,
        has_colors: bool,
        has_texcoords: bool,
    ) -> io::Result<()> {
        for i in 0..self.positions.len() {
            match &self.positions {
                PlyPositionData::Float32(values) => {
                    let [x, y, z] = values[i];
                    write!(writer, "{:.6} {:.6} {:.6}", x, y, z)?;
                }
                PlyPositionData::Float64(values) => {
                    let [x, y, z] = values[i];
                    write!(writer, "{:.6} {:.6} {:.6}", x, y, z)?;
                }
                PlyPositionData::Int32(values) => {
                    let [x, y, z] = values[i];
                    write!(writer, "{} {} {}", x, y, z)?;
                }
                PlyPositionData::Uint32(values) => {
                    let [x, y, z] = values[i];
                    write!(writer, "{} {} {}", x, y, z)?;
                }
            }

            if has_normals {
                let [nx, ny, nz] = self.normals[i];
                write!(writer, " {:.6} {:.6} {:.6}", nx, ny, nz)?;
            }

            if has_colors {
                let [r, g, b, a] = self.colors[i];
                write!(writer, " {} {} {}", r, g, b)?;
                if self.color_components > 3 {
                    write!(writer, " {}", a)?;
                }
            }

            writeln!(writer)?;
        }

        // Write faces
        for face in &self.faces {
            write!(writer, "3 {} {} {}", face[0], face[1], face[2])?;
            if has_texcoords {
                write!(writer, " 6")?;
                for index in face {
                    let [u, v] = self.texcoords[*index as usize];
                    write!(writer, " {:.6} {:.6}", u, v)?;
                }
            }
            writeln!(writer)?;
        }

        Ok(())
    }

    fn write_binary_body<W: Write>(
        &self,
        writer: &mut W,
        has_normals: bool,
        has_colors: bool,
        has_texcoords: bool,
        big_endian: bool,
    ) -> io::Result<()> {
        for i in 0..self.positions.len() {
            match &self.positions {
                PlyPositionData::Float32(values) => {
                    for component in values[i] {
                        writer.write_all(&if big_endian {
                            component.to_be_bytes()
                        } else {
                            component.to_le_bytes()
                        })?;
                    }
                }
                PlyPositionData::Float64(values) => {
                    for component in values[i] {
                        writer.write_all(&if big_endian {
                            component.to_be_bytes()
                        } else {
                            component.to_le_bytes()
                        })?;
                    }
                }
                PlyPositionData::Int32(values) => {
                    for component in values[i] {
                        writer.write_all(&if big_endian {
                            component.to_be_bytes()
                        } else {
                            component.to_le_bytes()
                        })?;
                    }
                }
                PlyPositionData::Uint32(values) => {
                    for component in values[i] {
                        writer.write_all(&if big_endian {
                            component.to_be_bytes()
                        } else {
                            component.to_le_bytes()
                        })?;
                    }
                }
            }

            if has_normals {
                let [nx, ny, nz] = self.normals[i];
                writer.write_all(&if big_endian {
                    nx.to_be_bytes()
                } else {
                    nx.to_le_bytes()
                })?;
                writer.write_all(&if big_endian {
                    ny.to_be_bytes()
                } else {
                    ny.to_le_bytes()
                })?;
                writer.write_all(&if big_endian {
                    nz.to_be_bytes()
                } else {
                    nz.to_le_bytes()
                })?;
            }

            if has_colors {
                writer.write_all(&self.colors[i][..self.color_components as usize])?;
            }
        }

        for face in &self.faces {
            writer.write_all(&[3u8])?;
            for index in face {
                let index = i32::try_from(*index).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "PLY binary writer only supports face indices up to i32::MAX",
                    )
                })?;
                writer.write_all(&if big_endian {
                    index.to_be_bytes()
                } else {
                    index.to_le_bytes()
                })?;
            }
            if has_texcoords {
                writer.write_all(&[6u8])?;
                for index in face {
                    let [u, v] = self.texcoords[*index as usize];
                    writer.write_all(&if big_endian {
                        u.to_be_bytes()
                    } else {
                        u.to_le_bytes()
                    })?;
                    writer.write_all(&if big_endian {
                        v.to_be_bytes()
                    } else {
                        v.to_le_bytes()
                    })?;
                }
            }
        }

        Ok(())
    }
}

/// Read a float3 from an attribute at a given point index.
fn read_float3(mesh: &Mesh, att_id: i32, point_idx: usize) -> [f32; 3] {
    let att = mesh.attribute(att_id);
    let byte_stride = att.byte_stride() as usize;
    let buffer = att.buffer();
    let mut bytes = [0u8; 12];
    buffer.read(point_idx * byte_stride, &mut bytes);
    [
        f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
    ]
}

/// Read a color from an attribute at a given point index.
fn read_color(mesh: &Mesh, att_id: i32, point_idx: usize) -> [u8; 4] {
    let att = mesh.attribute(att_id);
    let byte_stride = att.byte_stride() as usize;
    let buffer = att.buffer();

    // Colors can be stored in different formats
    let num_components = att.num_components() as usize;
    let component_size = byte_stride / num_components;

    if component_size == 1 {
        // u8 colors
        let mut bytes = [255u8; 4];
        let read_len = num_components.min(4);
        buffer.read(point_idx * byte_stride, &mut bytes[..read_len]);
        bytes
    } else if component_size == 4 {
        // f32 colors (0.0-1.0) - convert to u8
        let mut float_bytes = [0u8; 16];
        let read_len = (num_components * 4).min(16);
        buffer.read(point_idx * byte_stride, &mut float_bytes[..read_len]);

        let mut result = [255u8; 4];
        for i in 0..num_components.min(4) {
            let f = f32::from_le_bytes([
                float_bytes[i * 4],
                float_bytes[i * 4 + 1],
                float_bytes[i * 4 + 2],
                float_bytes[i * 4 + 3],
            ]);
            result[i] = (f.clamp(0.0, 1.0) * 255.0) as u8;
        }
        result
    } else {
        [255, 255, 255, 255] // Default white
    }
}

// ============================================================================
// Trait Implementations
// ============================================================================

impl Writer for PlyWriter {
    fn new() -> Self {
        Self::default()
    }

    fn add_mesh(&mut self, mesh: &Mesh, _name: Option<&str>) -> io::Result<()> {
        // PLY format doesn't support mesh names
        let vertex_offset = self.positions.len() as u32;

        // Extract positions
        let pos_att_id = mesh.named_attribute_id(GeometryAttributeType::Position);
        if pos_att_id >= 0 {
            let att = mesh.attribute(pos_att_id);
            append_positions_from_attribute(&mut self.positions, att, mesh.num_points());
        }

        // Extract normals if present
        let normal_att_id = mesh.named_attribute_id(GeometryAttributeType::Normal);
        if normal_att_id >= 0 {
            // Pad normals if we've added vertices without normals before
            while self.normals.len() < self.positions.len() - mesh.num_points() {
                self.normals.push([0.0, 0.0, 0.0]);
            }
            for i in 0..mesh.num_points() {
                self.normals.push(read_float3(mesh, normal_att_id, i));
            }
        }

        // Extract colors if present
        let color_att_id = mesh.named_attribute_id(GeometryAttributeType::Color);
        if color_att_id >= 0 {
            let color_att = mesh.attribute(color_att_id);
            let components = color_att.num_components().clamp(1, 4);
            self.color_components = self.color_components.max(components);
            // Pad colors if we've added vertices without colors before
            while self.colors.len() < self.positions.len() - mesh.num_points() {
                self.colors.push([255, 255, 255, 255]);
            }
            for i in 0..mesh.num_points() {
                self.colors.push(read_color(mesh, color_att_id, i));
            }
        }

        let texcoord_att_id = mesh.named_attribute_id(GeometryAttributeType::TexCoord);
        if texcoord_att_id >= 0 {
            let texcoord_att = mesh.attribute(texcoord_att_id);
            if texcoord_att.num_components() == 2 && texcoord_att.data_type() == DataType::Float32 {
                while self.texcoords.len() < self.positions.len() - mesh.num_points() {
                    self.texcoords.push([0.0, 0.0]);
                }
                for i in 0..mesh.num_points() {
                    self.texcoords.push(read_float2(mesh, texcoord_att_id, i));
                }
            }
        }

        // Extract faces (0-based indices with offset)
        for i in 0..mesh.num_faces() as u32 {
            let face = mesh.face(FaceIndex(i));
            self.faces.push([
                face[0].0 + vertex_offset,
                face[1].0 + vertex_offset,
                face[2].0 + vertex_offset,
            ]);
        }
        Ok(())
    }

    fn write<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        self.write(path)
    }

    fn vertex_count(&self) -> usize {
        self.vertex_count()
    }

    fn face_count(&self) -> usize {
        self.face_count()
    }
}

impl PointCloudWriter for PlyWriter {
    fn add_points(&mut self, points: &[[f32; 3]]) {
        self.add_points(points);
    }

    fn add_point(&mut self, point: [f32; 3]) {
        self.add_point(point);
    }
}

// ============================================================================
// Convenience Functions (for backward compatibility)
// ============================================================================

/// Write a mesh to a PLY file.
///
/// This is a convenience function. For more control, use `PlyWriter` directly.
pub fn write_ply_mesh<P: AsRef<Path>>(path: P, mesh: &Mesh) -> io::Result<()> {
    let mut writer = PlyWriter::new();
    Writer::add_mesh(&mut writer, mesh, None)?;
    writer.write(path)
}

/// Write point positions to a PLY file (point cloud, no faces).
///
/// This is a convenience function. For more control, use `PlyWriter` directly.
pub fn write_ply_positions<P: AsRef<Path>>(path: P, points: &[[f32; 3]]) -> io::Result<()> {
    let mut writer = PlyWriter::new();
    writer.add_points(points);
    writer.write(path)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "decoder")]
    use crate::ply_reader::PlyReader;
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::PointAttribute;
    use draco_core::geometry_indices::PointIndex;
    use std::fs;
    use tempfile::NamedTempFile;

    fn create_triangle_mesh() -> Mesh {
        let mut mesh = Mesh::new();
        let mut pos_att = PointAttribute::new();

        pos_att.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            3,
        );
        let buffer = pos_att.buffer_mut();
        let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        for (i, pos) in positions.iter().enumerate() {
            let bytes: Vec<u8> = pos.iter().flat_map(|v| v.to_le_bytes()).collect();
            buffer.write(i * 12, &bytes);
        }
        mesh.add_attribute(pos_att);

        mesh.set_num_faces(1);
        mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);

        mesh
    }

    #[test]
    fn test_ply_writer_new() {
        let writer = PlyWriter::new();
        assert_eq!(writer.vertex_count(), 0);
        assert_eq!(writer.face_count(), 0);
        assert!(!writer.has_normals());
        assert!(!writer.has_colors());
        assert!(!writer.is_binary_little_endian());
    }

    #[test]
    fn test_ply_writer_add_mesh() {
        let mesh = create_triangle_mesh();
        let mut writer = PlyWriter::new();
        Writer::add_mesh(&mut writer, &mesh, None).unwrap();
        assert_eq!(writer.vertex_count(), 3);
        assert_eq!(writer.face_count(), 1);
    }

    #[test]
    fn test_ply_writer_add_points() {
        let mut writer = PlyWriter::new();
        writer.add_points(&[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
        assert_eq!(writer.vertex_count(), 2);
        assert_eq!(writer.face_count(), 0);
    }

    #[test]
    fn test_ply_writer_add_points_with_colors() {
        let mut writer = PlyWriter::new();
        writer.add_points_with_colors(
            &[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
            &[[255, 0, 0, 255], [0, 255, 0, 255]],
        );
        assert_eq!(writer.vertex_count(), 2);
        assert!(writer.has_colors());
    }

    #[test]
    fn test_write_ply_positions() {
        let points = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

        let file = NamedTempFile::new().unwrap();
        write_ply_positions(file.path(), &points).unwrap();

        let content = fs::read_to_string(file.path()).unwrap();
        assert!(content.contains("ply"));
        assert!(content.contains("format ascii 1.0"));
        assert!(content.contains("element vertex 3"));
        assert!(content.contains("property float x"));
        assert!(content.contains("end_header"));
        assert!(content.contains("0.000000 0.000000 0.000000"));
        assert!(content.contains("1.000000 0.000000 0.000000"));
    }

    #[test]
    fn test_write_ply_mesh() {
        let mesh = create_triangle_mesh();
        let file = NamedTempFile::new().unwrap();
        write_ply_mesh(file.path(), &mesh).unwrap();

        let content = fs::read_to_string(file.path()).unwrap();
        assert!(content.contains("ply"));
        assert!(content.contains("element vertex 3"));
        assert!(content.contains("element face 1"));
        assert!(content.contains("property list uchar int vertex_indices"));
        assert!(content.contains("3 0 1 2")); // face with 0-based indices
    }

    #[test]
    fn test_multiple_meshes() {
        let mesh1 = create_triangle_mesh();
        let mesh2 = create_triangle_mesh();

        let mut writer = PlyWriter::new();
        Writer::add_mesh(&mut writer, &mesh1, None).unwrap();
        Writer::add_mesh(&mut writer, &mesh2, None).unwrap();

        assert_eq!(writer.vertex_count(), 6);
        assert_eq!(writer.face_count(), 2);

        let file = NamedTempFile::new().unwrap();
        writer.write(file.path()).unwrap();

        let content = fs::read_to_string(file.path()).unwrap();
        assert!(content.contains("element vertex 6"));
        assert!(content.contains("element face 2"));
        // Second mesh should have offset indices
        assert!(content.contains("3 3 4 5"));
    }

    #[test]
    fn test_ply_with_colors() {
        let mut writer = PlyWriter::new();
        writer.add_points_with_colors(
            &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            &[[255, 0, 0, 255], [0, 255, 0, 255]],
        );

        let file = NamedTempFile::new().unwrap();
        writer.write(file.path()).unwrap();

        let content = fs::read_to_string(file.path()).unwrap();
        assert!(content.contains("property uchar red"));
        assert!(content.contains("property uchar green"));
        assert!(content.contains("property uchar blue"));
        assert!(content.contains("property uchar alpha"));
        assert!(content.contains("255 0 0 255"));
        assert!(content.contains("0 255 0 255"));
    }

    #[test]
    fn test_ply_writer_can_switch_to_binary_little_endian() {
        let writer = PlyWriter::new().with_binary_little_endian();
        assert!(writer.is_binary_little_endian());

        let mut writer = PlyWriter::new();
        writer.set_binary_little_endian(true);
        assert!(writer.is_binary_little_endian());
        writer.set_binary_little_endian(false);
        assert!(!writer.is_binary_little_endian());
    }

    #[cfg(feature = "decoder")]
    #[test]
    fn test_write_binary_little_endian_positions_roundtrip() {
        let points = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

        let file = NamedTempFile::new().unwrap();
        let mut writer = PlyWriter::new().with_binary_little_endian();
        writer.add_points(&points);
        writer.write(file.path()).unwrap();

        let content = fs::read(file.path()).unwrap();
        let header_end = content
            .windows(b"end_header\n".len())
            .position(|window| window == b"end_header\n")
            .map(|idx| idx + b"end_header\n".len())
            .unwrap();
        let header = std::str::from_utf8(&content[..header_end]).unwrap();
        assert!(header.contains("format binary_little_endian 1.0"));

        let mut reader = PlyReader::open(file.path()).unwrap();
        let positions = reader.read_positions().unwrap();
        assert_eq!(positions, points);
    }

    #[cfg(feature = "decoder")]
    #[test]
    fn test_write_binary_little_endian_mesh_roundtrip() {
        let mesh = create_triangle_mesh();
        let file = NamedTempFile::new().unwrap();

        let mut writer = PlyWriter::new().with_binary_little_endian();
        Writer::add_mesh(&mut writer, &mesh, None).unwrap();
        writer.write(file.path()).unwrap();

        let bytes = fs::read(file.path()).unwrap();
        let header_end = bytes
            .windows(b"end_header\n".len())
            .position(|window| window == b"end_header\n")
            .map(|idx| idx + b"end_header\n".len())
            .unwrap();
        let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
        assert!(header.contains("format binary_little_endian 1.0"));
        assert!(header.contains("element vertex 3"));
        assert!(header.contains("element face 1"));

        let mut reader = PlyReader::open(file.path()).unwrap();
        let mesh = reader.read_mesh().unwrap();
        assert_eq!(mesh.num_points(), 3);
        assert_eq!(mesh.num_faces(), 1);
        assert_eq!(
            mesh.face(FaceIndex(0)),
            [PointIndex(0), PointIndex(1), PointIndex(2)]
        );
    }

    #[cfg(feature = "decoder")]
    #[test]
    fn test_write_binary_big_endian_mesh_roundtrip() {
        let mesh = create_triangle_mesh();
        let mut writer = PlyWriter::new().with_format(PlyFormat::BinaryBigEndian);
        Writer::add_mesh(&mut writer, &mesh, None).unwrap();
        let bytes = writer.write_to_vec().unwrap();
        let header_end = bytes
            .windows(b"end_header\n".len())
            .position(|window| window == b"end_header\n")
            .map(|idx| idx + b"end_header\n".len())
            .unwrap();
        let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
        assert!(header.contains("format binary_big_endian 1.0"));

        let mesh = PlyReader::read_from_bytes(&bytes).unwrap();
        assert_eq!(mesh.num_points(), 3);
        assert_eq!(mesh.num_faces(), 1);
    }

    #[test]
    fn test_write_preserves_int32_positions() {
        let mut mesh = Mesh::new();
        let mut pos_att = PointAttribute::new();
        pos_att.init(
            GeometryAttributeType::Position,
            3,
            DataType::Int32,
            false,
            2,
        );
        pos_att
            .buffer_mut()
            .write(0, &[1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
        pos_att
            .buffer_mut()
            .write(12, &[4, 0, 0, 0, 5, 0, 0, 0, 6, 0, 0, 0]);
        mesh.add_attribute(pos_att);

        let mut writer = PlyWriter::new();
        Writer::add_mesh(&mut writer, &mesh, None).unwrap();
        let output = String::from_utf8(writer.write_to_vec().unwrap()).unwrap();
        assert!(output.contains("property int x"));
        assert!(output.contains("1 2 3"));
    }
}

fn read_float2(mesh: &Mesh, att_id: i32, point_idx: usize) -> [f32; 2] {
    let att = mesh.attribute(att_id);
    let byte_stride = att.byte_stride() as usize;
    let buffer = att.buffer();
    let mut bytes = [0u8; 8];
    buffer.read(point_idx * byte_stride, &mut bytes);
    [
        f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
    ]
}

fn read_f64x3(att: &PointAttribute, point_idx: usize) -> [f64; 3] {
    let byte_stride = att.byte_stride() as usize;
    let buffer = att.buffer();
    let mut bytes = [0u8; 24];
    buffer.read(point_idx * byte_stride, &mut bytes);
    [
        f64::from_le_bytes(bytes[0..8].try_into().unwrap()),
        f64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        f64::from_le_bytes(bytes[16..24].try_into().unwrap()),
    ]
}

fn read_i32x3(att: &PointAttribute, point_idx: usize) -> [i32; 3] {
    let byte_stride = att.byte_stride() as usize;
    let buffer = att.buffer();
    let mut bytes = [0u8; 12];
    buffer.read(point_idx * byte_stride, &mut bytes);
    [
        i32::from_le_bytes(bytes[0..4].try_into().unwrap()),
        i32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        i32::from_le_bytes(bytes[8..12].try_into().unwrap()),
    ]
}

fn read_u32x3(att: &PointAttribute, point_idx: usize) -> [u32; 3] {
    let byte_stride = att.byte_stride() as usize;
    let buffer = att.buffer();
    let mut bytes = [0u8; 12];
    buffer.read(point_idx * byte_stride, &mut bytes);
    [
        u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
    ]
}

fn append_positions_from_attribute(
    positions: &mut PlyPositionData,
    att: &PointAttribute,
    num_points: usize,
) {
    if att.num_components() != 3 {
        return;
    }

    match att.data_type() {
        DataType::Float32 => {
            let values: Vec<[f32; 3]> = (0..num_points)
                .map(|i| {
                    let byte_stride = att.byte_stride() as usize;
                    let mut bytes = [0u8; 12];
                    att.buffer().read(i * byte_stride, &mut bytes);
                    [
                        f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                        f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                        f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                    ]
                })
                .collect();
            match positions {
                PlyPositionData::Float32(existing) => existing.extend(values),
                _ => {
                    positions.ensure_float32();
                    if let PlyPositionData::Float32(existing) = positions {
                        existing.extend(values);
                    }
                }
            }
        }
        DataType::Float64
            if positions.len() == 0 || matches!(positions, PlyPositionData::Float64(_)) =>
        {
            let values: Vec<[f64; 3]> = (0..num_points).map(|i| read_f64x3(att, i)).collect();
            match positions {
                PlyPositionData::Float32(existing) if existing.is_empty() => {
                    *positions = PlyPositionData::Float64(values);
                }
                PlyPositionData::Float64(existing) => existing.extend(values),
                _ => unreachable!(),
            }
        }
        DataType::Int32
            if positions.len() == 0 || matches!(positions, PlyPositionData::Int32(_)) =>
        {
            let values: Vec<[i32; 3]> = (0..num_points).map(|i| read_i32x3(att, i)).collect();
            match positions {
                PlyPositionData::Float32(existing) if existing.is_empty() => {
                    *positions = PlyPositionData::Int32(values);
                }
                PlyPositionData::Int32(existing) => existing.extend(values),
                _ => unreachable!(),
            }
        }
        DataType::Uint32
            if positions.len() == 0 || matches!(positions, PlyPositionData::Uint32(_)) =>
        {
            let values: Vec<[u32; 3]> = (0..num_points).map(|i| read_u32x3(att, i)).collect();
            match positions {
                PlyPositionData::Float32(existing) if existing.is_empty() => {
                    *positions = PlyPositionData::Uint32(values);
                }
                PlyPositionData::Uint32(existing) => existing.extend(values),
                _ => unreachable!(),
            }
        }
        _ => {
            let converted: Vec<[f32; 3]> = (0..num_points)
                .map(|i| read_numeric3_as_f32(att, i))
                .collect();
            positions.push_f32_slice(&converted);
        }
    }
}

fn read_numeric3_as_f32(att: &PointAttribute, point_idx: usize) -> [f32; 3] {
    match att.data_type() {
        DataType::Float64 => {
            let v = read_f64x3(att, point_idx);
            [v[0] as f32, v[1] as f32, v[2] as f32]
        }
        DataType::Int32 => {
            let v = read_i32x3(att, point_idx);
            [v[0] as f32, v[1] as f32, v[2] as f32]
        }
        DataType::Uint32 => {
            let v = read_u32x3(att, point_idx);
            [v[0] as f32, v[1] as f32, v[2] as f32]
        }
        _ => {
            let mut bytes = [0u8; 12];
            att.buffer()
                .read(point_idx * att.byte_stride() as usize, &mut bytes);
            [
                f32::from_le_bytes(bytes[0..4].try_into().unwrap()),
                f32::from_le_bytes(bytes[4..8].try_into().unwrap()),
                f32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            ]
        }
    }
}
