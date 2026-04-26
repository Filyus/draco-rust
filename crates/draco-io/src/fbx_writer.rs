//! FBX binary format writer for meshes.
//!
//! Supports writing:
//! - Binary FBX format (version 7.5 with 64-bit headers)
//! - Vertex positions
//! - Triangle faces
//! - Optional zlib compression for arrays (with `compression` feature)
//!
//! # Example
//!
//! ```ignore
//! use draco_io::fbx_writer::FbxWriter;
//! use draco_core::mesh::Mesh;
//!
//! let mesh: Mesh = /* ... */;
//! let mut writer = FbxWriter::new();
//! writer.add_mesh(&mesh, Some("MyMesh"));
//! writer.write("output.fbx")?;
//!
//! // With compression (requires 'compression' feature)
//! let mut writer = FbxWriter::new().with_compression(true);
//! writer.add_mesh(&mesh, Some("MyMesh"));
//! writer.write("output_compressed.fbx")?;
//! ```

use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;

use crate::traits::Writer;

/// FBX file magic: "Kaydara FBX Binary  \0"
const FBX_MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \0";

/// FBX version 7.5 (7500) - uses 64-bit node headers
const FBX_VERSION: u32 = 7500;

/// Size of a null record for 64-bit FBX
const NULL_RECORD_SIZE_64: usize = 25;

/// Size of a null record for 32-bit FBX
const NULL_RECORD_SIZE_32: usize = 13;

/// FBX binary format writer.
///
/// This struct provides a builder-style API for writing FBX files.
/// Meshes are added via `add_mesh()`, then written with `write()`.
///
/// # Example
///
/// ```ignore
/// use draco_io::fbx_writer::FbxWriter;
///
/// let mut writer = FbxWriter::new()
///     .with_compression(true)
///     .with_compression_threshold(64);
///
/// writer.add_mesh(&mesh, Some("CubeMesh"));
/// writer.write("output.fbx")?;
/// ```
#[derive(Debug, Clone)]
pub struct FbxWriter {
    /// Whether to compress arrays using zlib (requires `compression` feature).
    compress: bool,
    /// Minimum array size (in bytes) to consider for compression.
    compression_threshold: usize,
    /// Meshes to write, with optional names.
    meshes: Vec<MeshData>,
    /// ID allocator for generating unique object IDs.
    next_id: i64,
}

/// Internal mesh data storage.
#[derive(Debug, Clone)]
struct MeshData {
    vertices: Vec<f64>,
    indices: Vec<i32>,
    name: String,
    geometry_id: i64,
    model_id: i64,
}

impl Default for FbxWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl FbxWriter {
    /// Create a new FBX writer with default settings.
    pub fn new() -> Self {
        Self {
            compress: false,
            compression_threshold: 128,
            meshes: Vec::new(),
            next_id: 1000, // Start at 1000 to avoid reserved IDs (0 = root)
        }
    }

    /// Enable or disable zlib compression for arrays.
    ///
    /// Compression is only applied if the `compression` feature is enabled
    /// and the array size exceeds the compression threshold.
    pub fn with_compression(mut self, compress: bool) -> Self {
        self.compress = compress;
        self
    }

    /// Set the minimum byte size for arrays to be compressed.
    ///
    /// Arrays smaller than this threshold will not be compressed even
    /// if compression is enabled. Default is 128 bytes.
    pub fn with_compression_threshold(mut self, threshold: usize) -> Self {
        self.compression_threshold = threshold;
        self
    }

