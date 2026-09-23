//! PLY format writer for meshes and point clouds.
//!
//! Supports writing:
//! - ASCII PLY format
//! - Binary little-endian PLY format
//! - Binary big-endian PLY format
//! - Vertex positions
//! - Vertex normals (if present)
//! - Vertex colors (if present)
//! - Per-vertex texture coordinates (if present)
//! - Named `Generic` attributes as vertex properties of their own, when asked
//!   for with [`PlyWriter::with_generic_attributes`]
//! - Triangle faces (for meshes)
//!
//! # Example
//!
//! ```no_run
//! use draco_io::{PlyWriter, PointCloudWriter, Writer};
//!
//! let mesh = draco_core::mesh::Mesh::new();
//! let mut writer = PlyWriter::new();
//! writer.add_mesh(&mesh, None)?;
//! writer.write("output.ply")?;
//!
//! // Or write point cloud
//! let mut writer = PlyWriter::new();
//! writer.add_points(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
//! writer.write("points.ply")?;
//! # Ok::<(), std::io::Error>(())
//! ```

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;

pub use crate::ply_format::PlyFormat;
use crate::traits::{PointCloudWriter, WriteToBytes, Writer};

/// PLY format writer.
///
/// This struct provides a builder-style API for writing PLY files.
/// Meshes or points are added, then written with `write()`.
///
/// # Example
///
/// ```no_run
/// use draco_io::{PlyWriter, Writer};
/// # let mesh = draco_core::mesh::Mesh::new();
///
/// let mut writer = PlyWriter::new();
/// writer.add_mesh(&mesh, None)?;
/// writer.write("cube.ply")?;
/// # Ok::<(), std::io::Error>(())
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
    /// Whether named `Generic` attributes are written.
    carry_generics: bool,
    /// Collected named `Generic` attributes, one column per property.
    generics: Vec<GenericColumn>,
}

/// One vertex property written from a named `Generic` attribute.
#[derive(Debug, Clone)]
struct GenericColumn {
    /// The property name in the header.
    name: String,
    /// The component type, which decides both the PLY type and the width.
    data_type: DataType,
    /// One little-endian value per vertex added so far.
    bytes: Vec<u8>,
}

/// The PLY scalar type a component type is written as, if it has one.
///
/// PLY has no 64-bit integers, so those have none and are refused rather than
/// narrowed: a writer asked to carry a value should not change it on the way.
/// A `Bool` is a byte in Draco and goes out as one.
fn ply_scalar_type(data_type: DataType) -> Option<&'static str> {
    Some(match data_type {
        DataType::Int8 => "char",
        DataType::Uint8 | DataType::Bool => "uchar",
        DataType::Int16 => "short",
        DataType::Uint16 => "ushort",
        DataType::Int32 => "int",
        DataType::Uint32 => "uint",
        DataType::Float32 => "float",
        DataType::Float64 => "double",
        _ => return None,
    })
}

/// Property names a generic cannot take: every name the reader can claim.
///
/// Not only the names this writer declares. The reader also takes a `u`/`v` or
/// `s`/`t` pair as texture coordinates when both halves are `float` -- so two
/// generics written as `u` and `v` would come back as a texture coordinate
/// rather than as themselves. A lone one would survive, but whether it stays
/// lone depends on the other generics of the mesh, and a name that is safe or
/// not depending on its neighbours is not one to hand out.
const RESERVED_PROPERTY_NAMES: [&str; 16] = [
    "x",
    "y",
    "z",
    "nx",
    "ny",
    "nz",
    "red",
    "green",
    "blue",
    "alpha",
    "texture_u",
    "texture_v",
    "u",
    "v",
    "s",
    "t",
];

