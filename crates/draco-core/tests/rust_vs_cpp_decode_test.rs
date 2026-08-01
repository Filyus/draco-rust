//! Required Rust/C++ interop coverage for Rust-encoded Edgebreaker meshes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::PointIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::point_cloud::PointCloud;
use draco_core::point_cloud_decoder::PointCloudDecoder;
use draco_core::point_cloud_encoder::PointCloudEncoder;

const POSITION_TOLERANCE: f32 = 0.01;
const NORMAL_TOLERANCE: f32 = 0.02;
const TEX_COORD_TOLERANCE: f32 = 0.01;

const BUILD_HINT: &str = "C++ Draco tools are required for this test. Build them with: \
cmake -S . -B build -G \"Visual Studio 17 2022\" && \
cmake --build build --config Release --target draco_decoder draco_encoder";

#[derive(Debug, Clone)]
struct VertexRecord {
    position: [f32; 3],
    normal: [f32; 3],
    tex_coord: [f32; 2],
}

#[derive(Debug)]
struct ObjSummary {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    tex_coords: Vec<[f32; 2]>,
    faces: Vec<Vec<String>>,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates directory")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn cpp_tool_from_dir(build_dir: &Path, tool_name: &str) -> Option<PathBuf> {
    let direct = build_dir.join(tool_name);
    if direct.exists() {
        return Some(direct);
    }

    for config in ["Release", "Debug"] {
        let configured = build_dir.join(config).join(tool_name);
        if configured.exists() {
            return Some(configured);
        }
    }

    let nested = build_dir.join("src").join("draco");
    for config in ["Release", "Debug"] {
        let configured = nested.join(config).join(tool_name);
        if configured.exists() {
            return Some(configured);
        }
    }

    None
}

fn find_cpp_tool(env_var: &str, tool_name: &str) -> PathBuf {
    if let Ok(path) = std::env::var(env_var) {
        let path = PathBuf::from(path);
        assert!(
            path.exists(),
            "{} points to a missing {}: {}\n{}",
            env_var,
            tool_name,
            path.display(),
            BUILD_HINT
        );
        return path;
    }

    if let Ok(build_dir) = std::env::var("DRACO_CPP_BUILD_DIR") {
        if let Some(path) = cpp_tool_from_dir(Path::new(&build_dir), tool_name) {
            return path;
        }
    }

    let root = repo_root();
    let candidates = [
        root.join("build-original")
            .join("src")
            .join("draco")
            .join("Release")
            .join(tool_name),
        root.join("build")
            .join("src")
            .join("draco")
            .join("Release")
            .join(tool_name),
        root.join("build")
            .join("src")
            .join("draco")
            .join("Debug")
            .join(tool_name),
    ];

    candidates
        .into_iter()
        .find(|path| path.exists())
        .unwrap_or_else(|| panic!("Could not find required C++ tool {tool_name}.\n{BUILD_HINT}"))
}

fn parse_obj(obj_content: &str) -> ObjSummary {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut tex_coords = Vec::new();
    let mut faces = Vec::new();

    for line in obj_content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["v", x, y, z, ..] => positions.push([
                x.parse().expect("OBJ x position"),
                y.parse().expect("OBJ y position"),
                z.parse().expect("OBJ z position"),
            ]),
            ["vn", x, y, z, ..] => normals.push([
                x.parse().expect("OBJ x normal"),
                y.parse().expect("OBJ y normal"),
                z.parse().expect("OBJ z normal"),
            ]),
            ["vt", u, v, ..] => tex_coords.push([
                u.parse().expect("OBJ u tex coord"),
                v.parse().expect("OBJ v tex coord"),
            ]),
            ["f", indices @ ..] => {
                faces.push(indices.iter().map(|value| value.to_string()).collect())
            }
            _ => {}
        }
    }

    ObjSummary {
        positions,
        normals,
        tex_coords,
        faces,
    }
}

fn write_f32s(attribute: &mut PointAttribute, values: &[f32]) {
    for (i, value) in values.iter().enumerate() {
        attribute.buffer_mut().write(i * 4, &value.to_le_bytes());
    }
}

fn write_u8s(attribute: &mut PointAttribute, values: &[u8]) {
    for (i, value) in values.iter().enumerate() {
        attribute.buffer_mut().write(i, &[*value]);
    }
}

fn read_f32_tuple(attribute: &PointAttribute, point: PointIndex, components: usize) -> Vec<f32> {
    let value_index = attribute.mapped_index(point).0 as usize;
    let offset = value_index * attribute.byte_stride() as usize;
    let data = attribute.buffer().data();
    (0..components)
        .map(|component| {
            let start = offset + component * 4;
            f32::from_le_bytes(data[start..start + 4].try_into().expect("f32 bytes"))
        })
        .collect()
}

