//! FBX binary format reader for meshes.
//!
//! Supports reading:
//! - Binary FBX format (versions 7.x)
//! - Vertex positions
//! - Polygon/face indices
//! - Vertex normals (if present)
//!
//! # Example
//!
//! ```ignore
//! use draco_io::fbx_reader::FbxReader;
//!
//! let reader = FbxReader::open("model.fbx")?;
//! let meshes = reader.read_meshes()?;
//! for mesh in meshes {
//!     println!("Mesh has {} vertices", mesh.num_points());
//! }
//! ```

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;

/// FBX file magic: "Kaydara FBX Binary  \0"
const FBX_MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \0";

/// FBX reader for binary FBX files.
pub struct FbxReader<R: Read + Seek> {
    reader: R,
    version: u32,
}

/// An FBX node with properties and children.
#[derive(Debug, Clone)]
pub struct FbxNode {
    pub name: String,
    pub properties: Vec<FbxProperty>,
    pub children: Vec<FbxNode>,
}

/// FBX property value.
#[derive(Debug, Clone)]
pub enum FbxProperty {
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    String(String),
    Raw(Vec<u8>),
    BoolArray(Vec<bool>),
    I32Array(Vec<i32>),
    I64Array(Vec<i64>),
    F32Array(Vec<f32>),
    F64Array(Vec<f64>),
}

impl FbxReader<BufReader<File>> {
    /// Open an FBX file from a path.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        Self::new(reader)
    }
}

// Implement the Reader trait for the concrete BufReader<File> specialization.
impl crate::traits::Reader for FbxReader<BufReader<File>> {
    fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        FbxReader::open(path)
    }

    fn read_meshes(&mut self) -> io::Result<Vec<draco_core::mesh::Mesh>> {
        // Call the inherent method which already reads all meshes.
        // Use fully qualified syntax to avoid recursion.
        FbxReader::read_meshes(self)
    }
}

