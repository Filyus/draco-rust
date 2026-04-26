//! PLY format reader for meshes and point clouds.
//!
//! Provides both a struct-based API (`PlyReader`) and convenience functions.

use byteorder::{BigEndian, LittleEndian, ReadBytesExt};
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::Path;

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::mesh::Mesh;

pub use crate::ply_format::PlyFormat;
use crate::traits::{PointCloudReader, Reader};

#[derive(Debug)]
struct ParsedPlyColorData {
    num_components: u8,
    values: Vec<[u8; 4]>,
}

#[derive(Debug)]
struct ParsedPlyData {
    positions: ParsedPlyPositionData,
    faces: Vec<[u32; 3]>,
    normals: Option<Vec<[f32; 3]>>,
    colors: Option<ParsedPlyColorData>,
}

#[derive(Debug)]
enum ParsedPlyPositionData {
    Float32(Vec<[f32; 3]>),
    Int32(Vec<[i32; 3]>),
}

impl ParsedPlyPositionData {
    fn len(&self) -> usize {
        match self {
            ParsedPlyPositionData::Float32(values) => values.len(),
            ParsedPlyPositionData::Int32(values) => values.len(),
        }
    }

    fn to_f32_positions(&self) -> Vec<[f32; 3]> {
        match self {
            ParsedPlyPositionData::Float32(values) => values.clone(),
            ParsedPlyPositionData::Int32(values) => values
                .iter()
                .map(|value| [value[0] as f32, value[1] as f32, value[2] as f32])
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
enum PlyPropertyKind {
    Scalar(DataType),
    List {
        count_type: DataType,
        item_type: DataType,
    },
}

#[derive(Debug, Clone)]
struct PlyPropertyDef {
    name: String,
    kind: PlyPropertyKind,
}

impl PlyPropertyDef {
    fn scalar_type(&self) -> Option<DataType> {
        match self.kind {
            PlyPropertyKind::Scalar(data_type) => Some(data_type),
            PlyPropertyKind::List { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
struct PlyHeader {
    format: PlyFormat,
    vertex_count: usize,
    face_count: usize,
    elements: Vec<PlyElementDef>,
    vertex_properties: Vec<PlyPropertyDef>,
    face_properties: Vec<PlyPropertyDef>,
}

#[derive(Debug, Clone)]
struct PlyElementDef {
    name: String,
    count: usize,
    properties: Vec<PlyPropertyDef>,
}

#[derive(Debug, Clone, Copy)]
struct PlyReadSchema {
    position_data_type: DataType,
    has_normals: bool,
    color_components: u8,
}

fn parse_ply_scalar_type(token: &str) -> Option<DataType> {
    match token {
        "char" | "int8" => Some(DataType::Int8),
        "uchar" | "uint8" => Some(DataType::Uint8),
        "short" | "int16" => Some(DataType::Int16),
        "ushort" | "uint16" => Some(DataType::Uint16),
        "int" | "int32" => Some(DataType::Int32),
        "uint" | "uint32" => Some(DataType::Uint32),
        "float" | "float32" => Some(DataType::Float32),
        "double" | "float64" => Some(DataType::Float64),
        _ => None,
    }
}

/// PLY format reader.
///
/// Reads vertex positions from ASCII and little-endian binary PLY files.
#[derive(Debug)]
pub struct PlyReader {
    source: PlyReaderSource,
}

#[derive(Debug, Clone)]
enum PlyReaderSource {
    Path(std::path::PathBuf),
    Bytes(Vec<u8>),
}

impl PlyReader {
    /// Open a PLY file for reading.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found: {}", path.display()),
            ));
        }
        Ok(Self {
            source: PlyReaderSource::Path(path),
        })
    }

    /// Create a PLY reader from in-memory bytes.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            source: PlyReaderSource::Bytes(bytes.into()),
        }
    }

    /// Read a mesh directly from in-memory bytes.
    pub fn read_from_bytes(bytes: &[u8]) -> io::Result<Mesh> {
        let mut reader = Self::from_bytes(bytes.to_vec());
        reader.read_mesh()
    }

    /// Read all positions from the PLY file.
    pub fn read_positions(&mut self) -> io::Result<Vec<[f32; 3]>> {
        Ok(read_ply_source(&self.source)?.positions.to_f32_positions())
    }

    /// Read a mesh with positions (and faces if present).
    pub fn read_mesh(&mut self) -> io::Result<Mesh> {
        let parsed = read_ply_source(&self.source)?;
        let mut mesh = Mesh::new();

        if parsed.positions.len() == 0 {
            return Ok(mesh);
        }

        mesh.set_num_points(parsed.positions.len());
        mesh.set_num_faces(parsed.faces.len());

        // Create position attribute
        match &parsed.positions {
            ParsedPlyPositionData::Float32(values) => {
                mesh.add_attribute(make_f32x3_attribute(
                    GeometryAttributeType::Position,
                    values,
                ));
            }
            ParsedPlyPositionData::Int32(values) => {
                mesh.add_attribute(make_i32x3_attribute(
                    GeometryAttributeType::Position,
                    values,
                ));
            }
        }

        if let Some(normals) = parsed.normals.as_ref() {
            mesh.add_attribute(make_f32x3_attribute(GeometryAttributeType::Normal, normals));
        }

        if let Some(colors) = parsed.colors.as_ref() {
            mesh.add_attribute(make_u8_attribute(
                GeometryAttributeType::Color,
                colors.num_components,
                true,
                &colors.values,
            ));
        }

        for (i, face) in parsed.faces.iter().enumerate() {
            mesh.set_face(
                draco_core::geometry_indices::FaceIndex(i as u32),
                [
                    draco_core::geometry_indices::PointIndex(face[0]),
                    draco_core::geometry_indices::PointIndex(face[1]),
                    draco_core::geometry_indices::PointIndex(face[2]),
                ],
            );
        }

        if mesh.num_faces() > 0 {
            // Match C++ Draco behavior: deduplicate point IDs in face-traversal order.
            // This ensures binary compatibility when encoding.
            mesh.deduplicate_point_ids();
        }

        Ok(mesh)
    }
}

impl Reader for PlyReader {
    fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        PlyReader::open(path)
    }