fn read_position(attribute: &PointAttribute, point: PointIndex) -> [f32; 3] {
    let values = read_f32_tuple(attribute, point, 3);
    [values[0], values[1], values[2]]
}

fn read_normal(attribute: &PointAttribute, point: PointIndex) -> [f32; 3] {
    let values = read_f32_tuple(attribute, point, 3);
    [values[0], values[1], values[2]]
}

fn read_tex_coord(attribute: &PointAttribute, point: PointIndex) -> [f32; 2] {
    let values = read_f32_tuple(attribute, point, 2);
    [values[0], values[1]]
}

fn close_vec3(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
    (a[0] - b[0]).abs() <= tolerance
        && (a[1] - b[1]).abs() <= tolerance
        && (a[2] - b[2]).abs() <= tolerance
}

fn close_vec2(a: [f32; 2], b: [f32; 2], tolerance: f32) -> bool {
    (a[0] - b[0]).abs() <= tolerance && (a[1] - b[1]).abs() <= tolerance
}

fn build_multi_attribute_mesh() -> (Mesh, Vec<VertexRecord>, usize) {
    let positions: Vec<f32> = vec![
        -1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0, 1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, -1.0, -1.0,
        1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0,
    ];
    let normals: Vec<f32> = vec![
        0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0,
        0.0, 0.0, -1.0, 0.0, 0.0, -1.0,
    ];
    let tex_coords: Vec<f32> = vec![
        0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.1, 0.2, 0.1, 0.8, 0.9, 0.8, 0.9, 0.2,
    ];
    let colors: Vec<u8> = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 255, 0, 255, 255, 0, 255,
        255, 255, 128, 64, 255, 255, 255, 128, 64, 255,
    ];
    let indices: Vec<u32> = vec![0, 1, 2, 2, 3, 0, 4, 5, 6, 6, 7, 4];

    let vertex_count = positions.len() / 3;
    let face_count = indices.len() / 3;
    let mut mesh = Mesh::new();

    let mut position_attribute = PointAttribute::new();
    position_attribute.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        vertex_count,
    );
    write_f32s(&mut position_attribute, &positions);
    mesh.add_attribute(position_attribute);

    let mut normal_attribute = PointAttribute::new();
    normal_attribute.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        vertex_count,
    );
    write_f32s(&mut normal_attribute, &normals);
    mesh.add_attribute(normal_attribute);

    let mut tex_coord_attribute = PointAttribute::new();
    tex_coord_attribute.init(
        GeometryAttributeType::TexCoord,
        2,
        DataType::Float32,
        false,
        vertex_count,
    );
    write_f32s(&mut tex_coord_attribute, &tex_coords);
    mesh.add_attribute(tex_coord_attribute);

    let mut color_attribute = PointAttribute::new();
    color_attribute.init(
        GeometryAttributeType::Color,
        4,
        DataType::Uint8,
        true,
        vertex_count,
    );
    write_u8s(&mut color_attribute, &colors);
    mesh.add_attribute(color_attribute);

    for triangle in indices.chunks_exact(3) {
        mesh.add_face([
            PointIndex(triangle[0]),
            PointIndex(triangle[1]),
            PointIndex(triangle[2]),
        ]);
    }

    let expected_vertices = (0..vertex_count)
        .map(|i| VertexRecord {
            position: [positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2]],
            normal: [normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]],
            tex_coord: [tex_coords[i * 2], tex_coords[i * 2 + 1]],
        })
        .collect();

    (mesh, expected_vertices, face_count)
}

fn build_point_cloud_with_attributes() -> PointCloud {
    let positions: Vec<f32> = vec![
        -1.0, -1.0, 0.0, 0.0, -1.0, 0.5, 1.0, -1.0, 0.0, -0.5, 0.0, 1.0, 0.5, 0.0, 1.0, -1.0, 1.0,
        0.0, 0.0, 1.0, 0.5, 1.0, 1.0, 0.0,
    ];
    let normals: Vec<f32> = vec![
        0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0,
        1.0, 0.0, 0.0, 1.0, 0.0, 0.0,
    ];
    let colors: Vec<u8> = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 255, 0, 255, 255, 0, 255,
        255, 255, 128, 64, 255, 255, 255, 128, 64, 255,
    ];
    let point_count = positions.len() / 3;
    let mut point_cloud = PointCloud::new();
    point_cloud.set_num_points(point_count);

    let mut position_attribute = PointAttribute::new();
    position_attribute.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        point_count,
    );
    write_f32s(&mut position_attribute, &positions);
    point_cloud.add_attribute(position_attribute);

    let mut normal_attribute = PointAttribute::new();
    normal_attribute.init(
        GeometryAttributeType::Normal,
        3,
        DataType::Float32,
        false,
        point_count,
    );
    write_f32s(&mut normal_attribute, &normals);
    point_cloud.add_attribute(normal_attribute);

    let mut color_attribute = PointAttribute::new();
    color_attribute.init(
        GeometryAttributeType::Color,
        4,
        DataType::Uint8,
        true,
        point_count,
    );
    write_u8s(&mut color_attribute, &colors);
    point_cloud.add_attribute(color_attribute);

    point_cloud
}