    /// Allocate a unique ID for an object.
    fn allocate_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Add a mesh to be written.
    /// Write the FBX file to the given path.
    pub fn write<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        self.write_to(&mut writer)
    }

    /// Write the FBX data to a writer.
    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> io::Result<()> {
        let options = WriterOptions {
            compress: self.compress,
            compression_threshold: self.compression_threshold,
        };

        // Write header
        writer.write_all(FBX_MAGIC)?;
        writer.write_all(&[0x1A, 0x00])?; // Reserved bytes
        writer.write_all(&FBX_VERSION.to_le_bytes())?;

        let is_64 = FBX_VERSION >= 7500;

        // Write standard FBX sections
        write_header_extension(writer, is_64)?;
        write_global_settings(writer, is_64)?;
        write_documents(writer, is_64)?;
        write_definitions(writer, is_64, &self.meshes)?;
        write_objects(writer, &self.meshes, is_64, &options)?;
        write_connections(writer, &self.meshes, is_64)?;

        // Write NULL record to mark end of top-level nodes
        write_null_record(writer, is_64)?;

        // Write footer
        write_footer(writer)?;

        Ok(())
    }

    /// Get the number of meshes added.
    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    /// Check if compression is enabled.
    pub fn is_compression_enabled(&self) -> bool {
        self.compress
    }
}

/// Internal options passed during writing.
struct WriterOptions {
    compress: bool,
    compression_threshold: usize,
}

// ============================================================================
// Trait Implementations
// ============================================================================

impl Writer for FbxWriter {
    fn new() -> Self {
        Self::default()
    }

    fn add_mesh(&mut self, mesh: &Mesh, name: Option<&str>) -> io::Result<()> {
        let geometry_id = self.allocate_id();
        let model_id = self.allocate_id();
        let name = name.unwrap_or("Mesh").to_string();

        // Extract vertices
        let vertices = extract_vertices(mesh);

        // Extract polygon indices
        let indices = extract_polygon_indices(mesh);

        self.meshes.push(MeshData {
            vertices,
            indices,
            name,
            geometry_id,
            model_id,
        });
        Ok(())
    }

    fn write<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        self.write(path)
    }

    fn vertex_count(&self) -> usize {
        self.meshes.iter().map(|m| m.vertices.len() / 3).sum()
    }

    fn face_count(&self) -> usize {
        self.meshes.iter().map(|m| m.indices.len() / 3).sum()
    }
}

// ============================================================================
// Convenience Functions (for backward compatibility)
// ============================================================================

/// Write a mesh to a binary FBX file.
///
/// This is a convenience function. For more control, use `FbxWriter` directly.
pub fn write_fbx_mesh<P: AsRef<Path>>(path: P, mesh: &Mesh) -> io::Result<()> {
    let mut writer = FbxWriter::new();
    Writer::add_mesh(&mut writer, mesh, None)?;
    writer.write(path)
}

/// Write a mesh to a binary FBX file with compression.
///
/// This is a convenience function. For more control, use `FbxWriter` directly.
#[cfg(feature = "compression")]
pub fn write_fbx_mesh_compressed<P: AsRef<Path>>(path: P, mesh: &Mesh) -> io::Result<()> {
    let mut writer = FbxWriter::new().with_compression(true);
    Writer::add_mesh(&mut writer, mesh, None)?;
    writer.write(path)
}

// ============================================================================
// Node Writing Infrastructure
// ============================================================================

/// Helper struct for writing FBX nodes.
struct NodeWriter<'a, W: Write + Seek> {
    writer: &'a mut W,
    start_pos: u64,
    properties_start: u64,
    num_properties: u64,
    is_64: bool,
}

impl<'a, W: Write + Seek> NodeWriter<'a, W> {
    fn start(writer: &'a mut W, name: &str, is_64: bool) -> io::Result<Self> {
        let start_pos = writer.stream_position()?;

        // Write placeholder for end offset, num properties, property list len
        let header_size = if is_64 { 24 } else { 12 }; // 3 * 8 or 3 * 4
        writer.write_all(&vec![0u8; header_size])?;

        // Write name length and name
        writer.write_all(&[name.len() as u8])?;
        writer.write_all(name.as_bytes())?;

        let properties_start = writer.stream_position()?;

        Ok(Self {
            writer,
            start_pos,
            properties_start,
            num_properties: 0,
            is_64,
        })
    }