    fn read_meshes(&mut self) -> io::Result<Vec<Mesh>> {
        let m = self.read_mesh()?;
        Ok(vec![m])
    }
}

impl crate::traits::SceneReader for PlyReader {
    fn read_scene(&mut self) -> io::Result<crate::traits::Scene> {
        let meshes = self.read_meshes()?;
        let mut parts = Vec::with_capacity(meshes.len());
        let scene_name = match &self.source {
            PlyReaderSource::Path(path) => path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string()),
            PlyReaderSource::Bytes(_) => None,
        };
        let mut root = crate::traits::SceneNode::new(scene_name);
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

impl PointCloudReader for PlyReader {
    fn read_points(&mut self) -> io::Result<Vec<[f32; 3]>> {
        self.read_positions()
    }
}

// ============================================================================
// Convenience Functions (for backward compatibility)
// ============================================================================

/// Parse point positions from an ASCII or binary little-endian PLY file.
/// Returns a vec of [x, y, z] positions.
pub fn read_ply_positions<P: AsRef<Path>>(path: P) -> io::Result<Vec<[f32; 3]>> {
    Ok(read_ply(path)?.positions.to_f32_positions())
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

fn make_i32x3_attribute(
    attribute_type: GeometryAttributeType,
    values: &[[i32; 3]],
) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(attribute_type, 3, DataType::Int32, false, values.len());

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

fn make_u8_attribute(
    attribute_type: GeometryAttributeType,
    num_components: u8,
    normalized: bool,
    values: &[[u8; 4]],
) -> PointAttribute {
    let mut attribute = PointAttribute::new();
    attribute.init(
        attribute_type,
        num_components,
        DataType::Uint8,
        normalized,
        values.len(),
    );

    let buffer = attribute.buffer_mut();
    for (i, value) in values.iter().enumerate() {
        let end = num_components as usize;
        buffer.write(i * end, &value[..end]);
    }

    attribute
}

fn invalid_ply(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn parse_ply_property(parts: &[&str]) -> io::Result<PlyPropertyDef> {
    if parts.len() < 3 {
        return Err(invalid_ply("Malformed property declaration"));
    }

    if parts[1] == "list" {
        if parts.len() < 5 {
            return Err(invalid_ply("Malformed list property declaration"));
        }
        let count_type = parse_ply_scalar_type(parts[2])
            .ok_or_else(|| invalid_ply(format!("Unsupported PLY scalar type: {}", parts[2])))?;
        let item_type = parse_ply_scalar_type(parts[3])
            .ok_or_else(|| invalid_ply(format!("Unsupported PLY scalar type: {}", parts[3])))?;
        Ok(PlyPropertyDef {
            name: parts[4].to_string(),
            kind: PlyPropertyKind::List {
                count_type,
                item_type,
            },
        })
    } else {
        let data_type = parse_ply_scalar_type(parts[1])
            .ok_or_else(|| invalid_ply(format!("Unsupported PLY scalar type: {}", parts[1])))?;
        Ok(PlyPropertyDef {
            name: parts[2].to_string(),
            kind: PlyPropertyKind::Scalar(data_type),
        })
    }
}

fn parse_ply_header(bytes: &[u8]) -> io::Result<(PlyHeader, usize)> {
    if bytes.is_empty() {
        return Err(invalid_ply("Empty PLY file"));
    }

    let mut body_offset = None;
    let mut offset = 0usize;
    while offset < bytes.len() {
        let line_end = bytes[offset..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|idx| offset + idx);
        match line_end {
            Some(end) => {
                let line_bytes = if end > offset && bytes[end - 1] == b'\r' {
                    &bytes[offset..end - 1]
                } else {
                    &bytes[offset..end]
                };
                let line = std::str::from_utf8(line_bytes)
                    .map_err(|_| invalid_ply("PLY header must be valid UTF-8/ASCII"))?;
                offset = end + 1;
                if line.trim() == "end_header" {
                    body_offset = Some(offset);
                    break;
                }
            }
            None => {
                let line = std::str::from_utf8(&bytes[offset..])
                    .map_err(|_| invalid_ply("PLY header must be valid UTF-8/ASCII"))?;
                if line.trim() == "end_header" {
                    body_offset = Some(bytes.len());
                    break;
                }
                break;
            }
        }
    }

    let body_offset = body_offset.ok_or_else(|| invalid_ply("No end_header found"))?;
    let header_text = std::str::from_utf8(&bytes[..body_offset])
        .map_err(|_| invalid_ply("PLY header must be valid UTF-8/ASCII"))?;

    let mut lines = header_text.lines();
    let first_line = lines.next().ok_or_else(|| invalid_ply("Empty PLY file"))?;
    if first_line.trim() != "ply" {
        return Err(invalid_ply("Missing PLY header"));
    }

    let mut format = None;
    let mut vertex_count = 0usize;
    let mut face_count = 0usize;
    let mut elements: Vec<PlyElementDef> = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "end_header" {
            continue;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "comment" | "obj_info" => {}
            "format" => {
                if parts.len() < 2 {
                    return Err(invalid_ply("Malformed format declaration"));
                }
                format = Some(match parts[1] {
                    "ascii" => PlyFormat::Ascii,
                    "binary_little_endian" => PlyFormat::BinaryLittleEndian,
                    "binary_big_endian" => PlyFormat::BinaryBigEndian,
                    other => {
                        return Err(invalid_ply(format!("Unsupported PLY format: {other}")));
                    }
                });
            }
            "element" => {
                if parts.len() < 3 {
                    return Err(invalid_ply("Malformed element declaration"));
                }
                let count = parts[2]
                    .parse()
                    .map_err(|_| invalid_ply("Invalid element count"))?;
                elements.push(PlyElementDef {
                    name: parts[1].to_string(),
                    count,
                    properties: Vec::new(),
                });
                match parts[1] {
                    "vertex" => {
                        vertex_count = count;
                    }
                    "face" => {
                        face_count = count;
                    }
                    _ => {}
                }
            }
            "property" => {
                let property = parse_ply_property(&parts)?;
                let Some(element) = elements.last_mut() else {
                    return Err(invalid_ply("Property declared before element"));
                };
                element.properties.push(property);
            }
            _ => {}
        }
    }

    let mut vertex_properties = Vec::new();
    let mut face_properties = Vec::new();
    for element in &elements {
        match element.name.as_str() {
            "vertex" => vertex_properties = element.properties.clone(),
            "face" => face_properties = element.properties.clone(),
            _ => {}
        }
    }

    Ok((
        PlyHeader {
            format: format.ok_or_else(|| invalid_ply("Missing PLY format declaration"))?,
            vertex_count,
            face_count,
            elements,
            vertex_properties,
            face_properties,
        },
        body_offset,
    ))
}

fn skip_ascii_element_lines<'a>(lines: &mut std::str::Lines<'a>, count: usize) {
    for _ in 0..count {
        let _ = lines.next();
    }
}

fn ascii_scalar_token_count(data_type: DataType) -> usize {
    if data_type == DataType::Invalid {
        0
    } else {
        1
    }
}

fn split_ascii_vertex_lines<'a>(
    header: &PlyHeader,
    body_text: &'a str,
) -> io::Result<(Vec<&'a str>, Vec<&'a str>)> {
    let mut lines = body_text.lines();
    let mut vertex_lines = Vec::new();
    let mut face_lines = Vec::new();

    for element in &header.elements {
        match element.name.as_str() {
            "vertex" => {
                for _ in 0..element.count {
                    if let Some(line) = lines.next() {
                        vertex_lines.push(line);
                    }
                }
            }
            "face" => {
                for _ in 0..element.count {
                    if let Some(line) = lines.next() {
                        face_lines.push(line);
                    }
                }
            }
            _ => skip_ascii_element_lines(&mut lines, element.count),
        }
    }

    Ok((vertex_lines, face_lines))
}