fn rust_decode_mesh_invariants(bytes: &[u8], expected_method: u8) {
    assert_eq!(&bytes[0..5], b"DRACO");
    assert_eq!(bytes[7], 1, "expected triangular mesh geometry type");
    assert_eq!(bytes[8], expected_method, "unexpected mesh encoding method");

    let mut decoder = MeshDecoder::new();
    let mut mesh = Mesh::new();
    let mut decode_buffer = DecoderBuffer::new(bytes);
    decoder
        .decode(&mut decode_buffer, &mut mesh)
        .expect("Rust decode of Rust mesh stream failed");

    assert!(mesh.num_points() > 0);
    assert!(mesh.num_faces() > 0);
    assert!(mesh.num_attributes() >= 4);
    assert!(mesh.named_attribute_id(GeometryAttributeType::Position) >= 0);
    assert!(mesh.named_attribute_id(GeometryAttributeType::Normal) >= 0);
    assert!(mesh.named_attribute_id(GeometryAttributeType::TexCoord) >= 0);
    assert!(mesh.named_attribute_id(GeometryAttributeType::Color) >= 0);
}

fn rust_decode_point_cloud_invariants(bytes: &[u8]) {
    assert_eq!(&bytes[0..5], b"DRACO");
    assert_eq!(bytes[7], 0, "expected point cloud geometry type");
    assert_eq!(bytes[8], 0, "expected sequential point cloud encoding");

    let mut decoder = PointCloudDecoder::new();
    let mut point_cloud = PointCloud::new();
    let mut decode_buffer = DecoderBuffer::new(bytes);
    decoder
        .decode(&mut decode_buffer, &mut point_cloud)
        .expect("Rust decode of Rust point-cloud stream failed");

    assert!(point_cloud.num_points() > 0);
    assert!(point_cloud.num_attributes() >= 3);
    assert!(point_cloud.named_attribute_id(GeometryAttributeType::Position) >= 0);
    assert!(point_cloud.named_attribute_id(GeometryAttributeType::Normal) >= 0);
    assert!(point_cloud.named_attribute_id(GeometryAttributeType::Color) >= 0);
}

/// What the C++ decoder actually wrote, read back out of its own output file.
#[derive(Debug)]
struct CppDecoded {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    num_faces: usize,
}