    fn write_property_i16(&mut self, value: i16) -> io::Result<()> {
        self.writer.write_all(b"Y")?;
        self.writer.write_all(&value.to_le_bytes())?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_i32(&mut self, value: i32) -> io::Result<()> {
        self.writer.write_all(b"I")?;
        self.writer.write_all(&value.to_le_bytes())?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_i64(&mut self, value: i64) -> io::Result<()> {
        self.writer.write_all(b"L")?;
        self.writer.write_all(&value.to_le_bytes())?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_f64(&mut self, value: f64) -> io::Result<()> {
        self.writer.write_all(b"D")?;
        self.writer.write_all(&value.to_le_bytes())?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_string(&mut self, value: &str) -> io::Result<()> {
        self.writer.write_all(b"S")?;
        self.writer.write_all(&(value.len() as u32).to_le_bytes())?;
        self.writer.write_all(value.as_bytes())?;
        self.num_properties += 1;
        Ok(())
    }

    fn write_property_f64_array(
        &mut self,
        values: &[f64],
        options: &WriterOptions,
    ) -> io::Result<()> {
        self.write_array_property(b'd', values, options, |v| v.to_le_bytes().to_vec())
    }

    fn write_property_i32_array(
        &mut self,
        values: &[i32],
        options: &WriterOptions,
    ) -> io::Result<()> {
        self.write_array_property(b'i', values, options, |v| v.to_le_bytes().to_vec())
    }

    fn write_array_property<T, F>(
        &mut self,
        type_code: u8,
        values: &[T],
        options: &WriterOptions,
        to_bytes: F,
    ) -> io::Result<()>
    where
        F: Fn(&T) -> Vec<u8>,
    {
        self.writer.write_all(&[type_code])?;
        self.writer
            .write_all(&(values.len() as u32).to_le_bytes())?;

        // Serialize the raw data
        let raw_data: Vec<u8> = values.iter().flat_map(&to_bytes).collect();
        let raw_size = raw_data.len();

        // Decide whether to compress
        let should_compress = options.compress && raw_size >= options.compression_threshold;

        #[cfg(feature = "compression")]
        if should_compress {
            use miniz_oxide::deflate::compress_to_vec_zlib;
            let compressed = compress_to_vec_zlib(&raw_data, 6); // Level 6 is a good balance

            // Only use compression if it actually saves space
            if compressed.len() < raw_size {
                self.writer.write_all(&1u32.to_le_bytes())?; // encoding = 1 (zlib)
                self.writer
                    .write_all(&(compressed.len() as u32).to_le_bytes())?;
                self.writer.write_all(&compressed)?;
                self.num_properties += 1;
                return Ok(());
            }
        }

        // Write uncompressed (or if compression didn't help)
        #[cfg(not(feature = "compression"))]
        let _ = should_compress; // Suppress unused warning

        self.writer.write_all(&0u32.to_le_bytes())?; // encoding = 0 (uncompressed)
        self.writer.write_all(&(raw_size as u32).to_le_bytes())?;
        self.writer.write_all(&raw_data)?;
        self.num_properties += 1;
        Ok(())
    }

    fn finish(self) -> io::Result<()> {
        // Write null record to end children section
        write_null_record(self.writer, self.is_64)?;
        self.finalize_header()
    }

    fn finish_with_children<F>(self, write_children: F) -> io::Result<()>
    where
        F: FnOnce(&mut W) -> io::Result<()>,
    {
        let properties_end = self.writer.stream_position()?;
        let property_list_len = properties_end - self.properties_start;

        // Write children
        write_children(self.writer)?;

        // Write null record to end children
        write_null_record(self.writer, self.is_64)?;

        let end_pos = self.writer.stream_position()?;

        // Write the header
        self.writer.seek(SeekFrom::Start(self.start_pos))?;
        if self.is_64 {
            self.writer.write_all(&end_pos.to_le_bytes())?;
            self.writer.write_all(&self.num_properties.to_le_bytes())?;
            self.writer.write_all(&property_list_len.to_le_bytes())?;
        } else {
            self.writer.write_all(&(end_pos as u32).to_le_bytes())?;
            self.writer
                .write_all(&(self.num_properties as u32).to_le_bytes())?;
            self.writer
                .write_all(&(property_list_len as u32).to_le_bytes())?;
        }

        // Seek back to end
        self.writer.seek(SeekFrom::Start(end_pos))?;
        Ok(())
    }

    fn finalize_header(self) -> io::Result<()> {
        let end_pos = self.writer.stream_position()?;
        let null_size = if self.is_64 {
            NULL_RECORD_SIZE_64
        } else {
            NULL_RECORD_SIZE_32
        };
        let property_list_len = if self.num_properties > 0 {
            end_pos - self.properties_start - null_size as u64
        } else {
            0u64
        };

        // Write the header
        self.writer.seek(SeekFrom::Start(self.start_pos))?;
        if self.is_64 {
            self.writer.write_all(&end_pos.to_le_bytes())?;
            self.writer.write_all(&self.num_properties.to_le_bytes())?;
            self.writer.write_all(&property_list_len.to_le_bytes())?;
        } else {
            self.writer.write_all(&(end_pos as u32).to_le_bytes())?;
            self.writer
                .write_all(&(self.num_properties as u32).to_le_bytes())?;
            self.writer
                .write_all(&(property_list_len as u32).to_le_bytes())?;
        }

        // Seek back to end
        self.writer.seek(SeekFrom::Start(end_pos))?;
        Ok(())
    }
}

fn write_null_record<W: Write>(writer: &mut W, is_64: bool) -> io::Result<()> {
    let size = if is_64 {
        NULL_RECORD_SIZE_64
    } else {
        NULL_RECORD_SIZE_32
    };
    writer.write_all(&vec![0u8; size])
}

// ============================================================================
// FBX Section Writers
// ============================================================================

fn write_header_extension<W: Write + Seek>(writer: &mut W, is_64: bool) -> io::Result<()> {
    let node = NodeWriter::start(writer, "FBXHeaderExtension", is_64)?;
    node.finish_with_children(|w| {
        // FBXHeaderVersion
        let mut ver = NodeWriter::start(w, "FBXHeaderVersion", is_64)?;
        ver.write_property_i32(1003)?;
        ver.finish()?;

        // FBXVersion
        let mut ver = NodeWriter::start(w, "FBXVersion", is_64)?;
        ver.write_property_i32(FBX_VERSION as i32)?;
        ver.finish()?;

        // Creator
        let mut creator = NodeWriter::start(w, "Creator", is_64)?;
        creator.write_property_string("draco-io-rs")?;
        creator.finish()?;

        Ok(())
    })
}

fn write_global_settings<W: Write + Seek>(writer: &mut W, is_64: bool) -> io::Result<()> {
    let node = NodeWriter::start(writer, "GlobalSettings", is_64)?;
    node.finish_with_children(|w| {
        // Version
        let mut ver = NodeWriter::start(w, "Version", is_64)?;
        ver.write_property_i32(1000)?;
        ver.finish()?;

        // Properties70 - proper FBX property format
        let props = NodeWriter::start(w, "Properties70", is_64)?;
        props.finish_with_children(|pw| {
            write_property_node(pw, is_64, "UpAxis", "int", "Integer", "", 1i32)?;
            write_property_node(pw, is_64, "UpAxisSign", "int", "Integer", "", 1i32)?;
            write_property_node(pw, is_64, "FrontAxis", "int", "Integer", "", 2i32)?;
            write_property_node(pw, is_64, "FrontAxisSign", "int", "Integer", "", 1i32)?;
            write_property_node(pw, is_64, "CoordAxis", "int", "Integer", "", 0i32)?;
            write_property_node(pw, is_64, "CoordAxisSign", "int", "Integer", "", 1i32)?;
            write_property_node_f64(pw, is_64, "UnitScaleFactor", "double", "Number", "", 1.0)?;
            Ok(())
        })
    })
}

fn write_property_node<W: Write + Seek>(
    writer: &mut W,
    is_64: bool,
    name: &str,
    type1: &str,
    type2: &str,
    flags: &str,
    value: i32,
) -> io::Result<()> {
    let mut p = NodeWriter::start(writer, "P", is_64)?;
    p.write_property_string(name)?;
    p.write_property_string(type1)?;
    p.write_property_string(type2)?;
    p.write_property_string(flags)?;
    p.write_property_i32(value)?;
    p.finish()
}

fn write_property_node_f64<W: Write + Seek>(
    writer: &mut W,
    is_64: bool,
    name: &str,
    type1: &str,
    type2: &str,
    flags: &str,
    value: f64,
) -> io::Result<()> {
    let mut p = NodeWriter::start(writer, "P", is_64)?;
    p.write_property_string(name)?;
    p.write_property_string(type1)?;
    p.write_property_string(type2)?;
    p.write_property_string(flags)?;
    p.write_property_f64(value)?;
    p.finish()
}

fn write_documents<W: Write + Seek>(writer: &mut W, is_64: bool) -> io::Result<()> {
    let node = NodeWriter::start(writer, "Documents", is_64)?;
    node.finish_with_children(|w| {
        let mut count = NodeWriter::start(w, "Count", is_64)?;
        count.write_property_i32(1)?;
        count.finish()?;

        let mut doc = NodeWriter::start(w, "Document", is_64)?;
        doc.write_property_i64(0)?; // Document ID (0 for root)
        doc.write_property_string("")?;
        doc.write_property_string("Scene")?;
        doc.finish()
    })
}

fn write_definitions<W: Write + Seek>(
    writer: &mut W,
    is_64: bool,
    meshes: &[MeshData],
) -> io::Result<()> {
    let node = NodeWriter::start(writer, "Definitions", is_64)?;
    node.finish_with_children(|w| {
        // Version
        let mut ver = NodeWriter::start(w, "Version", is_64)?;
        ver.write_property_i32(100)?;
        ver.finish()?;

        // Count of object types
        let mut count = NodeWriter::start(w, "Count", is_64)?;
        count.write_property_i32(2)?; // Geometry + Model
        count.finish()?;

        // ObjectType: Geometry
        write_object_type(w, is_64, "Geometry", meshes.len() as i32)?;

        // ObjectType: Model
        write_object_type(w, is_64, "Model", meshes.len() as i32)?;

        Ok(())
    })
}

fn write_object_type<W: Write + Seek>(
    writer: &mut W,
    is_64: bool,
    type_name: &str,
    count: i32,
) -> io::Result<()> {
    let mut ot = NodeWriter::start(writer, "ObjectType", is_64)?;
    ot.write_property_string(type_name)?;
    ot.finish_with_children(|w| {
        let mut c = NodeWriter::start(w, "Count", is_64)?;
        c.write_property_i32(count)?;
        c.finish()
    })
}

fn write_objects<W: Write + Seek>(
    writer: &mut W,
    meshes: &[MeshData],
    is_64: bool,
    options: &WriterOptions,
) -> io::Result<()> {
    let node = NodeWriter::start(writer, "Objects", is_64)?;
    node.finish_with_children(|w| {
        for mesh_data in meshes {
            write_geometry(w, mesh_data, is_64, options)?;
            write_model(w, mesh_data, is_64)?;
        }
        Ok(())
    })
}

fn write_model<W: Write + Seek>(
    writer: &mut W,
    mesh_data: &MeshData,
    is_64: bool,
) -> io::Result<()> {
    let mut node = NodeWriter::start(writer, "Model", is_64)?;
    node.write_property_i64(mesh_data.model_id)?;
    // Name::Class separator format
    let name_class = format!("{}\x00\x01Model", mesh_data.name);
    node.write_property_string(&name_class)?;
    node.write_property_string("Mesh")?;

    node.finish_with_children(|w| {
        let mut ver = NodeWriter::start(w, "Version", is_64)?;
        ver.write_property_i32(232)?;
        ver.finish()?;

        // Empty Properties70
        let props = NodeWriter::start(w, "Properties70", is_64)?;
        props.finish()?;

        // Shading
        let mut shading = NodeWriter::start(w, "Shading", is_64)?;
        shading.write_property_i16(1)?;
        shading.finish()?;

        // Culling
        let mut culling = NodeWriter::start(w, "Culling", is_64)?;
        culling.write_property_string("CullingOff")?;
        culling.finish()?;

        Ok(())
    })
}

fn write_geometry<W: Write + Seek>(
    writer: &mut W,
    mesh_data: &MeshData,
    is_64: bool,
    options: &WriterOptions,
) -> io::Result<()> {
    let mut node = NodeWriter::start(writer, "Geometry", is_64)?;
    node.write_property_i64(mesh_data.geometry_id)?;
    // Name::Class separator format
    let name_class = format!("{}\x00\x01Geometry", mesh_data.name);
    node.write_property_string(&name_class)?;
    node.write_property_string("Mesh")?;

    node.finish_with_children(|w| {
        // GeometryVersion
        let mut gver = NodeWriter::start(w, "GeometryVersion", is_64)?;
        gver.write_property_i32(124)?;
        gver.finish()?;

        // Write Vertices
        if !mesh_data.vertices.is_empty() {
            let mut vert_node = NodeWriter::start(w, "Vertices", is_64)?;
            vert_node.write_property_f64_array(&mesh_data.vertices, options)?;
            vert_node.finish()?;
        }

        // Write PolygonVertexIndex
        if !mesh_data.indices.is_empty() {
            let mut poly_node = NodeWriter::start(w, "PolygonVertexIndex", is_64)?;
            poly_node.write_property_i32_array(&mesh_data.indices, options)?;
            poly_node.finish()?;
        }

        Ok(())
    })
}

fn write_connections<W: Write + Seek>(
    writer: &mut W,
    meshes: &[MeshData],
    is_64: bool,
) -> io::Result<()> {
    let node = NodeWriter::start(writer, "Connections", is_64)?;
    node.finish_with_children(|w| {
        for mesh_data in meshes {
            // Connect Model to Scene Root (ID 0)
            let mut c1 = NodeWriter::start(w, "C", is_64)?;
            c1.write_property_string("OO")?;
            c1.write_property_i64(mesh_data.model_id)?;
            c1.write_property_i64(0)?; // Root ID
            c1.finish()?;

            // Connect Geometry to Model
            let mut c2 = NodeWriter::start(w, "C", is_64)?;
            c2.write_property_string("OO")?;
            c2.write_property_i64(mesh_data.geometry_id)?;
            c2.write_property_i64(mesh_data.model_id)?;
            c2.finish()?;
        }
        Ok(())
    })
}

fn write_footer<W: Write + Seek>(writer: &mut W) -> io::Result<()> {
    // FBX footer consists of padding and a footer signature
    let padding = [0u8; 20];
    writer.write_all(&padding)?;

    // Footer signature
    let footer_version: [u8; 4] = [0xFA, 0xBC, 0xAB, 0x09];
    writer.write_all(&footer_version)?;

    // Pad to align to 16-byte boundary
    let pos = writer.stream_position()?;
    let padding_needed = (16 - (pos % 16)) % 16;
    if padding_needed > 0 {
        writer.write_all(&vec![0u8; padding_needed as usize])?;
    }

    Ok(())
}

// ============================================================================
// Mesh Data Extraction
// ============================================================================

fn extract_vertices(mesh: &Mesh) -> Vec<f64> {
    let pos_att_id = mesh.named_attribute_id(GeometryAttributeType::Position);
    if pos_att_id < 0 {
        return Vec::new();
    }

    let att = mesh.attribute(pos_att_id);
    let byte_stride = att.byte_stride() as usize;
    let buffer = att.buffer();
    let mut vertices = Vec::with_capacity(mesh.num_points() * 3);

    for i in 0..mesh.num_points() {
        let mut bytes = [0u8; 12];
        buffer.read(i * byte_stride, &mut bytes);
        let x = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64;
        let y = f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as f64;
        let z = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as f64;
        vertices.push(x);
        vertices.push(y);
        vertices.push(z);
    }
    vertices
}

fn extract_polygon_indices(mesh: &Mesh) -> Vec<i32> {
    let mut indices = Vec::with_capacity(mesh.num_faces() * 3);
    for i in 0..mesh.num_faces() as u32 {
        let face = mesh.face(FaceIndex(i));
        indices.push(face[0].0 as i32);
        indices.push(face[1].0 as i32);
        // Last index is bitwise NOT to mark end of polygon
        indices.push(!(face[2].0 as i32));
    }
    indices
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::PointAttribute;
    use draco_core::geometry_indices::PointIndex;
    use std::io::Cursor;
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
    fn test_fbx_writer_new() {
        let writer = FbxWriter::new();
        assert_eq!(writer.mesh_count(), 0);
        assert!(!writer.is_compression_enabled());
    }

    #[test]
    fn test_fbx_writer_with_options() {
        let writer = FbxWriter::new()
            .with_compression(true)
            .with_compression_threshold(64);
        assert!(writer.is_compression_enabled());
    }

    #[test]
    fn test_fbx_writer_add_mesh() {
        let mesh = create_triangle_mesh();
        let mut writer = FbxWriter::new();
        Writer::add_mesh(&mut writer, &mesh, Some("TestMesh")).unwrap();
        assert_eq!(writer.mesh_count(), 1);
    }

    #[test]
    fn test_fbx_writer_write() {
        let mesh = create_triangle_mesh();
        let mut writer = FbxWriter::new();
        Writer::add_mesh(&mut writer, &mesh, Some("Triangle")).unwrap();

        let mut buffer = Cursor::new(Vec::new());
        writer.write_to(&mut buffer).unwrap();

        let data = buffer.into_inner();

        // Check magic
        assert_eq!(&data[0..21], FBX_MAGIC);
        // Check version
        let version = u32::from_le_bytes([data[23], data[24], data[25], data[26]]);
        assert_eq!(version, FBX_VERSION);
    }

    #[test]
    fn test_write_fbx_mesh_convenience() {
        let mesh = create_triangle_mesh();
        let file = NamedTempFile::new().unwrap();
        write_fbx_mesh(file.path(), &mesh).unwrap();

        let metadata = std::fs::metadata(file.path()).unwrap();
        assert!(metadata.len() > 27);
    }

    #[test]
    fn test_multiple_meshes() {
        let mesh1 = create_triangle_mesh();
        let mesh2 = create_triangle_mesh();

        let mut writer = FbxWriter::new();
        Writer::add_mesh(&mut writer, &mesh1, Some("Mesh1")).unwrap();
        Writer::add_mesh(&mut writer, &mesh2, Some("Mesh2")).unwrap();

        assert_eq!(writer.mesh_count(), 2);

        let mut buffer = Cursor::new(Vec::new());
        writer.write_to(&mut buffer).unwrap();

        let data = buffer.into_inner();
        assert!(!data.is_empty());
    }

    #[cfg(feature = "compression")]
    #[test]
    fn test_write_with_compression() {
        let mesh = create_triangle_mesh();
        let mut writer = FbxWriter::new()
            .with_compression(true)
            .with_compression_threshold(0);
        Writer::add_mesh(&mut writer, &mesh, None).unwrap();

        let mut buffer = Cursor::new(Vec::new());
        writer.write_to(&mut buffer).unwrap();

        let data = buffer.into_inner();
        assert!(!data.is_empty());
    }
}