fn position_data_type_for_scalar(data_type: DataType) -> DataType {
    match data_type {
        DataType::Int32 => DataType::Int32,
        _ => DataType::Float32,
    }
}

fn build_read_schema(header: &PlyHeader) -> io::Result<PlyReadSchema> {
    let mut has_x = false;
    let mut has_y = false;
    let mut has_z = false;
    let mut position_data_type = DataType::Float32;
    let mut prop_nx_type = None;
    let mut prop_ny_type = None;
    let mut prop_nz_type = None;
    let mut prop_r_type = None;
    let mut prop_g_type = None;
    let mut prop_b_type = None;
    let mut prop_a_type = None;

    for property in &header.vertex_properties {
        let Some(data_type) = property.scalar_type() else {
            continue;
        };

        match property.name.as_str() {
            "x" => {
                has_x = true;
                position_data_type = position_data_type_for_scalar(data_type);
            }
            "y" => {
                has_y = true;
                position_data_type = position_data_type_for_scalar(data_type);
            }
            "z" => {
                has_z = true;
                position_data_type = position_data_type_for_scalar(data_type);
            }
            "nx" => prop_nx_type = Some(data_type),
            "ny" => prop_ny_type = Some(data_type),
            "nz" => prop_nz_type = Some(data_type),
            "red" => prop_r_type = Some(data_type),
            "green" => prop_g_type = Some(data_type),
            "blue" => prop_b_type = Some(data_type),
            "alpha" => prop_a_type = Some(data_type),
            _ => {}
        }
    }

    if !has_x {
        return Err(invalid_ply("No x property"));
    }
    if !has_y {
        return Err(invalid_ply("No y property"));
    }
    if !has_z {
        return Err(invalid_ply("No z property"));
    }

    let has_normals = prop_nx_type == Some(DataType::Float32)
        && prop_ny_type == Some(DataType::Float32)
        && prop_nz_type == Some(DataType::Float32);

    let color_types = [prop_r_type, prop_g_type, prop_b_type, prop_a_type];
    let color_components = color_types.iter().flatten().count() as u8;
    if color_components > 0 {
        for color_type in color_types.into_iter().flatten() {
            if color_type != DataType::Uint8 {
                return Err(invalid_ply("Color properties must be uint8"));
            }
        }
    }

    Ok(PlyReadSchema {
        position_data_type,
        has_normals,
        color_components,
    })
}

fn triangulate_vertex_indices(indices: &[u32], faces: &mut Vec<[u32; 3]>) {
    if indices.len() < 3 {
        return;
    }

    for j in 1..indices.len() - 1 {
        faces.push([indices[0], indices[j], indices[j + 1]]);
    }
}

fn parse_ascii_face_line(
    header: &PlyHeader,
    line: &str,
    faces: &mut Vec<[u32; 3]>,
) -> io::Result<()> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(());
    }

    if header.face_properties.is_empty() {
        let indices: Vec<u32> = parts
            .iter()
            .map(|part| {
                part.parse::<u32>()
                    .map_err(|_| invalid_ply("Bad face index value"))
            })
            .collect::<io::Result<Vec<u32>>>()?;

        if indices.is_empty() {
            return Ok(());
        }

        let polygon_size = indices[0] as usize;
        if polygon_size < 3 || indices.len() < polygon_size + 1 {
            return Ok(());
        }

        triangulate_vertex_indices(&indices[1..polygon_size + 1], faces);
        return Ok(());
    }

    let mut cursor = 0usize;
    let mut polygon_indices: Option<Vec<u32>> = None;

    for property in &header.face_properties {
        match property.kind {
            PlyPropertyKind::Scalar(_) => {
                if cursor >= parts.len() {
                    return Ok(());
                }
                cursor += 1;
            }
            PlyPropertyKind::List { .. } => {
                if cursor >= parts.len() {
                    return Ok(());
                }
                let count: usize = parts[cursor]
                    .parse()
                    .map_err(|_| invalid_ply("Bad face list size"))?;
                cursor += 1;
                if parts.len() < cursor + count {
                    return Ok(());
                }

                let values = parts[cursor..cursor + count]
                    .iter()
                    .map(|part| {
                        part.parse::<u32>()
                            .map_err(|_| invalid_ply("Bad face index value"))
                    })
                    .collect::<io::Result<Vec<u32>>>()?;
                cursor += count;

                if property.name == "vertex_indices" || polygon_indices.is_none() {
                    polygon_indices = Some(values);
                }
            }
        }
    }

    if let Some(indices) = polygon_indices {
        triangulate_vertex_indices(&indices, faces);
    }

    Ok(())
}