/// Runs the C++ decoder and returns what it produced.
///
/// The output file is parsed rather than merely weighed: a decoder that exits
/// zero and writes a header with no vertices satisfies "succeeded" and "not
/// empty" while proving nothing about the stream it was given.
fn run_cpp_decoder(
    decoder_exe: &Path,
    drc_path: &Path,
    out_path: &Path,
    context: &str,
) -> CppDecoded {
    let output = Command::new(decoder_exe)
        .arg("-i")
        .arg(drc_path)
        .arg("-o")
        .arg(out_path)
        .output()
        .expect("run C++ Draco decoder");

    assert!(
        output.status.success(),
        "C++ decoder failed for {context}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let bytes = fs::read(out_path)
        .unwrap_or_else(|err| panic!("{context}: C++ decoder output missing: {err}"));
    parse_binary_ply(&bytes, context)
}

/// Parses the little-endian binary PLY `draco_decoder` writes.
///
/// Deliberately narrow: it understands the fixed-width scalar properties this
/// test's fixtures produce and the two list properties a Draco-decoded face
/// carries, and panics on anything else rather than guessing.
fn parse_binary_ply(bytes: &[u8], context: &str) -> CppDecoded {
    const MARKER: &[u8] = b"end_header\n";
    let header_end = bytes
        .windows(MARKER.len())
        .position(|window| window == MARKER)
        .unwrap_or_else(|| panic!("{context}: no PLY header terminator"))
        + MARKER.len();
    let header = std::str::from_utf8(&bytes[..header_end])
        .unwrap_or_else(|err| panic!("{context}: PLY header is not UTF-8: {err}"));

    let scalar_size = |kind: &str| -> usize {
        match kind {
            "char" | "uchar" | "int8" | "uint8" => 1,
            "short" | "ushort" | "int16" | "uint16" => 2,
            "int" | "uint" | "int32" | "uint32" | "float" | "float32" => 4,
            "double" | "float64" => 8,
            other => panic!("{context}: unsupported PLY property type {other}"),
        }
    };

    assert!(
        header.contains("format binary_little_endian"),
        "{context}: expected a binary little-endian PLY"
    );

    // Scalar vertex properties, in file order, as (name, offset, size).
    let mut vertex_properties: Vec<(String, usize, usize)> = Vec::new();
    let mut vertex_stride = 0usize;
    let mut num_vertices = 0usize;
    let mut num_faces = 0usize;
    let mut face_lists: Vec<(usize, usize)> = Vec::new(); // (count size, entry size)
    let mut element = "";

    for line in header.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["element", name, count] => {
                element = if *name == "vertex" {
                    "vertex"
                } else if *name == "face" {
                    "face"
                } else {
                    "other"
                };
                let count = count.parse().expect("PLY element count");
                match element {
                    "vertex" => num_vertices = count,
                    "face" => num_faces = count,
                    _ => {}
                }
            }
            ["property", "list", count_type, entry_type, _] if element == "face" => {
                face_lists.push((scalar_size(count_type), scalar_size(entry_type)));
            }
            ["property", kind, name] if element == "vertex" => {
                let size = scalar_size(kind);
                vertex_properties.push((name.to_string(), vertex_stride, size));
                vertex_stride += size;
            }
            _ => {}
        }
    }

    let float_at = |record: &[u8], name: &str| -> Option<f32> {
        let (_, offset, size) = vertex_properties.iter().find(|(n, _, _)| n == name)?;
        assert_eq!(*size, 4, "{context}: PLY property {name} is not a float");
        Some(f32::from_le_bytes(
            record[*offset..*offset + 4].try_into().unwrap(),
        ))
    };

    let body = &bytes[header_end..];
    assert!(
        body.len() >= num_vertices * vertex_stride,
        "{context}: PLY body is shorter than its header promises"
    );

    let mut positions = Vec::with_capacity(num_vertices);
    let mut normals = Vec::with_capacity(num_vertices);
    for i in 0..num_vertices {
        let record = &body[i * vertex_stride..(i + 1) * vertex_stride];
        let read = |names: [&str; 3]| names.map(|name| float_at(record, name));
        if let [Some(x), Some(y), Some(z)] = read(["x", "y", "z"]) {
            positions.push([x, y, z]);
        }
        if let [Some(x), Some(y), Some(z)] = read(["nx", "ny", "nz"]) {
            normals.push([x, y, z]);
        }
    }

    // Walk the face records too, so a truncated face section is caught rather
    // than trusted from the header count alone.
    let mut cursor = num_vertices * vertex_stride;
    for face in 0..num_faces {
        for &(count_size, entry_size) in &face_lists {
            assert!(
                cursor + count_size <= body.len(),
                "{context}: PLY face {face} is truncated"
            );
            let count = body[cursor..cursor + count_size]
                .iter()
                .rev()
                .fold(0usize, |acc, &b| (acc << 8) | b as usize);
            cursor += count_size + count * entry_size;
            assert!(
                cursor <= body.len(),
                "{context}: PLY face {face} is truncated"
            );
        }
    }

    CppDecoded {
        positions,
        normals,
        num_faces,
    }
}

fn decoded_vertex_records(mesh: &Mesh) -> Vec<VertexRecord> {
    let position_id = mesh.named_attribute_id(GeometryAttributeType::Position);
    let normal_id = mesh.named_attribute_id(GeometryAttributeType::Normal);
    let tex_coord_id = mesh.named_attribute_id(GeometryAttributeType::TexCoord);
    assert!(position_id >= 0, "Rust decode missing POSITION attribute");
    assert!(normal_id >= 0, "Rust decode missing NORMAL attribute");
    assert!(tex_coord_id >= 0, "Rust decode missing TEX_COORD attribute");

    let position_attribute = mesh.attribute(position_id);
    let normal_attribute = mesh.attribute(normal_id);
    let tex_coord_attribute = mesh.attribute(tex_coord_id);

    (0..mesh.num_points())
        .map(|point| {
            let point = PointIndex(point as u32);
            VertexRecord {
                position: read_position(position_attribute, point),
                normal: read_normal(normal_attribute, point),
                tex_coord: read_tex_coord(tex_coord_attribute, point),
            }
        })
        .collect()
}