/// A little-endian value of the given type, as ASCII PLY spells it.
///
/// Floats are written in the shortest form that reads back to the same value,
/// not to the six places positions get: a splat's harmonics sit around a
/// thousandth, where six places would keep three significant digits.
fn ascii_scalar(data_type: DataType, bytes: &[u8]) -> String {
    let mut wide = [0u8; 8];
    wide[..bytes.len()].copy_from_slice(bytes);
    match data_type {
        DataType::Int8 => (wide[0] as i8).to_string(),
        DataType::Uint8 | DataType::Bool => wide[0].to_string(),
        DataType::Int16 => i16::from_le_bytes([wide[0], wide[1]]).to_string(),
        DataType::Uint16 => u16::from_le_bytes([wide[0], wide[1]]).to_string(),
        DataType::Int32 => i32::from_le_bytes([wide[0], wide[1], wide[2], wide[3]]).to_string(),
        DataType::Uint32 => u32::from_le_bytes([wide[0], wide[1], wide[2], wide[3]]).to_string(),
        DataType::Float32 => f32::from_le_bytes([wide[0], wide[1], wide[2], wide[3]]).to_string(),
        _ => f64::from_le_bytes(wide).to_string(),
    }
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

    /// Write named `Generic` attributes as vertex properties of their own.
    ///
    /// The writing half of
    /// [`PlyReader::with_generic_attributes`](crate::PlyReader::with_generic_attributes):
    /// a `Generic` attribute whose metadata carries a `"name"` entry -- the key
    /// upstream Draco writes and reads -- becomes a property under that name,
    /// in the type it holds. A Gaussian-splat PLY read with that option on
    /// writes back out whole rather than as bare positions.
    ///
    /// A multi-component attribute is spread into `name_0`, `name_1`, ...,
    /// since a PLY property holds one value. A generic without a name is not
    /// written, because nothing says what to call it.
    ///
    /// Off by default, because it changes what a write produces. On, a mesh
    /// whose generics cannot be written faithfully is refused before anything
    /// of it is added: a 64-bit integer, which PLY has no type for; a name
    /// that is not one token of printable ASCII, which the header cannot
    /// hold; a name the reader claims for something else, such as `x`, `red`
    /// or the texture coordinates `u`/`v` and `s`/`t`; and a name an earlier
    /// mesh gave a different type.
    pub fn with_generic_attributes(mut self, enabled: bool) -> Self {
        self.carry_generics = enabled;
        self
    }

    /// The mutable form of [`with_generic_attributes`](Self::with_generic_attributes).
    pub fn set_generic_attributes(&mut self, enabled: bool) -> &mut Self {
        self.carry_generics = enabled;
        self
    }

    /// The named generics `mesh` would add, checked, without adding anything.
    ///
    /// Each entry is the property name, the attribute id and the component.
    fn plan_generics(&self, mesh: &Mesh) -> io::Result<Vec<(String, i32, usize)>> {
        let mut planned: Vec<(String, i32, usize)> = Vec::new();
        if !self.carry_generics {
            return Ok(planned);
        }
        let refuse = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
        for id in 0..mesh.num_attributes() {
            let attribute = mesh.attribute(id);
            if attribute.attribute_type() != GeometryAttributeType::Generic {
                continue;
            }
            let Some(name) = mesh
                .attribute_metadata_by_unique_id(attribute.unique_id())
                .and_then(|metadata| metadata.metadata().get_string("name"))
            else {
                continue;
            };
            let data_type = attribute.data_type();
            if ply_scalar_type(data_type).is_none() {
                return Err(refuse(format!(
                    "PLY has no type for the {data_type:?} attribute \"{name}\""
                )));
            }
            // A PLY header is ASCII text split on whitespace, so a name has to
            // be one token of printable ASCII to be a name at all.
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_graphic()) {
                return Err(refuse(format!(
                    "\"{name}\" cannot name a PLY property: a name is one token of \
                     printable ASCII, without whitespace"
                )));
            }
            let components = attribute.num_components() as usize;
            for component in 0..components {
                let property = if components == 1 {
                    name.to_string()
                } else {
                    format!("{name}_{component}")
                };
                if RESERVED_PROPERTY_NAMES.contains(&property.as_str()) {
                    return Err(refuse(format!(
                        "the attribute \"{property}\" would reuse a property this writer declares"
                    )));
                }
                if planned.iter().any(|(taken, _, _)| *taken == property) {
                    return Err(refuse(format!(
                        "two attributes of one mesh would both write \"{property}\""
                    )));
                }
                if let Some(column) = self.generics.iter().find(|column| column.name == property) {
                    if column.data_type != data_type {
                        return Err(refuse(format!(
                            "\"{property}\" is {:?} in an earlier mesh and {data_type:?} here",
                            column.data_type
                        )));
                    }
                }
                planned.push((property, id, component));
            }
        }
        Ok(planned)
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

    /// Brings every non-empty per-vertex list up to the current vertex count.
    fn pad_optional_lists(&mut self) {
        let vertex_count = self.positions.len();
        if !self.normals.is_empty() {
            self.normals.resize(vertex_count, [0.0, 0.0, 0.0]);
        }
        if !self.colors.is_empty() {
            self.colors.resize(vertex_count, [255, 255, 255, 255]);
        }
        if !self.texcoords.is_empty() {
            self.texcoords.resize(vertex_count, [0.0, 0.0]);
        }
        for column in &mut self.generics {
            column
                .bytes
                .resize(vertex_count * column.data_type.byte_length(), 0);
        }
    }

    /// Write the PLY data to a writer.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let has_normals = self.normals.len() == self.positions.len();
        let has_colors = self.colors.len() == self.positions.len() && self.color_components > 0;

        let has_texcoords = self.texcoords.len() == self.positions.len();
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

        if has_texcoords {
            writeln!(writer, "property float texture_u")?;
            writeln!(writer, "property float texture_v")?;
        }

        for column in &self.generics {
            let ply_type = ply_scalar_type(column.data_type)
                .expect("a column is only created for a type PLY has");
            writeln!(writer, "property {ply_type} {}", column.name)?;
        }

        if !self.faces.is_empty() {
            writeln!(writer, "element face {}", self.faces.len())?;
            writeln!(writer, "property list uchar int vertex_indices")?;
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

            if has_texcoords {
                let [u, v] = self.texcoords[i];
                write!(writer, " {:.6} {:.6}", u, v)?;
            }

            for column in &self.generics {
                write!(
                    writer,
                    " {}",
                    ascii_scalar(column.data_type, &generic_value(column, i))
                )?;
            }

            writeln!(writer)?;
        }

        // Write faces
        for face in &self.faces {
            write!(writer, "3 {} {} {}", face[0], face[1], face[2])?;
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

            if has_texcoords {
                let [u, v] = self.texcoords[i];
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

            for column in &self.generics {
                let mut value = generic_value(column, i);
                if big_endian {
                    value.reverse();
                }
                writer.write_all(&value)?;
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
        }

        Ok(())
    }
}

/// A generic column's value for one vertex, little-endian.
///
/// Zero for a vertex added after the column's last mesh -- `add_points` does
/// not pad the columns -- which is what an absent value is everywhere else in
/// this writer.
fn generic_value(column: &GenericColumn, vertex: usize) -> Vec<u8> {
    let width = column.data_type.byte_length();
    column
        .bytes
        .get(vertex * width..(vertex + 1) * width)
        .map_or_else(|| vec![0; width], <[u8]>::to_vec)
}

/// Read a float3 from an attribute at a given point index.
fn read_float3(mesh: &Mesh, att_id: i32, point_idx: usize) -> [f32; 3] {
    read_components_as_f32::<3>(mesh.attribute(att_id), point_idx)
}

/// Read a color from an attribute at a given point index.
fn read_color(mesh: &Mesh, att_id: i32, point_idx: usize) -> [u8; 4] {
    let att = mesh.attribute(att_id);
    let byte_stride = att.byte_stride() as usize;
    let buffer = att.buffer();

    // Colors can be stored in different formats. A zero component count would
    // divide by zero here, and both reads below take their length from the
    // component count while the buffer's length is independent of it - the
    // attribute descriptor comes from the bitstream, so neither is a given.
    let num_components = att.num_components() as usize;
    if num_components == 0 {
        return [255, 255, 255, 255];
    }
    let component_size = byte_stride / num_components;

    if component_size == 1 {
        // u8 colors
        let mut bytes = [255u8; 4];
        let read_len = num_components.min(4);
        if !buffer.try_read(
            crate::traits::value_offset(att, point_idx),
            &mut bytes[..read_len],
        ) {
            return [255, 255, 255, 255];
        }
        bytes
    } else if component_size == 4 {
        // f32 colors (0.0-1.0) - convert to u8
        let mut float_bytes = [0u8; 16];
        let read_len = (num_components * 4).min(16);
        if !buffer.try_read(
            crate::traits::value_offset(att, point_idx),
            &mut float_bytes[..read_len],
        ) {
            return [255, 255, 255, 255];
        }

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
        crate::traits::ensure_attributes_cover_points(mesh, "PLY")?;
        // Checked before anything is added, so a refused mesh leaves the
        // writer as it found it.
        let planned_generics = self.plan_generics(mesh)?;
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
            while self.normals.len() < vertex_offset as usize {
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
            // Floored at three, not at one: the header always declares
            // `red`/`green`/`blue`, and the binary payload writes exactly
            // `color_components` bytes per vertex. A one- or two-component
            // colour attribute therefore used to promise three values and
            // write one, which is a payload the reader runs off the end of --
            // `read_color` already pads the missing channels with 255, so
            // three is what the file has to carry.
            let components = color_att.num_components().clamp(3, 4);
            self.color_components = self.color_components.max(components);
            // Pad colors if we've added vertices without colors before
            while self.colors.len() < vertex_offset as usize {
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
                while self.texcoords.len() < vertex_offset as usize {
                    self.texcoords.push([0.0, 0.0]);
                }
                for i in 0..mesh.num_points() {
                    self.texcoords.push(read_float2(mesh, texcoord_att_id, i));
                }
            }
        }

        for (property, att_id, component) in planned_generics {
            let attribute = mesh.attribute(att_id);
            let data_type = attribute.data_type();
            let width = data_type.byte_length();
            let index = match self
                .generics
                .iter()
                .position(|column| column.name == property)
            {
                Some(index) => index,
                None => {
                    self.generics.push(GenericColumn {
                        name: property,
                        data_type,
                        bytes: Vec::new(),
                    });
                    self.generics.len() - 1
                }
            };
            let column = &mut self.generics[index];
            // Up to where this mesh starts: a property first seen now reads as
            // zero for every vertex an earlier mesh added.
            column.bytes.resize(vertex_offset as usize * width, 0);
            for point in 0..mesh.num_points() {
                let mut value = vec![0u8; width];
                let offset = crate::traits::value_offset(attribute, point) + component * width;
                if !attribute.buffer().try_read(offset, &mut value) {
                    value.fill(0);
                }
                column.bytes.extend_from_slice(&value);
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

        // Each optional list is padded up to the vertex count before this
        // mesh's values are appended - and has to be padded up to the new one
        // afterwards too, for the mesh that does not carry the attribute at
        // all. Without this the list stays short, `write_to` finds its length
        // unequal to the position count and drops the property entirely: a
        // mesh with normals followed by one without wrote a file with no
        // normals in it, while the same two meshes in the other order kept
        // them. Only lists something has already contributed to are padded, so
        // a file gains no property that no mesh ever had.
        self.pad_optional_lists();
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

impl WriteToBytes for PlyWriter {
    fn write_to_vec(&self) -> io::Result<Vec<u8>> {
        PlyWriter::write_to_vec(self)
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        PlyWriter::write_to(self, writer)
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
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    #[cfg(feature = "ply-reader")]
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

    /// A point cloud of `points` points with the given named generics, each a
    /// single component of the given type, values handed in as `f64`.
    fn with_named_generics(points: usize, generics: &[(&str, DataType, &[f64])]) -> Mesh {
        let mut mesh = Mesh::new();
        let mut position = PointAttribute::new();
        position.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            points,
        );
        for point in 0..points {
            let bytes: Vec<u8> = [point as f32, 0.0, 0.0]
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            position.buffer_mut().write(point * 12, &bytes);
        }
        mesh.add_attribute(position);
        for (name, data_type, values) in generics {
            let mut attribute = PointAttribute::new();
            attribute.init(GeometryAttributeType::Generic, 1, *data_type, false, points);
            let bytes: Vec<u8> = values
                .iter()
                .flat_map(|value| match data_type {
                    DataType::Float32 => (*value as f32).to_le_bytes().to_vec(),
                    DataType::Float64 => value.to_le_bytes().to_vec(),
                    DataType::Uint8 => vec![*value as u8],
                    DataType::Int16 => (*value as i16).to_le_bytes().to_vec(),
                    DataType::Uint32 => (*value as u32).to_le_bytes().to_vec(),
                    DataType::Int64 => (*value as i64).to_le_bytes().to_vec(),
                    other => panic!("no fixture encoding for {other:?}"),
                })
                .collect();
            attribute.buffer_mut().write(0, &bytes);
            let id = mesh.add_attribute(attribute);
            if !name.is_empty() {
                let unique_id = mesh.attribute(id).unique_id();
                let mut metadata = draco_core::metadata::Metadata::new();
                metadata.set_string("name", *name).unwrap();
                mesh.metadata_or_insert()
                    .set_attribute_metadata(unique_id, metadata);
            }
        }
        mesh
    }

    /// The values of the generic named `name`, widened to `f64`.
    #[cfg(feature = "ply-reader")]
    fn read_generic(mesh: &Mesh, name: &str) -> Option<(DataType, Vec<f64>)> {
        (0..mesh.num_attributes()).find_map(|id| {
            let attribute = mesh.attribute(id);
            let named = mesh
                .attribute_metadata_by_unique_id(attribute.unique_id())
                .and_then(|metadata| metadata.metadata().get_string("name"))
                .is_some_and(|found| found == name);
            if !named {
                return None;
            }
            let data_type = attribute.data_type();
            let width = data_type.byte_length();
            let values = (0..mesh.num_points())
                .map(|point| {
                    let mut bytes = vec![0u8; width];
                    attribute
                        .buffer()
                        .read(crate::traits::value_offset(attribute, point), &mut bytes);
                    match data_type {
                        DataType::Float32 => f32::from_le_bytes(bytes.try_into().unwrap()) as f64,
                        DataType::Float64 => f64::from_le_bytes(bytes.try_into().unwrap()),
                        DataType::Uint8 => bytes[0] as f64,
                        DataType::Int16 => i16::from_le_bytes(bytes.try_into().unwrap()) as f64,
                        DataType::Uint32 => u32::from_le_bytes(bytes.try_into().unwrap()) as f64,
                        other => panic!("no fixture decoding for {other:?}"),
                    }
                })
                .collect();
            Some((data_type, values))
        })
    }

    /// What the reader carries with generics on, this writes back out: names,
    /// declared types and exact values, in all three encodings.
    ///
    /// The float values are chosen to need more than six decimal places,
    /// which is where a splat's harmonics sit and where positions' fixed
    /// ASCII precision would already have rounded them away.
    #[test]
    #[cfg(feature = "ply-reader")]
    fn named_generics_round_trip_through_the_reader() {
        let opacity = [-3.25, 0.000123456, 13.3246];
        let segment = [0.0, 200.0, 255.0];
        let weight = [1.0e-9, -2.5, 0.1];
        let offset = [-300.0, 0.0, 32767.0];
        let label = [0.0, 16_777_217.0, 4_000_000_000.0];
        let mesh = with_named_generics(
            3,
            &[
                ("opacity", DataType::Float32, &opacity),
                ("segment", DataType::Uint8, &segment),
                ("weight", DataType::Float64, &weight),
                ("offset", DataType::Int16, &offset),
                ("label", DataType::Uint32, &label),
            ],
        );

        for format in [
            PlyFormat::Ascii,
            PlyFormat::BinaryLittleEndian,
            PlyFormat::BinaryBigEndian,
        ] {
            let mut writer = PlyWriter::new()
                .with_format(format)
                .with_generic_attributes(true);
            Writer::add_mesh(&mut writer, &mesh, None).unwrap();
            let bytes = writer.write_to_vec().unwrap();

            let read = PlyReader::from_bytes(bytes)
                .with_generic_attributes(true)
                .read_mesh()
                .unwrap();
            let expect = |name: &str, data_type: DataType, values: &[f64]| {
                let (found_type, found) = read_generic(&read, name)
                    .unwrap_or_else(|| panic!("{format:?}: {name} did not come back"));
                assert_eq!(found_type, data_type, "{format:?}: {name}");
                assert_eq!(found, values, "{format:?}: {name}");
            };
            expect(
                "opacity",
                DataType::Float32,
                &opacity.map(|value| value as f32 as f64),
            );
            expect("segment", DataType::Uint8, &segment);
            expect("weight", DataType::Float64, &weight);
            expect("offset", DataType::Int16, &offset);
            expect("label", DataType::Uint32, &label);
        }
    }

    /// Values PLY's text form has no spelling for in its specification --
    /// not-a-number and the infinities -- still go out as tokens a C `strtod`
    /// accepts, and read back as themselves.
    #[test]
    #[cfg(feature = "ply-reader")]
    fn non_finite_floats_survive_ascii() {
        let values = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e-45, 3.0e38];
        let mesh = with_named_generics(5, &[("weight", DataType::Float32, &values)]);
        let mut writer = PlyWriter::new()
            .with_format(PlyFormat::Ascii)
            .with_generic_attributes(true);
        Writer::add_mesh(&mut writer, &mesh, None).unwrap();
        let read = PlyReader::from_bytes(writer.write_to_vec().unwrap())
            .with_generic_attributes(true)
            .read_mesh()
            .unwrap();
        let (_, back) = read_generic(&read, "weight").unwrap();
        assert!(back[0].is_nan());
        assert_eq!(
            back[1..],
            [
                f64::INFINITY,
                f64::NEG_INFINITY,
                1e-45f32 as f64,
                3.0e38f32 as f64
            ]
        );
    }

    /// Off unless asked for, so an existing caller's output does not change.
    #[test]
    fn named_generics_are_not_written_by_default() {
        let mesh = with_named_generics(2, &[("opacity", DataType::Float32, &[1.0, 2.0])]);
        let mut writer = PlyWriter::new();
        Writer::add_mesh(&mut writer, &mesh, None).unwrap();
        let text = String::from_utf8(writer.write_to_vec().unwrap()).unwrap();
        assert!(!text.contains("opacity"), "{text}");
    }

    /// A generic with no name has nothing to be called in a header, so it is
    /// the one kind left out.
    #[test]
    fn an_unnamed_generic_is_not_written() {
        let mesh = with_named_generics(
            2,
            &[
                ("", DataType::Float32, &[1.0, 2.0]),
                ("opacity", DataType::Float32, &[3.0, 4.0]),
            ],
        );
        let mut writer = PlyWriter::new().with_generic_attributes(true);
        Writer::add_mesh(&mut writer, &mesh, None).unwrap();
        let text = String::from_utf8(writer.write_to_vec().unwrap()).unwrap();
        let properties = text
            .lines()
            .filter(|line| line.starts_with("property"))
            .count();
        assert_eq!(properties, 4, "x, y, z and opacity:\n{text}");
    }

    /// Meshes are merged the way every other per-vertex property is: a mesh
    /// without the property reads as zero for it, whichever order they come in.
    #[test]
    #[cfg(feature = "ply-reader")]
    fn a_mesh_without_the_property_reads_as_zero_for_it() {
        let with = with_named_generics(2, &[("opacity", DataType::Float32, &[5.0, 6.0])]);
        let without = with_named_generics(2, &[]);
        for (first, second, expected) in [
            (&with, &without, [5.0, 6.0, 0.0, 0.0]),
            (&without, &with, [0.0, 0.0, 5.0, 6.0]),
        ] {
            let mut writer = PlyWriter::new().with_generic_attributes(true);
            Writer::add_mesh(&mut writer, first, None).unwrap();
            Writer::add_mesh(&mut writer, second, None).unwrap();
            let read = PlyReader::from_bytes(writer.write_to_vec().unwrap())
                .with_generic_attributes(true)
                .read_mesh()
                .unwrap();
            assert_eq!(read_generic(&read, "opacity").unwrap().1, expected);
        }
    }

    /// A mesh whose generics cannot be written faithfully is refused whole,
    /// and the writer keeps what it had rather than half of the mesh.
    #[test]
    fn unwritable_generics_are_refused_before_anything_is_added() {
        let refused = |mesh: Mesh, why: &str| {
            let mut writer = PlyWriter::new().with_generic_attributes(true);
            let error = Writer::add_mesh(&mut writer, &mesh, None)
                .expect_err(why)
                .to_string();
            assert_eq!(
                writer.vertex_count(),
                0,
                "{why}: the writer took part of it"
            );
            error
        };

        let error = refused(
            with_named_generics(1, &[("id", DataType::Int64, &[7.0])]),
            "PLY has no 64-bit integer type",
        );
        assert!(error.contains("no type"), "{error}");

        let error = refused(
            with_named_generics(1, &[("my weight", DataType::Float32, &[1.0])]),
            "whitespace would split the header line",
        );
        assert!(error.contains("whitespace"), "{error}");

        let error = refused(
            with_named_generics(1, &[("непрозрачность", DataType::Float32, &[1.0])]),
            "a PLY header is ASCII",
        );
        assert!(error.contains("ASCII"), "{error}");

        // Claimed by the reader as a texture coordinate, though this writer
        // never declares it: written, it would not read back as itself.
        let error = refused(
            with_named_generics(1, &[("v", DataType::Float32, &[1.0])]),
            "the reader takes v as a texture coordinate",
        );
        assert!(error.contains("reuse"), "{error}");

        let error = refused(
            with_named_generics(1, &[("red", DataType::Float32, &[1.0])]),
            "red is a colour channel this writer declares",
        );
        assert!(error.contains("reuse"), "{error}");

        // The same name in a later mesh with a different type.
        let mut writer = PlyWriter::new().with_generic_attributes(true);
        let first = with_named_generics(1, &[("opacity", DataType::Float32, &[1.0])]);
        Writer::add_mesh(&mut writer, &first, None).unwrap();
        let second = with_named_generics(1, &[("opacity", DataType::Float64, &[1.0])]);
        let error = Writer::add_mesh(&mut writer, &second, None)
            .expect_err("one property cannot hold two types")
            .to_string();
        assert!(error.contains("earlier mesh"), "{error}");
        assert_eq!(writer.vertex_count(), 1, "the refused mesh was not added");
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

    /// The header names three colour channels, so three is what every vertex
    /// has to carry -- including when the mesh's colour attribute has fewer.
    ///
    /// The binary payload wrote `color_components` bytes per vertex while the
    /// header always declared `red`/`green`/`blue`, so a single-component
    /// colour promised three values and delivered one. Found by
    /// `mesh_text_roundtrip`: the reader ran off the end of a file this writer
    /// had just produced.
    #[test]
    fn a_colour_attribute_with_one_component_still_writes_three_channels() {
        let mut mesh = create_triangle_mesh();
        let mut color_att = PointAttribute::new();
        color_att.init(GeometryAttributeType::Color, 1, DataType::Uint8, false, 3);
        color_att.buffer_mut().write(0, &[10, 20, 30]);
        mesh.add_attribute(color_att);

        let mut writer = PlyWriter::new().with_format(PlyFormat::BinaryLittleEndian);
        Writer::add_mesh(&mut writer, &mesh, None).unwrap();
        let bytes = writer.write_to_vec().unwrap();

        let header_end = bytes
            .windows(11)
            .position(|window| {
                window
                    == b"end_header
"
            })
            .unwrap()
            + 11;
        let header = String::from_utf8_lossy(&bytes[..header_end]);
        assert!(header.contains("property uchar blue"), "{header}");
        // Three vertices of three position floats plus three colour bytes,
        // then the one face: a `uchar` count and three `int` indices.
        assert_eq!(bytes.len() - header_end, 3 * (12 + 3) + (1 + 12));

        crate::ply_reader::PlyReader::read_from_bytes(&bytes)
            .expect("the reader must accept what this writer wrote");
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

    #[cfg(feature = "ply-reader")]
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

    #[cfg(feature = "ply-reader")]
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

    #[cfg(feature = "ply-reader")]
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
    read_components_as_f32::<2>(mesh.attribute(att_id), point_idx)
}

/// Reads up to `N` components of any scalar type as `f32`, without assuming the
/// element is as wide as the caller wants to read.
///
/// The typed readers around this one each slice a fixed number of bytes at
/// `point * byte_stride` through the panicking `DataBuffer::read`, which holds
/// only while the attribute really is the type they assume. It need not be: a
/// decoded `.drc` declares its own data type and component count, so a mesh
/// arriving from `MeshDecoder` may present Uint8x3 positions or Int16x3
/// normals, and reading twelve bytes from a three-byte element runs off the
/// buffer. This reads each component at its own width and stops at the end of
/// the buffer, so a truncated or narrower attribute yields zeros instead of a
/// panic.
fn read_components_as_f32<const N: usize>(att: &PointAttribute, point_idx: usize) -> [f32; N] {
    let mut out = [0.0f32; N];
    let component_size = att.data_type().byte_length();
    if component_size == 0 {
        return out;
    }
    let base = crate::traits::value_offset(att, point_idx);
    let available = att.num_components() as usize;
    let buffer = att.buffer();
    let mut raw = [0u8; 8];
    for (c, slot) in out.iter_mut().enumerate().take(available.min(N)) {
        let start = match base.checked_add(c * component_size) {
            Some(start) => start,
            None => return out,
        };
        let raw = &mut raw[..component_size];
        if !buffer.try_read(start, raw) {
            return out;
        }
        *slot = match att.data_type() {
            DataType::Int8 => raw[0] as i8 as f32,
            DataType::Uint8 => raw[0] as f32,
            DataType::Int16 => i16::from_le_bytes([raw[0], raw[1]]) as f32,
            DataType::Uint16 => u16::from_le_bytes([raw[0], raw[1]]) as f32,
            DataType::Int32 => i32::from_le_bytes(raw.try_into().unwrap()) as f32,
            DataType::Uint32 => u32::from_le_bytes(raw.try_into().unwrap()) as f32,
            DataType::Int64 => i64::from_le_bytes(raw.try_into().unwrap()) as f32,
            DataType::Uint64 => u64::from_le_bytes(raw.try_into().unwrap()) as f32,
            DataType::Float32 => f32::from_le_bytes(raw.try_into().unwrap()),
            DataType::Float64 => f64::from_le_bytes(raw.try_into().unwrap()) as f32,
            DataType::Bool => f32::from(raw[0] != 0),
            DataType::Invalid => 0.0,
        };
    }
    out
}

fn read_f64x3(att: &PointAttribute, point_idx: usize) -> [f64; 3] {
    let buffer = att.buffer();
    let mut bytes = [0u8; 24];
    // Zero where the value is narrower than three components: the attribute
    // has fewer, and reading past it would be reading the next value.
    if !buffer.try_read(crate::traits::value_offset(att, point_idx), &mut bytes) {
        bytes = [0u8; 24];
    }
    [
        f64::from_le_bytes(bytes[0..8].try_into().unwrap()),
        f64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        f64::from_le_bytes(bytes[16..24].try_into().unwrap()),
    ]
}

fn read_i32x3(att: &PointAttribute, point_idx: usize) -> [i32; 3] {
    let buffer = att.buffer();
    let mut bytes = [0u8; 12];
    // Zero where the value is narrower than three components: the attribute
    // has fewer, and reading past it would be reading the next value.
    if !buffer.try_read(crate::traits::value_offset(att, point_idx), &mut bytes) {
        bytes = [0u8; 12];
    }
    [
        i32::from_le_bytes(bytes[0..4].try_into().unwrap()),
        i32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        i32::from_le_bytes(bytes[8..12].try_into().unwrap()),
    ]
}

fn read_u32x3(att: &PointAttribute, point_idx: usize) -> [u32; 3] {
    let buffer = att.buffer();
    let mut bytes = [0u8; 12];
    // Zero where the value is narrower than three components: the attribute
    // has fewer, and reading past it would be reading the next value.
    if !buffer.try_read(crate::traits::value_offset(att, point_idx), &mut bytes) {
        bytes = [0u8; 12];
    }
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
                    let mut bytes = [0u8; 12];
                    if !att
                        .buffer()
                        .try_read(crate::traits::value_offset(att, i), &mut bytes)
                    {
                        bytes = [0u8; 12];
                    }
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
        _ => read_components_as_f32::<3>(att, point_idx),
    }
}