fn parse_ascii_f32(token: &str, label: &str) -> io::Result<f32> {
    token
        .parse()
        .map_err(|_| invalid_ply(format!("Bad {label} value")))
}

fn parse_ascii_i32(token: &str, label: &str) -> io::Result<i32> {
    token
        .parse()
        .map_err(|_| invalid_ply(format!("Bad {label} value")))
}

fn parse_ascii_u8(token: &str) -> io::Result<u8> {
    token
        .parse()
        .map_err(|_| invalid_ply("Bad color component value"))
}

fn read_ply_ascii_body(header: &PlyHeader, body: &[u8]) -> io::Result<ParsedPlyData> {
    let schema = build_read_schema(header)?;
    let body_text = std::str::from_utf8(body)
        .map_err(|_| invalid_ply("ASCII PLY payload must be valid UTF-8/ASCII"))?;
    let (vertex_lines, face_lines) = split_ascii_vertex_lines(header, body_text)?;

    let mut float_positions = matches!(schema.position_data_type, DataType::Float32)
        .then(|| Vec::with_capacity(header.vertex_count));
    let mut int_positions = matches!(schema.position_data_type, DataType::Int32)
        .then(|| Vec::with_capacity(header.vertex_count));
    let mut normals = schema
        .has_normals
        .then(|| Vec::with_capacity(header.vertex_count));
    let mut colors = (schema.color_components > 0).then(|| ParsedPlyColorData {
        num_components: schema.color_components,
        values: Vec::with_capacity(header.vertex_count),
    });

    for line in vertex_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let mut float_position = [0.0f32; 3];
        let mut int_position = [0i32; 3];
        let mut normal = [0.0f32; 3];
        let mut color = [0u8; 4];
        let mut color_component = 0usize;
        let mut cursor = 0usize;

        for property in &header.vertex_properties {
            let Some(data_type) = property.scalar_type() else {
                if cursor >= parts.len() {
                    break;
                }
                let count: usize = parts[cursor]
                    .parse()
                    .map_err(|_| invalid_ply("Bad vertex list size"))?;
                cursor = cursor
                    .checked_add(1 + count)
                    .ok_or_else(|| invalid_ply("ASCII PLY line is too large"))?;
                continue;
            };
            if cursor >= parts.len() {
                break;
            }
            let token = parts[cursor];
            cursor += ascii_scalar_token_count(data_type);

            match property.name.as_str() {
                "x" => match schema.position_data_type {
                    DataType::Int32 => int_position[0] = parse_ascii_i32(token, "x")?,
                    _ => float_position[0] = parse_ascii_f32(token, "x")?,
                },
                "y" => match schema.position_data_type {
                    DataType::Int32 => int_position[1] = parse_ascii_i32(token, "y")?,
                    _ => float_position[1] = parse_ascii_f32(token, "y")?,
                },
                "z" => match schema.position_data_type {
                    DataType::Int32 => int_position[2] = parse_ascii_i32(token, "z")?,
                    _ => float_position[2] = parse_ascii_f32(token, "z")?,
                },
                "nx" if schema.has_normals => normal[0] = parse_ascii_f32(token, "nx")?,
                "ny" if schema.has_normals => normal[1] = parse_ascii_f32(token, "ny")?,
                "nz" if schema.has_normals => normal[2] = parse_ascii_f32(token, "nz")?,
                "red" | "green" | "blue" | "alpha" if schema.color_components > 0 => {
                    color[color_component] = parse_ascii_u8(token)?;
                    color_component += 1;
                }
                _ => {}
            }
        }

        match schema.position_data_type {
            DataType::Int32 => int_positions.as_mut().unwrap().push(int_position),
            _ => float_positions.as_mut().unwrap().push(float_position),
        }

        if let Some(normals) = normals.as_mut() {
            normals.push(normal);
        }

        if let Some(colors) = colors.as_mut() {
            colors.values.push(color);
        }
    }

    let mut faces = Vec::with_capacity(header.face_count);
    for line in face_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        parse_ascii_face_line(header, trimmed, &mut faces)?;
    }

    Ok(ParsedPlyData {
        positions: match schema.position_data_type {
            DataType::Int32 => ParsedPlyPositionData::Int32(int_positions.unwrap_or_default()),
            _ => ParsedPlyPositionData::Float32(float_positions.unwrap_or_default()),
        },
        faces,
        normals,
        colors,
    })
}

fn ensure_remaining(cursor: &Cursor<&[u8]>, bytes_needed: usize) -> io::Result<()> {
    let position = cursor.position() as usize;
    let end = position
        .checked_add(bytes_needed)
        .ok_or_else(|| invalid_ply("PLY payload is too large"))?;
    if end > cursor.get_ref().len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Unexpected end of binary PLY payload",
        ));
    }
    Ok(())
}