fn assert_vertex_records_match(expected: &[VertexRecord], actual: &[VertexRecord]) {
    assert_eq!(actual.len(), expected.len(), "decoded point count mismatch");
    let mut matched = vec![false; actual.len()];

    for expected_vertex in expected {
        let Some((actual_index, _)) = actual.iter().enumerate().find(|(index, actual_vertex)| {
            !matched[*index]
                && close_vec3(
                    expected_vertex.position,
                    actual_vertex.position,
                    POSITION_TOLERANCE,
                )
                && close_vec3(
                    expected_vertex.normal,
                    actual_vertex.normal,
                    NORMAL_TOLERANCE,
                )
                && close_vec2(
                    expected_vertex.tex_coord,
                    actual_vertex.tex_coord,
                    TEX_COORD_TOLERANCE,
                )
        }) else {
            panic!(
                "No decoded Rust vertex matched expected vertex {:?}\nActual vertices: {:?}",
                expected_vertex, actual
            );
        };
        matched[actual_index] = true;
    }
}

fn assert_position_sets_match(expected: &[[f32; 3]], actual: &[[f32; 3]], context: &str) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{context} position count mismatch"
    );
    let mut matched = vec![false; actual.len()];

    for &expected_position in expected {
        let Some((actual_index, _)) =
            actual.iter().enumerate().find(|(index, &actual_position)| {
                !matched[*index]
                    && close_vec3(expected_position, actual_position, POSITION_TOLERANCE)
            })
        else {
            panic!(
                "{context}: no decoded position matched expected {:?}\nActual positions: {:?}",
                expected_position, actual
            );
        };
        matched[actual_index] = true;
    }
}

fn assert_vec3_sets_match(
    expected: &[[f32; 3]],
    actual: &[[f32; 3]],
    tolerance: f32,
    context: &str,
) {
    assert_eq!(actual.len(), expected.len(), "{context} count mismatch");
    let mut matched = vec![false; actual.len()];

    for &expected_value in expected {
        let Some((actual_index, _)) = actual.iter().enumerate().find(|(index, &actual_value)| {
            !matched[*index] && close_vec3(expected_value, actual_value, tolerance)
        }) else {
            panic!(
                "{context}: no decoded value matched expected {:?}\nActual values: {:?}",
                expected_value, actual
            );
        };
        matched[actual_index] = true;
    }
}

fn assert_vec2_sets_match(
    expected: &[[f32; 2]],
    actual: &[[f32; 2]],
    tolerance: f32,
    context: &str,
) {
    assert_eq!(actual.len(), expected.len(), "{context} count mismatch");
    let mut matched = vec![false; actual.len()];

    for &expected_value in expected {
        let Some((actual_index, _)) = actual.iter().enumerate().find(|(index, &actual_value)| {
            !matched[*index] && close_vec2(expected_value, actual_value, tolerance)
        }) else {
            panic!(
                "{context}: no decoded value matched expected {:?}\nActual values: {:?}",
                expected_value, actual
            );
        };
        matched[actual_index] = true;
    }
}