impl crate::traits::SceneReader for FbxReader<BufReader<File>> {
    fn read_scene(&mut self) -> io::Result<crate::traits::Scene> {
        let nodes = self.read_nodes()?;

        // Build maps: id -> Model/Geometry nodes
        use std::collections::HashMap;
        let mut model_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut geometry_map: HashMap<i64, &FbxNode> = HashMap::new();
        let mut connections: Vec<(i64, i64)> = Vec::new(); // (child, parent)

        for n in &nodes {
            if n.name == "Objects" {
                for child in &n.children {
                    match child.name.as_str() {
                        "Model" => {
                            if let Some(FbxProperty::I64(id)) = child.properties.first() {
                                model_map.insert(*id, child);
                            }
                        }
                        "Geometry" => {
                            if let Some(FbxProperty::I64(id)) = child.properties.first() {
                                geometry_map.insert(*id, child);
                            }
                        }
                        _ => {}
                    }
                }
            } else if n.name == "Connections" {
                for c in &n.children {
                    // Expect properties: String("OO"), I64(child), I64(parent)
                    if let (
                        Some(FbxProperty::String(_kind)),
                        Some(FbxProperty::I64(child)),
                        Some(FbxProperty::I64(parent)),
                    ) = (
                        c.properties.first(),
                        c.properties.get(1),
                        c.properties.get(2),
                    ) {
                        connections.push((*child, *parent));
                    }
                }
            }
        }

        // Build parent map for models
        let mut model_children: HashMap<i64, Vec<i64>> = HashMap::new();
        for (child, parent) in connections.iter() {
            if model_map.contains_key(child) || model_map.contains_key(parent) {
                model_children.entry(*parent).or_default().push(*child);
            }
        }

        // Helper to parse transform from Model node's Properties70
        fn parse_transform(node: &FbxNode) -> Option<crate::traits::Transform> {
            let mut translation = None;
            let mut rotation = None;
            let mut scaling = None;

            for child in &node.children {
                if child.name == "Properties70" {
                    for prop in &child.children {
                        // property nodes often have first property as name string
                        if let Some(crate::fbx_reader::FbxProperty::String(name)) =
                            prop.properties.first()
                        {
                            if name.contains("Lcl Translation") {
                                // find F64Array in properties
                                for p in &prop.properties {
                                    if let crate::fbx_reader::FbxProperty::F64Array(arr) = p {
                                        if arr.len() >= 3 {
                                            translation =
                                                Some([arr[0] as f32, arr[1] as f32, arr[2] as f32]);
                                        }
                                    }
                                }
                            }
                            if name.contains("Lcl Rotation") {
                                for p in &prop.properties {
                                    if let crate::fbx_reader::FbxProperty::F64Array(arr) = p {
                                        if arr.len() >= 3 {
                                            rotation =
                                                Some([arr[0] as f32, arr[1] as f32, arr[2] as f32]);
                                        }
                                    }
                                }
                            }
                            if name.contains("Lcl Scaling") {
                                for p in &prop.properties {
                                    if let crate::fbx_reader::FbxProperty::F64Array(arr) = p {
                                        if arr.len() >= 3 {
                                            scaling =
                                                Some([arr[0] as f32, arr[1] as f32, arr[2] as f32]);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if translation.is_none() && rotation.is_none() && scaling.is_none() {
                return None;
            }

            // Build simple 4x4 matrix from TRS (rotation in degrees XYZ)
            let t = translation.unwrap_or([0.0, 0.0, 0.0]);
            let r_deg = rotation.unwrap_or([0.0, 0.0, 0.0]);
            let s = scaling.unwrap_or([1.0, 1.0, 1.0]);

            let rx = r_deg[0].to_radians();
            let ry = r_deg[1].to_radians();
            let rz = r_deg[2].to_radians();

            let (sx, cx) = rx.sin_cos();
            let (sy, cy) = ry.sin_cos();
            let (sz, cz) = rz.sin_cos();

            // Rotation matrices around X, Y, Z (Rz * Ry * Rx)
            let m00 = cz * cy;
            let m01 = cz * sy * sx - sz * cx;
            let m02 = cz * sy * cx + sz * sx;

            let m10 = sz * cy;
            let m11 = sz * sy * sx + cz * cx;
            let m12 = sz * sy * cx - cz * sx;

            let m20 = -sy;
            let m21 = cy * sx;
            let m22 = cy * cx;

            let mat = [
                [m00 * s[0], m01 * s[1], m02 * s[2], 0.0],
                [m10 * s[0], m11 * s[1], m12 * s[2], 0.0],
                [m20 * s[0], m21 * s[1], m22 * s[2], 0.0],
                [t[0], t[1], t[2], 1.0],
            ];

            Some(crate::traits::Transform { matrix: mat })
        }

        // Build nodes recursively
        fn build_model_node(
            id: i64,
            model_map: &std::collections::HashMap<i64, &FbxNode>,
            model_children: &std::collections::HashMap<i64, Vec<i64>>,
        ) -> crate::traits::SceneNode {
            let node_src = model_map.get(&id).unwrap();
            let mut node = crate::traits::SceneNode::new(Some(node_src.name.clone()));
            node.transform = parse_transform(node_src);

            if let Some(children) = model_children.get(&id) {
                for &cid in children {
                    if model_map.contains_key(&cid) {
                        node.children
                            .push(build_model_node(cid, model_map, model_children));
                    }
                }
            }
            node
        }

        // Map geometries to models and create parts
        let mut model_parts: std::collections::HashMap<i64, Vec<crate::traits::SceneObject>> =
            std::collections::HashMap::new();
        for (geom_id, geom_node) in geometry_map.iter() {
            if let Some(mesh) = self.geometry_to_mesh(geom_node)? {
                // find connection mapping geometry -> model
                for (child, parent) in connections.iter() {
                    if *child == *geom_id && model_map.contains_key(parent) {
                        let part = crate::traits::SceneObject {
                            name: Some(geom_node.name.clone()),
                            mesh: mesh.clone(),
                            transform: None,
                        };
                        model_parts.entry(*parent).or_default().push(part);
                    }
                }
            }
        }

        // Build root nodes: any model with parent 0 (or with no parent present)
        let mut root_nodes = Vec::new();
        // find top-level model ids
        let top_level: Vec<i64> = model_map
            .keys()
            .cloned()
            .filter(|id| {
                !connections
                    .iter()
                    .any(|(child, parent)| child == id && model_map.contains_key(parent))
            })
            .collect();

        for id in top_level {
            let mut root_node = build_model_node(id, &model_map, &model_children);
            // attach parts if present
            if let Some(parts) = model_parts.get(&id) {
                root_node.parts.extend(parts.clone());
            }
            root_nodes.push(root_node);
        }

        // Flatten parts for Scene.parts
        let mut all_parts = Vec::new();
        for parts in model_parts.values() {
            for p in parts {
                all_parts.push(p.clone());
            }
        }

        Ok(crate::traits::Scene {
            name: None,
            parts: all_parts,
            root_nodes,
        })
    }
}

impl<R: Read + Seek> FbxReader<R> {
    /// Create a new FBX reader from a reader.
    pub fn new(mut reader: R) -> io::Result<Self> {
        // Read and verify magic
        let mut magic = [0u8; 21];
        reader.read_exact(&mut magic)?;
        if &magic != FBX_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a valid binary FBX file",
            ));
        }

        // Skip 2 unknown bytes
        reader.seek(SeekFrom::Current(2))?;

        // Read version
        let mut version_bytes = [0u8; 4];
        reader.read_exact(&mut version_bytes)?;
        let version = u32::from_le_bytes(version_bytes);

        Ok(Self { reader, version })
    }

    /// Get the FBX file version.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Check if this is FBX 7.5+ (uses 64-bit offsets).
    fn is_64bit(&self) -> bool {
        self.version >= 7500
    }

    /// Read a node record.
    fn read_node(&mut self) -> io::Result<Option<FbxNode>> {
        let (end_offset, num_properties, _property_list_len, name_len) = if self.is_64bit() {
            let mut buf = [0u8; 25];
            self.reader.read_exact(&mut buf)?;
            let end_offset = u64::from_le_bytes([
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            ]);
            let num_properties = u64::from_le_bytes([
                buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
            ]);
            let property_list_len = u64::from_le_bytes([
                buf[16], buf[17], buf[18], buf[19], buf[20], buf[21], buf[22], buf[23],
            ]);
            let name_len = buf[24];
            (
                end_offset,
                num_properties as u32,
                property_list_len,
                name_len,
            )
        } else {
            let mut buf = [0u8; 13];
            self.reader.read_exact(&mut buf)?;
            let end_offset = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as u64;
            let num_properties = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
            let _property_list_len = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as u64;
            let name_len = buf[12];
            (end_offset, num_properties, _property_list_len, name_len)
        };

        // NULL record marks end of children
        if end_offset == 0 {
            return Ok(None);
        }

        // Read name
        let mut name_bytes = vec![0u8; name_len as usize];
        self.reader.read_exact(&mut name_bytes)?;
        let name = String::from_utf8_lossy(&name_bytes).to_string();

        // Read properties
        let mut properties = Vec::with_capacity(num_properties as usize);
        for _ in 0..num_properties {
            properties.push(self.read_property()?);
        }

        // Read children
        let mut children = Vec::new();
        let current_pos = self.reader.stream_position()?;
        if current_pos < end_offset {
            while let Some(child) = self.read_node()? {
                children.push(child);
            }
        }

        // Seek to end offset to be safe
        self.reader.seek(SeekFrom::Start(end_offset))?;

        Ok(Some(FbxNode {
            name,
            properties,
            children,
        }))
    }

    /// Read a property.
    fn read_property(&mut self) -> io::Result<FbxProperty> {
        let mut type_code = [0u8; 1];
        self.reader.read_exact(&mut type_code)?;

        match type_code[0] {
            b'C' => {
                let mut v = [0u8; 1];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::Bool(v[0] != 0))
            }
            b'Y' => {
                let mut v = [0u8; 2];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::I16(i16::from_le_bytes(v)))
            }
            b'I' => {
                let mut v = [0u8; 4];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::I32(i32::from_le_bytes(v)))
            }
            b'L' => {
                let mut v = [0u8; 8];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::I64(i64::from_le_bytes(v)))
            }
            b'F' => {
                let mut v = [0u8; 4];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::F32(f32::from_le_bytes(v)))
            }
            b'D' => {
                let mut v = [0u8; 8];
                self.reader.read_exact(&mut v)?;
                Ok(FbxProperty::F64(f64::from_le_bytes(v)))
            }
            b'S' | b'R' => {
                let mut len_bytes = [0u8; 4];
                self.reader.read_exact(&mut len_bytes)?;
                let len = u32::from_le_bytes(len_bytes) as usize;
                let mut data = vec![0u8; len];
                self.reader.read_exact(&mut data)?;
                if type_code[0] == b'S' {
                    Ok(FbxProperty::String(
                        String::from_utf8_lossy(&data).to_string(),
                    ))
                } else {
                    Ok(FbxProperty::Raw(data))
                }
            }
            b'b' => Ok(FbxProperty::BoolArray(self.read_array_bool()?)),
            b'i' => Ok(FbxProperty::I32Array(self.read_array_i32()?)),
            b'l' => Ok(FbxProperty::I64Array(self.read_array_i64()?)),
            b'f' => Ok(FbxProperty::F32Array(self.read_array_f32()?)),
            b'd' => Ok(FbxProperty::F64Array(self.read_array_f64()?)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown property type: {}", type_code[0] as char),
            )),
        }
    }

    /// Read array header and return (length, encoding, compressed_length).
    fn read_array_header(&mut self) -> io::Result<(u32, u32, u32)> {
        let mut buf = [0u8; 12];
        self.reader.read_exact(&mut buf)?;
        let array_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let encoding = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let compressed_len = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        Ok((array_len, encoding, compressed_len))
    }

    /// Read array data (handles compression).
    fn read_array_data(
        &mut self,
        encoding: u32,
        compressed_len: u32,
        uncompressed_size: usize,
    ) -> io::Result<Vec<u8>> {
        if encoding == 0 {
            let mut data = vec![0u8; uncompressed_size];
            self.reader.read_exact(&mut data)?;
            Ok(data)
        } else if encoding == 1 {
            // Deflate/zlib compressed
            let mut compressed = vec![0u8; compressed_len as usize];
            self.reader.read_exact(&mut compressed)?;

            #[cfg(feature = "compression")]
            {
                use miniz_oxide::inflate::decompress_to_vec_zlib;
                decompress_to_vec_zlib(&compressed).map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Decompression error: {:?}", e),
                    )
                })
            }

            #[cfg(not(feature = "compression"))]
            {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "FBX array compression not supported (enable 'compression' feature)",
                ))
            }
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unknown array encoding: {}", encoding),
            ))
        }
    }

    fn read_array_bool(&mut self) -> io::Result<Vec<bool>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(encoding, compressed_len, len as usize)?;
        Ok(data.into_iter().map(|b| b != 0).collect())
    }

    fn read_array_i32(&mut self) -> io::Result<Vec<i32>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(encoding, compressed_len, len as usize * 4)?;
        Ok(data
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }

    fn read_array_i64(&mut self) -> io::Result<Vec<i64>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(encoding, compressed_len, len as usize * 8)?;
        Ok(data
            .chunks_exact(8)
            .map(|c| i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect())
    }

    fn read_array_f32(&mut self) -> io::Result<Vec<f32>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(encoding, compressed_len, len as usize * 4)?;
        Ok(data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect())
    }

    fn read_array_f64(&mut self) -> io::Result<Vec<f64>> {
        let (len, encoding, compressed_len) = self.read_array_header()?;
        let data = self.read_array_data(encoding, compressed_len, len as usize * 8)?;
        Ok(data
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect())
    }

    /// Read all top-level nodes.
    pub fn read_nodes(&mut self) -> io::Result<Vec<FbxNode>> {
        // Seek to start of nodes (after header)
        self.reader.seek(SeekFrom::Start(27))?;

        let mut nodes = Vec::new();
        while let Some(node) = self.read_node()? {
            nodes.push(node);
        }
        Ok(nodes)
    }

    /// Read meshes from the FBX file.
    pub fn read_meshes(&mut self) -> io::Result<Vec<Mesh>> {
        let nodes = self.read_nodes()?;
        let mut meshes = Vec::new();

        // Find Objects node
        for node in &nodes {
            if node.name == "Objects" {
                for child in &node.children {
                    if child.name == "Geometry" {
                        if let Some(mesh) = self.geometry_to_mesh(child)? {
                            meshes.push(mesh);
                        }
                    }
                }
            }
        }

        Ok(meshes)
    }

    /// Convert a Geometry node to a Mesh.
    fn geometry_to_mesh(&self, geometry: &FbxNode) -> io::Result<Option<Mesh>> {
        let mut vertices: Option<Vec<f64>> = None;
        let mut polygon_indices: Option<Vec<i32>> = None;

        for child in &geometry.children {
            match child.name.as_str() {
                "Vertices" => {
                    if let Some(FbxProperty::F64Array(arr)) = child.properties.first() {
                        vertices = Some(arr.clone());
                    }
                }
                "PolygonVertexIndex" => {
                    if let Some(FbxProperty::I32Array(arr)) = child.properties.first() {
                        polygon_indices = Some(arr.clone());
                    }
                }
                _ => {}
            }
        }

        let vertices = match vertices {
            Some(v) => v,
            None => return Ok(None),
        };
        let polygon_indices = match polygon_indices {
            Some(p) => p,
            None => return Ok(None),
        };

        // Build mesh
        let mut mesh = Mesh::new();

        // Add positions
        let num_vertices = vertices.len() / 3;
        let mut pos_att = PointAttribute::new();
        pos_att.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            num_vertices,
        );
        let buffer = pos_att.buffer_mut();
        for i in 0..num_vertices {
            let x = vertices[i * 3] as f32;
            let y = vertices[i * 3 + 1] as f32;
            let z = vertices[i * 3 + 2] as f32;
            let bytes: Vec<u8> = [x, y, z].iter().flat_map(|v| v.to_le_bytes()).collect();
            buffer.write(i * 12, &bytes);
        }
        mesh.add_attribute(pos_att);

        // Parse polygon indices (FBX uses negative index to mark end of polygon)
        let mut faces: Vec<[u32; 3]> = Vec::new();
        let mut current_polygon: Vec<i32> = Vec::new();

        for &idx in &polygon_indices {
            if idx < 0 {
                // End of polygon (index is bitwise NOT of actual index)
                let actual_idx = !idx;
                current_polygon.push(actual_idx);

                // Triangulate polygon (simple fan triangulation)
                if current_polygon.len() >= 3 {
                    let v0 = current_polygon[0] as u32;
                    for i in 1..current_polygon.len() - 1 {
                        let v1 = current_polygon[i] as u32;
                        let v2 = current_polygon[i + 1] as u32;
                        faces.push([v0, v1, v2]);
                    }
                }
                current_polygon.clear();
            } else {
                current_polygon.push(idx);
            }
        }

        // Set faces
        mesh.set_num_faces(faces.len());
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

        // Match C++ Draco behavior: deduplicate point IDs in face-traversal order.
        // This ensures binary compatibility when encoding.
        mesh.deduplicate_point_ids();

        Ok(Some(mesh))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_fbx_magic() {
        let mut data = Vec::new();
        data.extend_from_slice(FBX_MAGIC);
        data.extend_from_slice(&[0x1A, 0x00]); // Unknown bytes
        data.extend_from_slice(&7300u32.to_le_bytes()); // Version 7.3
                                                        // Add null record to end nodes
        data.extend_from_slice(&[0u8; 13]);

        let cursor = Cursor::new(data);
        let reader = FbxReader::new(cursor).unwrap();
        assert_eq!(reader.version(), 7300);
    }

    #[test]
    fn test_invalid_magic() {
        let data = b"Not an FBX file at all";
        let cursor = Cursor::new(data.to_vec());
        assert!(FbxReader::new(cursor).is_err());
    }
}