fn skip_binary_scalar(cursor: &mut Cursor<&[u8]>, data_type: DataType) -> io::Result<()> {
    ensure_remaining(cursor, data_type.byte_length())?;
    cursor.set_position(cursor.position() + data_type.byte_length() as u64);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum BinaryEndian {
    Little,
    Big,
}

fn read_binary_scalar_as_f32(
    cursor: &mut Cursor<&[u8]>,
    data_type: DataType,
    endian: BinaryEndian,
) -> io::Result<f32> {
    ensure_remaining(cursor, data_type.byte_length())?;
    match data_type {
        DataType::Int8 => cursor.read_i8().map(|value| value as f32),
        DataType::Uint8 => cursor.read_u8().map(|value| value as f32),
        DataType::Int16 => match endian {
            BinaryEndian::Little => cursor.read_i16::<LittleEndian>().map(|value| value as f32),
            BinaryEndian::Big => cursor.read_i16::<BigEndian>().map(|value| value as f32),
        },
        DataType::Uint16 => match endian {
            BinaryEndian::Little => cursor.read_u16::<LittleEndian>().map(|value| value as f32),
            BinaryEndian::Big => cursor.read_u16::<BigEndian>().map(|value| value as f32),
        },
        DataType::Int32 => match endian {
            BinaryEndian::Little => cursor.read_i32::<LittleEndian>().map(|value| value as f32),
            BinaryEndian::Big => cursor.read_i32::<BigEndian>().map(|value| value as f32),
        },
        DataType::Uint32 => match endian {
            BinaryEndian::Little => cursor.read_u32::<LittleEndian>().map(|value| value as f32),
            BinaryEndian::Big => cursor.read_u32::<BigEndian>().map(|value| value as f32),
        },
        DataType::Int64 => match endian {
            BinaryEndian::Little => cursor.read_i64::<LittleEndian>().map(|value| value as f32),
            BinaryEndian::Big => cursor.read_i64::<BigEndian>().map(|value| value as f32),
        },
        DataType::Uint64 => match endian {
            BinaryEndian::Little => cursor.read_u64::<LittleEndian>().map(|value| value as f32),
            BinaryEndian::Big => cursor.read_u64::<BigEndian>().map(|value| value as f32),
        },
        DataType::Float32 => match endian {
            BinaryEndian::Little => cursor.read_f32::<LittleEndian>(),
            BinaryEndian::Big => cursor.read_f32::<BigEndian>(),
        },
        DataType::Float64 => match endian {
            BinaryEndian::Little => cursor.read_f64::<LittleEndian>().map(|value| value as f32),
            BinaryEndian::Big => cursor.read_f64::<BigEndian>().map(|value| value as f32),
        },
        _ => Err(invalid_ply("Unsupported binary scalar type")),
    }
}

fn read_binary_scalar_as_i32(
    cursor: &mut Cursor<&[u8]>,
    data_type: DataType,
    endian: BinaryEndian,
) -> io::Result<i32> {
    ensure_remaining(cursor, data_type.byte_length())?;
    match data_type {
        DataType::Int8 => cursor.read_i8().map(|value| value as i32),
        DataType::Uint8 => cursor.read_u8().map(|value| value as i32),
        DataType::Int16 => match endian {
            BinaryEndian::Little => cursor.read_i16::<LittleEndian>().map(|value| value as i32),
            BinaryEndian::Big => cursor.read_i16::<BigEndian>().map(|value| value as i32),
        },
        DataType::Uint16 => match endian {
            BinaryEndian::Little => cursor.read_u16::<LittleEndian>().map(|value| value as i32),
            BinaryEndian::Big => cursor.read_u16::<BigEndian>().map(|value| value as i32),
        },
        DataType::Int32 => match endian {
            BinaryEndian::Little => cursor.read_i32::<LittleEndian>(),
            BinaryEndian::Big => cursor.read_i32::<BigEndian>(),
        },
        DataType::Uint32 => {
            let value = match endian {
                BinaryEndian::Little => cursor.read_u32::<LittleEndian>()?,
                BinaryEndian::Big => cursor.read_u32::<BigEndian>()?,
            };
            i32::try_from(value).map_err(|_| invalid_ply("Binary PLY value does not fit in int32"))
        }
        _ => Err(invalid_ply("Unsupported binary int32 scalar type")),
    }
}

fn read_binary_scalar_as_u8(cursor: &mut Cursor<&[u8]>, data_type: DataType) -> io::Result<u8> {
    ensure_remaining(cursor, data_type.byte_length())?;
    match data_type {
        DataType::Uint8 => cursor.read_u8(),
        DataType::Int8 => {
            let value = cursor.read_i8()?;
            u8::try_from(value).map_err(|_| invalid_ply("Negative color component value"))
        }
        _ => Err(invalid_ply("Color properties must be uint8")),
    }
}

fn read_binary_scalar_as_u32(
    cursor: &mut Cursor<&[u8]>,
    data_type: DataType,
    endian: BinaryEndian,
) -> io::Result<u32> {
    ensure_remaining(cursor, data_type.byte_length())?;
    match data_type {
        DataType::Uint8 => cursor.read_u8().map(|value| value as u32),
        DataType::Int8 => {
            let value = cursor.read_i8()?;
            u32::try_from(value).map_err(|_| invalid_ply("Negative face index value"))
        }
        DataType::Uint16 => match endian {
            BinaryEndian::Little => cursor.read_u16::<LittleEndian>().map(|value| value as u32),
            BinaryEndian::Big => cursor.read_u16::<BigEndian>().map(|value| value as u32),
        },
        DataType::Int16 => {
            let value = match endian {
                BinaryEndian::Little => cursor.read_i16::<LittleEndian>()?,
                BinaryEndian::Big => cursor.read_i16::<BigEndian>()?,
            };
            u32::try_from(value).map_err(|_| invalid_ply("Negative face index value"))
        }
        DataType::Uint32 => match endian {
            BinaryEndian::Little => cursor.read_u32::<LittleEndian>(),
            BinaryEndian::Big => cursor.read_u32::<BigEndian>(),
        },
        DataType::Int32 => {
            let value = match endian {
                BinaryEndian::Little => cursor.read_i32::<LittleEndian>()?,
                BinaryEndian::Big => cursor.read_i32::<BigEndian>()?,
            };
            u32::try_from(value).map_err(|_| invalid_ply("Negative face index value"))
        }
        _ => Err(invalid_ply("Unsupported face index scalar type")),
    }
}

fn read_binary_scalar_as_usize(
    cursor: &mut Cursor<&[u8]>,
    data_type: DataType,
    endian: BinaryEndian,
) -> io::Result<usize> {
    let value = read_binary_scalar_as_u32(cursor, data_type, endian)?;
    usize::try_from(value).map_err(|_| invalid_ply("Binary list size is too large"))
}

fn skip_binary_element(
    cursor: &mut Cursor<&[u8]>,
    element: &PlyElementDef,
    endian: BinaryEndian,
) -> io::Result<()> {
    for _ in 0..element.count {
        for property in &element.properties {
            match property.kind {
                PlyPropertyKind::Scalar(data_type) => skip_binary_scalar(cursor, data_type)?,
                PlyPropertyKind::List {
                    count_type,
                    item_type,
                } => {
                    let count = read_binary_scalar_as_usize(cursor, count_type, endian)?;
                    for _ in 0..count {
                        skip_binary_scalar(cursor, item_type)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn read_ply_binary_body(
    header: &PlyHeader,
    body: &[u8],
    endian: BinaryEndian,
) -> io::Result<ParsedPlyData> {
    let schema = build_read_schema(header)?;
    let mut cursor = Cursor::new(body);
    let vertex_element_index = header
        .elements
        .iter()
        .position(|element| element.name == "vertex")
        .ok_or_else(|| invalid_ply("Missing vertex element"))?;
    for element in &header.elements[..vertex_element_index] {
        skip_binary_element(&mut cursor, element, endian)?;
    }

    let mut float_positions = matches!(schema.position_data_type, DataType::Float32)
        .then(|| Vec::with_capacity(header.vertex_count));
    let mut int_positions = matches!(schema.position_data_type, DataType::Int32)
        .then(|| Vec::with_capacity(header.vertex_count));
    let mut normals = schema
        .has_normals
        .then(|| Vec::with_capacity(header.vertex_count));
    let mut colors = (schema.color_components > 0).then(|| ParsedPlyColorData {
        num_components: schema.color_components,
        values: Vec::with_capacity(header.vertex_count),
    });

    for _ in 0..header.vertex_count {
        let mut float_position = [0.0f32; 3];
        let mut int_position = [0i32; 3];
        let mut normal = [0.0f32; 3];
        let mut color = [0u8; 4];
        let mut color_component = 0usize;

        for property in &header.vertex_properties {
            match property.kind {
                PlyPropertyKind::Scalar(data_type) => match property.name.as_str() {
                    "x" => match schema.position_data_type {
                        DataType::Int32 => {
                            int_position[0] =
                                read_binary_scalar_as_i32(&mut cursor, data_type, endian)?
                        }
                        _ => {
                            float_position[0] =
                                read_binary_scalar_as_f32(&mut cursor, data_type, endian)?
                        }
                    },
                    "y" => match schema.position_data_type {
                        DataType::Int32 => {
                            int_position[1] =
                                read_binary_scalar_as_i32(&mut cursor, data_type, endian)?
                        }
                        _ => {
                            float_position[1] =
                                read_binary_scalar_as_f32(&mut cursor, data_type, endian)?
                        }
                    },
                    "z" => match schema.position_data_type {
                        DataType::Int32 => {
                            int_position[2] =
                                read_binary_scalar_as_i32(&mut cursor, data_type, endian)?
                        }
                        _ => {
                            float_position[2] =
                                read_binary_scalar_as_f32(&mut cursor, data_type, endian)?
                        }
                    },
                    "nx" if schema.has_normals => {
                        normal[0] = read_binary_scalar_as_f32(&mut cursor, data_type, endian)?
                    }
                    "ny" if schema.has_normals => {
                        normal[1] = read_binary_scalar_as_f32(&mut cursor, data_type, endian)?
                    }
                    "nz" if schema.has_normals => {
                        normal[2] = read_binary_scalar_as_f32(&mut cursor, data_type, endian)?
                    }
                    "red" | "green" | "blue" | "alpha" if schema.color_components > 0 => {
                        color[color_component] = read_binary_scalar_as_u8(&mut cursor, data_type)?;
                        color_component += 1;
                    }
                    _ => skip_binary_scalar(&mut cursor, data_type)?,
                },
                PlyPropertyKind::List {
                    count_type,
                    item_type,
                } => {
                    let count = read_binary_scalar_as_usize(&mut cursor, count_type, endian)?;
                    for _ in 0..count {
                        skip_binary_scalar(&mut cursor, item_type)?;
                    }
                }
            }
        }

        match schema.position_data_type {
            DataType::Int32 => int_positions.as_mut().unwrap().push(int_position),
            _ => float_positions.as_mut().unwrap().push(float_position),
        }

        if let Some(normals) = normals.as_mut() {
            normals.push(normal);
        }

        if let Some(colors) = colors.as_mut() {
            colors.values.push(color);
        }
    }

    let face_element_index = header
        .elements
        .iter()
        .position(|element| element.name == "face");
    if let Some(face_element_index) = face_element_index {
        if face_element_index < vertex_element_index {
            return Err(invalid_ply(
                "PLY face element before vertex element is not supported",
            ));
        }
        for element in &header.elements[vertex_element_index + 1..face_element_index] {
            skip_binary_element(&mut cursor, element, endian)?;
        }
    }

    if header.face_count > 0 && header.face_properties.is_empty() {
        return Err(invalid_ply(
            "Binary PLY faces require a face property declaration",
        ));
    }

    let mut faces = Vec::with_capacity(header.face_count);
    for _ in 0..header.face_count {
        let mut polygon_indices: Option<Vec<u32>> = None;

        for property in &header.face_properties {
            match property.kind {
                PlyPropertyKind::Scalar(data_type) => skip_binary_scalar(&mut cursor, data_type)?,
                PlyPropertyKind::List {
                    count_type,
                    item_type,
                } => {
                    let count = read_binary_scalar_as_usize(&mut cursor, count_type, endian)?;
                    let mut values = Vec::with_capacity(count);
                    for _ in 0..count {
                        values.push(read_binary_scalar_as_u32(&mut cursor, item_type, endian)?);
                    }

                    if property.name == "vertex_indices" || polygon_indices.is_none() {
                        polygon_indices = Some(values);
                    }
                }
            }
        }

        if let Some(indices) = polygon_indices {
            triangulate_vertex_indices(&indices, &mut faces);
        }
    }

    Ok(ParsedPlyData {
        positions: match schema.position_data_type {
            DataType::Int32 => ParsedPlyPositionData::Int32(int_positions.unwrap_or_default()),
            _ => ParsedPlyPositionData::Float32(float_positions.unwrap_or_default()),
        },
        faces,
        normals,
        colors,
    })
}

fn read_ply<P: AsRef<Path>>(path: P) -> io::Result<ParsedPlyData> {
    let bytes = fs::read(path)?;
    read_ply_bytes(&bytes)
}

fn read_ply_source(source: &PlyReaderSource) -> io::Result<ParsedPlyData> {
    match source {
        PlyReaderSource::Path(path) => read_ply(path),
        PlyReaderSource::Bytes(bytes) => read_ply_bytes(bytes),
    }
}

fn read_ply_bytes(bytes: &[u8]) -> io::Result<ParsedPlyData> {
    let (header, body_offset) = parse_ply_header(&bytes)?;

    match header.format {
        PlyFormat::Ascii => read_ply_ascii_body(&header, &bytes[body_offset..]),
        PlyFormat::BinaryLittleEndian => {
            read_ply_binary_body(&header, &bytes[body_offset..], BinaryEndian::Little)
        }
        PlyFormat::BinaryBigEndian => {
            read_ply_binary_body(&header, &bytes[body_offset..], BinaryEndian::Big)
        }
    }
}

/// Write point positions to an ASCII PLY file.
pub fn write_ply_positions<P: AsRef<Path>>(path: P, points: &[[f32; 3]]) -> io::Result<()> {
    let mut file = fs::File::create(path)?;

    writeln!(file, "ply")?;
    writeln!(file, "format ascii 1.0")?;
    writeln!(file, "element vertex {}", points.len())?;
    writeln!(file, "property float x")?;
    writeln!(file, "property float y")?;
    writeln!(file, "property float z")?;
    writeln!(file, "end_header")?;

    for p in points {
        writeln!(file, "{:.6} {:.6} {:.6}", p[0], p[1], p[2])?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use draco_core::geometry_attribute::GeometryAttributeType;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_write_ply() {
        let expected = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [-1.0, -1.0, -1.0],
        ];

        let file = NamedTempFile::new().unwrap();
        write_ply_positions(file.path(), &expected).unwrap();

        let positions = read_ply_positions(file.path()).unwrap();
        assert_eq!(positions.len(), expected.len());

        for (i, (a, b)) in positions.iter().zip(expected.iter()).enumerate() {
            let diff = (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs();
            assert!(
                diff < 1e-5,
                "Position mismatch at index {i}: {a:?} vs {b:?}"
            );
        }
    }

    #[test]
    fn test_read_mesh_parses_and_triangulates_faces() {
        let file = NamedTempFile::new().unwrap();
        let ply = r#"ply
format ascii 1.0
element vertex 4
property float x
property float y
property float z
element face 2
property list uchar int vertex_indices
end_header
0 0 0
1 0 0
1 1 0
0 1 0
3 0 1 2
4 0 1 2 3
"#;

        std::fs::write(file.path(), ply).unwrap();

        let mut reader = PlyReader::open(file.path()).unwrap();
        let mesh = reader.read_mesh().unwrap();

        assert_eq!(mesh.num_points(), 4);
        assert_eq!(mesh.num_faces(), 3);
        assert_eq!(
            mesh.face(draco_core::geometry_indices::FaceIndex(0)),
            [0u32.into(), 1u32.into(), 2u32.into()]
        );
        assert_eq!(
            mesh.face(draco_core::geometry_indices::FaceIndex(1)),
            [0u32.into(), 1u32.into(), 2u32.into()]
        );
        assert_eq!(
            mesh.face(draco_core::geometry_indices::FaceIndex(2)),
            [0u32.into(), 2u32.into(), 3u32.into()]
        );
    }

    #[test]
    fn test_read_mesh_parses_normals_and_colors() {
        let file = NamedTempFile::new().unwrap();
        let ply = r#"ply
format ascii 1.0
element vertex 2
property float x
property float y
property float z
property float nx
property float ny
property float nz
property uchar red
property uchar green
property uchar blue
property uchar alpha
end_header
0 0 0 0 0 1 10 20 30 40
1 0 0 0 1 0 50 60 70 80
"#;

        std::fs::write(file.path(), ply).unwrap();

        let mut reader = PlyReader::open(file.path()).unwrap();
        let mesh = reader.read_mesh().unwrap();

        assert_eq!(mesh.num_points(), 2);
        assert_eq!(mesh.num_faces(), 0);
        assert_eq!(mesh.num_attributes(), 3);

        let normal_att = mesh.named_attribute(GeometryAttributeType::Normal).unwrap();
        assert_eq!(normal_att.data_type(), DataType::Float32);
        assert_eq!(normal_att.num_components(), 3);
        assert!(!normal_att.normalized());

        let normal_data = normal_att.buffer().data();
        let first_normal = [
            f32::from_le_bytes(normal_data[0..4].try_into().unwrap()),
            f32::from_le_bytes(normal_data[4..8].try_into().unwrap()),
            f32::from_le_bytes(normal_data[8..12].try_into().unwrap()),
        ];
        assert_eq!(first_normal, [0.0, 0.0, 1.0]);

        let color_att = mesh.named_attribute(GeometryAttributeType::Color).unwrap();
        assert_eq!(color_att.data_type(), DataType::Uint8);
        assert_eq!(color_att.num_components(), 4);
        assert!(color_att.normalized());
        assert_eq!(color_att.buffer().data(), &[10, 20, 30, 40, 50, 60, 70, 80]);
    }

    #[test]
    fn test_read_mesh_preserves_int32_positions() {
        let file = NamedTempFile::new().unwrap();
        let ply = r#"ply
format ascii 1.0
element vertex 2
property int x
property int y
property int z
end_header
1 2 3
4 5 6
"#;

        std::fs::write(file.path(), ply).unwrap();

        let mut reader = PlyReader::open(file.path()).unwrap();
        let mesh = reader.read_mesh().unwrap();

        let position_att = mesh
            .named_attribute(GeometryAttributeType::Position)
            .unwrap();
        assert_eq!(position_att.data_type(), DataType::Int32);
        assert_eq!(position_att.num_components(), 3);
        assert!(!position_att.normalized());

        let position_data = position_att.buffer().data();
        let first_position = [
            i32::from_le_bytes(position_data[0..4].try_into().unwrap()),
            i32::from_le_bytes(position_data[4..8].try_into().unwrap()),
            i32::from_le_bytes(position_data[8..12].try_into().unwrap()),
        ];
        assert_eq!(first_position, [1, 2, 3]);
    }

    #[test]
    fn test_read_mesh_ignores_non_float_normals() {
        let file = NamedTempFile::new().unwrap();
        let ply = r#"ply
format ascii 1.0
element vertex 1
property float x
property float y
property float z
property int nx
property int ny
property int nz
end_header
0 0 0 0 0 1
"#;

        std::fs::write(file.path(), ply).unwrap();

        let mut reader = PlyReader::open(file.path()).unwrap();
        let mesh = reader.read_mesh().unwrap();

        assert_eq!(mesh.named_attribute_id(GeometryAttributeType::Normal), -1);
    }

    #[test]
    fn test_read_mesh_rejects_non_uint8_colors() {
        let file = NamedTempFile::new().unwrap();
        let ply = r#"ply
format ascii 1.0
element vertex 1
property float x
property float y
property float z
property int red
property int green
property int blue
end_header
0 0 0 1 2 3
"#;

        std::fs::write(file.path(), ply).unwrap();

        let mut reader = PlyReader::open(file.path()).unwrap();
        let error = reader.read_mesh().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("Color properties must be uint8"));
    }

    #[test]
    fn test_read_binary_little_endian_mesh() {
        let file = NamedTempFile::new().unwrap();
        let mut ply = Vec::new();
        ply.extend_from_slice(
            br#"ply
format binary_little_endian 1.0
element vertex 4
property float x
property float y
property float z
element face 2
property list uchar int vertex_indices
end_header
"#,
        );

        for vertex in [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ] {
            for component in vertex {
                ply.extend_from_slice(&component.to_le_bytes());
            }
        }

        ply.push(3);
        for index in [0i32, 1, 2] {
            ply.extend_from_slice(&index.to_le_bytes());
        }

        ply.push(4);
        for index in [0i32, 1, 2, 3] {
            ply.extend_from_slice(&index.to_le_bytes());
        }

        std::fs::write(file.path(), ply).unwrap();

        let mut reader = PlyReader::open(file.path()).unwrap();
        let mesh = reader.read_mesh().unwrap();

        assert_eq!(mesh.num_points(), 4);
        assert_eq!(mesh.num_faces(), 3);
        assert_eq!(
            mesh.face(draco_core::geometry_indices::FaceIndex(0)),
            [0u32.into(), 1u32.into(), 2u32.into()]
        );
        assert_eq!(
            mesh.face(draco_core::geometry_indices::FaceIndex(1)),
            [0u32.into(), 1u32.into(), 2u32.into()]
        );
        assert_eq!(
            mesh.face(draco_core::geometry_indices::FaceIndex(2)),
            [0u32.into(), 2u32.into(), 3u32.into()]
        );
    }

    #[test]
    fn test_read_binary_little_endian_attributes_and_int_positions() {
        let file = NamedTempFile::new().unwrap();
        let mut ply = Vec::new();
        ply.extend_from_slice(
            br#"ply
format binary_little_endian 1.0
element vertex 2
property int x
property int y
property int z
property float nx
property float ny
property float nz
property uchar red
property uchar green
property uchar blue
property uchar alpha
end_header
"#,
        );

        for (position, normal, color) in [
            ([1i32, 2, 3], [0.0f32, 0.0, 1.0], [10u8, 20, 30, 40]),
            ([4i32, 5, 6], [0.0f32, 1.0, 0.0], [50u8, 60, 70, 80]),
        ] {
            for component in position {
                ply.extend_from_slice(&component.to_le_bytes());
            }
            for component in normal {
                ply.extend_from_slice(&component.to_le_bytes());
            }
            ply.extend_from_slice(&color);
        }

        std::fs::write(file.path(), ply).unwrap();

        let mut reader = PlyReader::open(file.path()).unwrap();
        let mesh = reader.read_mesh().unwrap();

        let position_att = mesh
            .named_attribute(GeometryAttributeType::Position)
            .unwrap();
        assert_eq!(position_att.data_type(), DataType::Int32);
        assert_eq!(position_att.num_components(), 3);

        let position_data = position_att.buffer().data();
        let first_position = [
            i32::from_le_bytes(position_data[0..4].try_into().unwrap()),
            i32::from_le_bytes(position_data[4..8].try_into().unwrap()),
            i32::from_le_bytes(position_data[8..12].try_into().unwrap()),
        ];
        assert_eq!(first_position, [1, 2, 3]);

        let normal_att = mesh.named_attribute(GeometryAttributeType::Normal).unwrap();
        assert_eq!(normal_att.data_type(), DataType::Float32);
        assert_eq!(normal_att.num_components(), 3);

        let normal_data = normal_att.buffer().data();
        let first_normal = [
            f32::from_le_bytes(normal_data[0..4].try_into().unwrap()),
            f32::from_le_bytes(normal_data[4..8].try_into().unwrap()),
            f32::from_le_bytes(normal_data[8..12].try_into().unwrap()),
        ];
        assert_eq!(first_normal, [0.0, 0.0, 1.0]);

        let color_att = mesh.named_attribute(GeometryAttributeType::Color).unwrap();
        assert_eq!(color_att.data_type(), DataType::Uint8);
        assert_eq!(color_att.num_components(), 4);
        assert!(color_att.normalized());
        assert_eq!(color_att.buffer().data(), &[10, 20, 30, 40, 50, 60, 70, 80]);
    }

    #[test]
    fn test_read_binary_big_endian_mesh() {
        let mut ply = Vec::new();
        ply.extend_from_slice(
            br#"ply
format binary_big_endian 1.0
element vertex 4
property float x
property float y
property float z
element face 1
property list uchar int vertex_indices
end_header
"#,
        );

        for vertex in [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ] {
            for component in vertex {
                ply.extend_from_slice(&component.to_be_bytes());
            }
        }

        ply.push(4);
        for index in [0i32, 1, 2, 3] {
            ply.extend_from_slice(&index.to_be_bytes());
        }

        let mesh = PlyReader::read_from_bytes(&ply).unwrap();
        assert_eq!(mesh.num_points(), 4);
        assert_eq!(mesh.num_faces(), 2);
        assert_eq!(
            mesh.face(draco_core::geometry_indices::FaceIndex(1)),
            [0u32.into(), 2u32.into(), 3u32.into()]
        );
    }
}