#[test]
fn rust_encode_cpp_decode_small_matrix() {
    let decoder_exe = find_cpp_tool("DRACO_CPP_DECODER", "draco_decoder.exe");
    let tmp = std::env::temp_dir().join("draco_rust_encode_cpp_decode_small_matrix");
    fs::create_dir_all(&tmp).expect("create temp dir");

    for (name, encoding_method, encoding_speed) in [
        ("mesh_sequential_pos_norm_uv_color", 0, 10),
        ("mesh_edgebreaker_pos_norm_uv_color", 1, 5),
    ] {
        let (mesh, expected_vertices, expected_face_count) = build_multi_attribute_mesh();
        let position_id = mesh.named_attribute_id(GeometryAttributeType::Position);
        let normal_id = mesh.named_attribute_id(GeometryAttributeType::Normal);
        let tex_coord_id = mesh.named_attribute_id(GeometryAttributeType::TexCoord);

        let mut options = EncoderOptions::default();
        options.set_global_int("encoding_method", encoding_method);
        options.set_global_int("encoding_speed", encoding_speed);
        options.set_global_int("decoding_speed", encoding_speed);
        options.set_global_int("split_mesh_on_seams", 0);
        options.set_attribute_int(position_id, "quantization_bits", 14);
        options.set_attribute_int(normal_id, "quantization_bits", 10);
        options.set_attribute_int(tex_coord_id, "quantization_bits", 12);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh);
        let mut encoded = EncoderBuffer::new();
        encoder
            .encode(&options, &mut encoded)
            .unwrap_or_else(|err| panic!("Rust mesh encode failed for {name}: {err:?}"));
        let draco_bytes = encoded.data().to_vec();

        rust_decode_mesh_invariants(&draco_bytes, encoding_method as u8);

        let drc_path = tmp.join(format!("{name}.drc"));
        let ply_path = tmp.join(format!("{name}.ply"));
        fs::write(&drc_path, &draco_bytes).expect("write Rust mesh DRC");

        let decoded = run_cpp_decoder(&decoder_exe, &drc_path, &ply_path, name);
        assert_eq!(
            decoded.num_faces, expected_face_count,
            "{name}: C++ decoded a different number of faces"
        );
        let expected_positions: Vec<[f32; 3]> =
            expected_vertices.iter().map(|v| v.position).collect();
        let expected_normals: Vec<[f32; 3]> = expected_vertices.iter().map(|v| v.normal).collect();
        assert_position_sets_match(&expected_positions, &decoded.positions, name);
        assert_vec3_sets_match(
            &expected_normals,
            &decoded.normals,
            NORMAL_TOLERANCE,
            &format!("{name} normals"),
        );
    }

    for (name, prediction_scheme) in [
        ("point_cloud_sequential_pos_norm_color", None),
        (
            "point_cloud_sequential_no_prediction_pos_norm_color",
            Some(-2),
        ),
    ] {
        let point_cloud = build_point_cloud_with_attributes();
        let position_id = point_cloud.named_attribute_id(GeometryAttributeType::Position);
        let normal_id = point_cloud.named_attribute_id(GeometryAttributeType::Normal);

        // Read the inputs back out of the fixture, so the comparison below is
        // against what was encoded rather than a second copy of the literals.
        let expected_positions: Vec<[f32; 3]> = (0..point_cloud.num_points())
            .map(|point| {
                read_position(point_cloud.attribute(position_id), PointIndex(point as u32))
            })
            .collect();
        let expected_normals: Vec<[f32; 3]> = (0..point_cloud.num_points())
            .map(|point| read_normal(point_cloud.attribute(normal_id), PointIndex(point as u32)))
            .collect();

        let mut options = EncoderOptions::default();
        options.set_global_int("encoding_method", 0);
        options.set_global_int("encoding_speed", 5);
        options.set_global_int("decoding_speed", 5);
        options.set_version(2, 3);
        options.set_attribute_int(position_id, "quantization_bits", 14);
        options.set_attribute_int(normal_id, "quantization_bits", 10);
        if let Some(prediction_scheme) = prediction_scheme {
            options.set_prediction_scheme(prediction_scheme);
        }

        let mut encoder = PointCloudEncoder::new();
        encoder.set_point_cloud(point_cloud);
        let mut encoded = EncoderBuffer::new();
        encoder
            .encode(&options, &mut encoded)
            .unwrap_or_else(|err| panic!("Rust point-cloud encode failed for {name}: {err:?}"));
        let draco_bytes = encoded.data().to_vec();

        rust_decode_point_cloud_invariants(&draco_bytes);

        let drc_path = tmp.join(format!("{name}.drc"));
        let ply_path = tmp.join(format!("{name}.ply"));
        fs::write(&drc_path, &draco_bytes).expect("write Rust point-cloud DRC");

        let decoded = run_cpp_decoder(&decoder_exe, &drc_path, &ply_path, name);
        assert_eq!(decoded.num_faces, 0, "{name}: a point cloud has no faces");
        assert_position_sets_match(&expected_positions, &decoded.positions, name);
        assert_vec3_sets_match(
            &expected_normals,
            &decoded.normals,
            NORMAL_TOLERANCE,
            &format!("{name} normals"),
        );
    }
}

/// A grid with holes punched in it, so EdgeBreaker emits topology splits.
///
/// Returns the mesh and the positions it was built from. A plain grid is a disc
/// and produces no split events at all, which is why the split layouts need a
/// mesh of their own to be tested against anything.
#[cfg(feature = "legacy_bitstream_encode")]
fn annulus_mesh(n: usize) -> (Mesh, Vec<[f32; 3]>) {
    let holes = [(n / 4, n / 4), (n / 2, n / 2), (3 * n / 4 - 1, n / 4)];

    let mut positions = Vec::with_capacity(n * n);
    let mut attribute = PointAttribute::new();
    attribute.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        n * n,
    );
    for y in 0..n {
        for x in 0..n {
            let value = [x as f32, y as f32, ((x + y) % 5) as f32];
            let offset = (y * n + x) * 12;
            for (component, part) in value.iter().enumerate() {
                attribute
                    .buffer_mut()
                    .write(offset + component * 4, &part.to_le_bytes());
            }
            positions.push(value);
        }
    }

    let mut mesh = Mesh::new();
    mesh.set_num_points(n * n);
    mesh.add_attribute(attribute);

    let mut faces = Vec::new();
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            if holes.contains(&(x, y)) {
                continue;
            }
            let p0 = (y * n + x) as u32;
            let p1 = (y * n + x + 1) as u32;
            let p2 = ((y + 1) * n + x) as u32;
            let p3 = ((y + 1) * n + x + 1) as u32;
            faces.push([p0, p1, p2]);
            faces.push([p1, p3, p2]);
        }
    }
    mesh.set_num_faces(faces.len());
    for (id, face) in faces.iter().enumerate() {
        mesh.set_face_from_indices(id, *face);
    }

    (mesh, positions)
}

/// C++ Draco reads back the legacy streams this crate writes.
///
/// The round-trip matrix in `version_roundtrip_test` proves the encoder and
/// this crate's decoder agree. It cannot prove either agrees with Draco, and
/// agreeing with Draco is the entire purpose of writing an old version: the
/// only reason to emit 1.1 is that something else will read it. Two defects
/// survived until this test existed, and neither was visible from Rust:
///
/// - split events below 1.2 are two absolute `u32` ids and an edge byte, and
///   the encoder wrote the 1.2 delta/varint form at every version;
/// - speed 0 selects the constrained multi-parallelogram prediction scheme,
///   which postdates 1.1, so a 1.1 stream written at speed 0 was readable by no
///   released decoder at all.
///
/// The speeds are here for the second one, and the holes for the first. 1.2 is
/// beside 1.1 so a failure names the boundary rather than the feature.
///
/// Setting `DRACO_CPP_DECODER` to a Draco 0.9.1 build adds the one reader
/// contemporary with bitstream 1.1, which is the only thing that can see the
/// rANS probability-table difference; a modern decoder accepts both forms. The
/// format rule itself is pinned in CI by
/// `rans_symbol_encoder::tests::the_zero_run_token_is_written_only_from_1_2`.
#[test]
#[cfg(feature = "legacy_bitstream_encode")]
fn cpp_decodes_the_legacy_streams_this_crate_writes() {
    let decoder_exe = find_cpp_tool("DRACO_CPP_DECODER", "draco_decoder.exe");
    let tmp = std::env::temp_dir().join("draco_cpp_decodes_legacy_streams");
    fs::create_dir_all(&tmp).expect("create temp dir");

    for (major, minor) in [(1u8, 1u8), (1, 2)] {
        for speed in [0, 5, 10] {
            let (mesh, expected_positions) = annulus_mesh(17);
            let expected_face_count = mesh.num_faces();
            let position_id = mesh.named_attribute_id(GeometryAttributeType::Position);

            let mut options = EncoderOptions::default();
            options.set_version(major, minor);
            options.set_global_int("encoding_method", 1); // EdgeBreaker
            options.set_global_int("encoding_speed", speed);
            options.set_global_int("decoding_speed", speed);
            options.set_attribute_int(position_id, "quantization_bits", 14);

            let mut encoder = MeshEncoder::new();
            encoder.set_mesh(mesh);
            let mut encoded = EncoderBuffer::new();
            encoder
                .encode(&options, &mut encoded)
                .unwrap_or_else(|err| {
                    panic!("v{major}.{minor} at speed {speed}: Rust encode failed: {err:?}")
                });

            let name = format!("legacy_v{major}_{minor}_s{speed}");
            let drc_path = tmp.join(format!("{name}.drc"));
            let ply_path = tmp.join(format!("{name}.ply"));
            fs::write(&drc_path, encoded.data()).expect("write Rust DRC");

            let decoded = run_cpp_decoder(&decoder_exe, &drc_path, &ply_path, &name);
            assert_eq!(
                decoded.num_faces, expected_face_count,
                "{name}: C++ decoded a different number of faces"
            );
            assert_position_sets_match(&expected_positions, &decoded.positions, &name);
        }
    }
}

#[test]
fn compare_rust_vs_cpp_decode() {
    let decoder_exe = find_cpp_tool("DRACO_CPP_DECODER", "draco_decoder.exe");
    let encoder_exe = find_cpp_tool("DRACO_CPP_ENCODER", "draco_encoder.exe");
    assert!(
        encoder_exe.exists(),
        "Required C++ encoder is missing: {}\n{}",
        encoder_exe.display(),
        BUILD_HINT
    );

    let (mesh, expected_vertices, expected_face_count) = build_multi_attribute_mesh();

    let position_id = mesh.named_attribute_id(GeometryAttributeType::Position);
    let normal_id = mesh.named_attribute_id(GeometryAttributeType::Normal);
    let tex_coord_id = mesh.named_attribute_id(GeometryAttributeType::TexCoord);

    let mut options = EncoderOptions::default();
    options.set_global_int("encoding_method", 1);
    options.set_global_int("encoding_speed", 5);
    options.set_global_int("decoding_speed", 5);
    options.set_global_int("split_mesh_on_seams", 0);
    options.set_attribute_int(position_id, "quantization_bits", 14);
    options.set_attribute_int(normal_id, "quantization_bits", 10);
    options.set_attribute_int(tex_coord_id, "quantization_bits", 12);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut encoded = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoded)
        .expect("Rust Edgebreaker encode failed");
    let draco_bytes = encoded.data().to_vec();

    assert_eq!(&draco_bytes[0..5], b"DRACO");
    assert_eq!(draco_bytes[7], 1, "expected triangular mesh geometry type");
    assert_eq!(draco_bytes[8], 1, "expected Rust Edgebreaker encoding");

    let mut rust_decoder = MeshDecoder::new();
    let mut rust_mesh = Mesh::new();
    let mut decode_buffer = DecoderBuffer::new(&draco_bytes);
    rust_decoder
        .decode(&mut decode_buffer, &mut rust_mesh)
        .expect("Rust decode of Rust Edgebreaker stream failed");

    assert_eq!(
        rust_mesh.num_faces(),
        expected_face_count,
        "Rust decoded face count mismatch"
    );
    let rust_vertices = decoded_vertex_records(&rust_mesh);

    let tmp = std::env::temp_dir().join("draco_edgebreaker_multi_attribute_cpp_required");
    fs::create_dir_all(&tmp).expect("create temp dir");
    let drc_path = tmp.join("multi_attr_edgebreaker.drc");
    let obj_path = tmp.join("multi_attr_edgebreaker.obj");
    fs::write(&drc_path, &draco_bytes).expect("write Rust Edgebreaker DRC");

    let output = Command::new(&decoder_exe)
        .arg("-i")
        .arg(&drc_path)
        .arg("-o")
        .arg(&obj_path)
        .output()
        .expect("run C++ Draco decoder");

    assert!(
        output.status.success(),
        "C++ decoder failed for Rust Edgebreaker multi-attribute stream\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let obj_content = fs::read_to_string(&obj_path).expect("read C++ decoded OBJ");
    let obj = parse_obj(&obj_content);
    let expected_positions: Vec<[f32; 3]> = expected_vertices
        .iter()
        .map(|vertex| vertex.position)
        .collect();
    let expected_normals: Vec<[f32; 3]> = expected_vertices
        .iter()
        .map(|vertex| vertex.normal)
        .collect();
    let expected_tex_coords: Vec<[f32; 2]> = expected_vertices
        .iter()
        .map(|vertex| vertex.tex_coord)
        .collect();
    let rust_positions: Vec<[f32; 3]> =
        rust_vertices.iter().map(|vertex| vertex.position).collect();
    let rust_normals: Vec<[f32; 3]> = rust_vertices.iter().map(|vertex| vertex.normal).collect();
    let rust_tex_coords: Vec<[f32; 2]> = rust_vertices
        .iter()
        .map(|vertex| vertex.tex_coord)
        .collect();

    assert_position_sets_match(&rust_positions, &obj.positions, "C++ vs Rust");
    assert_position_sets_match(&expected_positions, &obj.positions, "C++ vs expected");
    assert_vec3_sets_match(
        &rust_normals,
        &obj.normals,
        NORMAL_TOLERANCE,
        "C++ vs Rust normals",
    );
    assert_vec3_sets_match(
        &expected_normals,
        &obj.normals,
        NORMAL_TOLERANCE,
        "C++ vs expected normals",
    );
    assert_vec2_sets_match(
        &rust_tex_coords,
        &obj.tex_coords,
        TEX_COORD_TOLERANCE,
        "C++ vs Rust tex coords",
    );
    assert_vec2_sets_match(
        &expected_tex_coords,
        &obj.tex_coords,
        TEX_COORD_TOLERANCE,
        "C++ vs expected tex coords",
    );
    assert_eq!(
        obj.faces.len(),
        expected_face_count,
        "C++ decoded OBJ face count mismatch"
    );

    assert_vertex_records_match(&expected_vertices, &rust_vertices);
}
