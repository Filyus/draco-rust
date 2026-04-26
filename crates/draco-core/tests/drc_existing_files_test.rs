use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use draco_core::compression_config::EncodedGeometryType;
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
use draco_core::status::DracoError;

fn repo_testdata_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = <repo>/crates/draco-core
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata")
}

fn collect_drc_files_recursive(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(v) => v,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("drc"))
            {
                out.push(path);
            }
        }
    }

    out
}

fn read_file_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

fn parse_header(bytes: &[u8]) -> (u8, u8, EncodedGeometryType, u8) {
    // Draco header (common):
    // 0..5: "DRACO", 5: major, 6: minor, 7: geometry_type, 8: encoding method
    assert!(bytes.len() >= 9, "file too small for drc header");
    assert_eq!(&bytes[0..5], b"DRACO", "invalid magic");
    let major = bytes[5];
    let minor = bytes[6];
    let geometry_type = match bytes[7] {
        0 => EncodedGeometryType::PointCloud,
        1 => EncodedGeometryType::TriangularMesh,
        other => panic!("unexpected geometry type in header: {other}"),
    };
    let method = bytes[8];
    (major, minor, geometry_type, method)
}

fn supports_mesh_bitstream(major: u8, _minor: u8) -> bool {
    // Rust MeshDecoder supports the modern v2.2+ layout and the v2.0/v2.1
    // legacy mesh layout used by Draco 1.0.0/1.1.0 test fixtures.
    major >= 2
}

fn supports_point_cloud_bitstream(major: u8, minor: u8, method: u8) -> bool {
    // Current PointCloudDecoder supports:
    // - v2.0+ sequential (method=0), covering the Draco 1.0.0+ policy floor
    // - v2.3 KD-tree (method=1)
    // - our v1.3 sequential format (method=0)
    (major == 2 && method == 0)
        || (major == 2 && minor == 3 && method == 1)
        || (major == 1 && minor == 3 && method == 0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeometryKind {
    Mesh,
    PointCloud,
}

impl From<EncodedGeometryType> for GeometryKind {
    fn from(value: EncodedGeometryType) -> Self {
        match value {
            EncodedGeometryType::TriangularMesh => Self::Mesh,
            EncodedGeometryType::PointCloud => Self::PointCloud,
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SkipReason {
    UnsupportedBitstream,
    UnsupportedTraversal,
}

#[derive(Debug, Eq, PartialEq)]
struct SkippedFixture {
    path: String,
    major: u8,
    minor: u8,
    geometry: GeometryKind,
    method: u8,
    reason: SkipReason,
}

fn skipped(
    path: &str,
    major: u8,
    minor: u8,
    geometry: GeometryKind,
    method: u8,
    reason: SkipReason,
) -> SkippedFixture {
    SkippedFixture {
        path: path.to_string(),
        major,
        minor,
        geometry,
        method,
        reason,
    }
}

fn relative_testdata_path(path: &Path) -> String {
    path.strip_prefix(repo_testdata_dir())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn skipped_fixture_for_current_decoder(path: &Path, bytes: &[u8]) -> Option<SkippedFixture> {
    let (major, minor, geometry_type, method) = parse_header(bytes);
    let path = relative_testdata_path(path);
    let geometry = GeometryKind::from(geometry_type);

    match geometry_type {
        EncodedGeometryType::TriangularMesh => {
            if !supports_mesh_bitstream(major, minor) {
                return Some(skipped(
                    &path,
                    major,
                    minor,
                    geometry,
                    method,
                    SkipReason::UnsupportedBitstream,
                ));
            }

            let mut buffer = DecoderBuffer::new(bytes);
            let mut mesh = Mesh::new();
            let mut decoder = MeshDecoder::new();
            if let Err(DracoError::DracoError(msg)) = decoder.decode(&mut buffer, &mut mesh) {
                if msg.starts_with("Unsupported Edgebreaker traversal decoder type") {
                    return Some(skipped(
                        &path,
                        major,
                        minor,
                        geometry,
                        method,
                        SkipReason::UnsupportedTraversal,
                    ));
                }
            }
        }
        EncodedGeometryType::PointCloud => {
            if !supports_point_cloud_bitstream(major, minor, method) {
                return Some(skipped(
                    &path,
                    major,
                    minor,
                    geometry,
                    method,
                    SkipReason::UnsupportedBitstream,
                ));
            }
        }
        _ => unreachable!(),
    }

    None
}

fn decode_drc(bytes: &[u8]) -> (EncodedGeometryType, Option<Mesh>, Option<PointCloud>) {
    let (_major, _minor, geometry_type, _method) = parse_header(bytes);

    match geometry_type {
        EncodedGeometryType::TriangularMesh => {
            let mut buffer = DecoderBuffer::new(bytes);
            let mut mesh = Mesh::new();
            let mut decoder = MeshDecoder::new();
            let status = decoder.decode(&mut buffer, &mut mesh);
            assert!(status.is_ok(), "mesh decode failed: {:?}", status.err());
            (geometry_type, Some(mesh), None)
        }
        EncodedGeometryType::PointCloud => {
            let mut buffer = DecoderBuffer::new(bytes);
            let mut pc = PointCloud::new();
            let mut decoder = PointCloudDecoder::new();
            let status = decoder.decode(&mut buffer, &mut pc);
            assert!(
                status.is_ok(),
                "point cloud decode failed: {:?}",
                status.err()
            );
            (geometry_type, None, Some(pc))
        }
        _ => unreachable!(),
    }
}

#[derive(Debug, Clone)]
struct LegacyCornerRecord {
    position: [f32; 3],
    tex_coord: [f32; 2],
    normal: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
enum LegacyDecoderVersion {
    V1_0_0,
    V1_1_0,
}

impl LegacyDecoderVersion {
    fn env_var(self) -> &'static str {
        match self {
            Self::V1_0_0 => "DRACO_LEGACY_DECODER_1_0_0",
            Self::V1_1_0 => "DRACO_LEGACY_DECODER_1_1_0",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::V1_0_0 => "1.0.0",
            Self::V1_1_0 => "1.1.0",
        }
    }
}

fn legacy_decoder_path(version: LegacyDecoderVersion) -> Option<PathBuf> {
    let env_var = version.env_var();

    let path = std::env::var_os(env_var).map(PathBuf::from)?;
    if path.exists() {
        Some(path)
    } else {
        eprintln!(
            "Skipping legacy decoder comparison for {}: {env_var} points to missing path {}",
            version.label(),
            path.display()
        );
        None
    }
}

fn parse_obj_triplet_index(value: &str, component: usize) -> usize {
    let raw = value
        .split('/')
        .nth(component)
        .unwrap_or_else(|| panic!("invalid OBJ face element: {value}"));
    assert!(!raw.is_empty(), "missing OBJ face component in {value}");
    raw.parse::<usize>()
        .unwrap_or_else(|e| panic!("invalid OBJ face index {raw}: {e}"))
        - 1
}

fn parse_cpp_obj_corner_records(obj: &str) -> Vec<LegacyCornerRecord> {
    let mut positions = Vec::new();
    let mut tex_coords = Vec::new();
    let mut normals = Vec::new();
    let mut records = Vec::new();

    for line in obj.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["v", x, y, z, ..] => positions.push([
                x.parse().expect("OBJ x position"),
                y.parse().expect("OBJ y position"),
                z.parse().expect("OBJ z position"),
            ]),
            ["vt", u, v, ..] => tex_coords.push([
                u.parse().expect("OBJ u tex coord"),
                v.parse().expect("OBJ v tex coord"),
            ]),
            ["vn", x, y, z, ..] => normals.push([
                x.parse().expect("OBJ x normal"),
                y.parse().expect("OBJ y normal"),
                z.parse().expect("OBJ z normal"),
            ]),
            ["f", corners @ ..] => {
                assert_eq!(corners.len(), 3, "expected triangulated OBJ face: {line}");
                for corner in corners {
                    let position = positions[parse_obj_triplet_index(corner, 0)];
                    let tex_coord = tex_coords[parse_obj_triplet_index(corner, 1)];
                    let normal = normals[parse_obj_triplet_index(corner, 2)];
                    records.push(LegacyCornerRecord {
                        position,
                        tex_coord,
                        normal,
                    });
                }
            }
            _ => {}
        }
    }

    records
}

fn read_f32_tuple(attribute: &PointAttribute, point: PointIndex, components: usize) -> Vec<f32> {
    assert_eq!(
        attribute.data_type(),
        DataType::Float32,
        "legacy smoke fixture attributes should decode to float32"
    );
    let value_index = attribute.mapped_index(point).0 as usize;
    assert_ne!(
        value_index,
        u32::MAX as usize,
        "attribute has invalid mapping for point {}",
        point.0
    );
    assert!(
        value_index < attribute.size(),
        "attribute mapping for point {} is out of range: {} >= {}",
        point.0,
        value_index,
        attribute.size()
    );
    let offset = value_index * attribute.byte_stride() as usize;
    let data = attribute.buffer().data();
    (0..components)
        .map(|component| {
            let start = offset + component * 4;
            f32::from_le_bytes(data[start..start + 4].try_into().expect("f32 bytes"))
        })
        .collect()
}

fn rust_corner_records(mesh: &Mesh) -> Vec<LegacyCornerRecord> {
    let pos_id = mesh.named_attribute_id(GeometryAttributeType::Position);
    let tex_id = mesh.named_attribute_id(GeometryAttributeType::TexCoord);
    let normal_id = mesh.named_attribute_id(GeometryAttributeType::Normal);
    assert!(pos_id >= 0, "Rust decode missing POSITION attribute");
    assert!(tex_id >= 0, "Rust decode missing TEX_COORD attribute");
    assert!(normal_id >= 0, "Rust decode missing NORMAL attribute");

    let pos = mesh.attribute(pos_id);
    let tex = mesh.attribute(tex_id);
    let normal = mesh.attribute(normal_id);
    assert_eq!(pos.num_components(), 3);
    assert_eq!(tex.num_components(), 2);
    assert_eq!(normal.num_components(), 3);

    let mut records = Vec::with_capacity(mesh.num_faces() * 3);
    for face_id in 0..mesh.num_faces() {
        for point in mesh.face(draco_core::geometry_indices::FaceIndex(face_id as u32)) {
            let position = read_f32_tuple(pos, point, 3);
            let tex_coord = read_f32_tuple(tex, point, 2);
            let normal = read_f32_tuple(normal, point, 3);
            records.push(LegacyCornerRecord {
                position: [position[0], position[1], position[2]],
                tex_coord: [tex_coord[0], tex_coord[1]],
                normal: [normal[0], normal[1], normal[2]],
            });
        }
    }
    records
}

fn close_vec2(a: [f32; 2], b: [f32; 2], tolerance: f32) -> bool {
    (a[0] - b[0]).abs() <= tolerance && (a[1] - b[1]).abs() <= tolerance
}

fn close_vec3(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
    (a[0] - b[0]).abs() <= tolerance
        && (a[1] - b[1]).abs() <= tolerance
        && (a[2] - b[2]).abs() <= tolerance
}

fn close_legacy_corner_record(expected: &LegacyCornerRecord, actual: &LegacyCornerRecord) -> bool {
    close_vec3(expected.position, actual.position, 0.01)
        && close_vec2(expected.tex_coord, actual.tex_coord, 0.01)
        && close_vec3(expected.normal, actual.normal, 0.03)
}

fn assert_legacy_corner_records_match(
    fixture: &str,
    expected: &[LegacyCornerRecord],
    actual: &[LegacyCornerRecord],
) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{fixture}: corner record count mismatch"
    );
    let mut matched = vec![false; actual.len()];

    for expected_record in expected {
        let Some((actual_index, _)) = actual.iter().enumerate().find(|(index, actual_record)| {
            !matched[*index] && close_legacy_corner_record(expected_record, actual_record)
        }) else {
            panic!(
                "{fixture}: no Rust corner matched C++ corner {:?}\nRust corners: {:?}",
                expected_record, actual
            );
        };
        matched[actual_index] = true;
    }
}

#[test]
fn decode_legacy_mesh_v20_v21_from_testdata() {
    let fixtures = [
        "test_nm.obj.edgebreaker.1.0.0.drc",
        "test_nm.obj.edgebreaker.1.1.0.drc",
        "test_nm.obj.sequential.1.0.0.drc",
        "test_nm.obj.sequential.1.1.0.drc",
    ];

    for fixture in fixtures {
        let path = repo_testdata_dir().join(fixture);
        let bytes = read_file_bytes(&path);
        let (major, minor, geometry_type, _method) = parse_header(&bytes);

        assert_eq!(
            geometry_type,
            EncodedGeometryType::TriangularMesh,
            "{fixture} should be a mesh fixture"
        );
        assert!(
            major == 2 && (minor == 0 || minor == 1),
            "{fixture} should cover mesh bitstream v2.0 or v2.1, got v{major}.{minor}"
        );

        let mut buffer = DecoderBuffer::new(&bytes);
        let mut mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();
        let status = decoder.decode(&mut buffer, &mut mesh);

        assert!(
            status.is_ok(),
            "legacy mesh decode failed for {fixture} (v{major}.{minor}): {:?}",
            status.err()
        );
        assert!(mesh.num_points() > 0, "{fixture} decoded with 0 points");
        assert!(mesh.num_faces() > 0, "{fixture} decoded with 0 faces");
        assert!(
            mesh.num_attributes() > 0,
            "{fixture} decoded with 0 attributes"
        );
    }
}

#[test]
fn decode_point_cloud_sequential_v22_v23_from_testdata() {
    let fixtures = [
        "pc_color.drc",
        "point_cloud_no_qp.drc",
        "production_draco/bpy_point_cloud.seq.v2.3.pos_norm_color.drc",
    ];

    for fixture in fixtures {
        let path = repo_testdata_dir().join(fixture);
        let bytes = read_file_bytes(&path);
        let (major, minor, geometry_type, method) = parse_header(&bytes);

        assert_eq!(
            geometry_type,
            EncodedGeometryType::PointCloud,
            "{fixture} should be a point-cloud fixture"
        );
        assert_eq!(
            method, 0,
            "{fixture} should cover sequential point-cloud method"
        );
        assert!(
            major == 2 && (minor == 2 || minor == 3),
            "{fixture} should cover point-cloud bitstream v2.2 or v2.3, got v{major}.{minor}"
        );

        let mut buffer = DecoderBuffer::new(&bytes);
        let mut pc = PointCloud::new();
        let mut decoder = PointCloudDecoder::new();
        let status = decoder.decode(&mut buffer, &mut pc);

        assert!(
            status.is_ok(),
            "point-cloud sequential decode failed for {fixture} (v{major}.{minor}): {:?}",
            status.err()
        );
        assert!(pc.num_points() > 0, "{fixture} decoded with 0 points");
        assert!(
            pc.num_attributes() > 0,
            "{fixture} decoded with 0 attributes"
        );
    }
}

#[test]
fn decode_production_point_cloud_kdtree_fixture() {
    let fixture = "production_draco/bpy_point_cloud.kd.v2.3.pos_norm_color.drc";
    let path = repo_testdata_dir().join(fixture);
    let bytes = read_file_bytes(&path);
    let (major, minor, geometry_type, method) = parse_header(&bytes);

    assert_eq!(
        geometry_type,
        EncodedGeometryType::PointCloud,
        "{fixture} should be a point-cloud fixture"
    );
    assert_eq!(
        method, 1,
        "{fixture} should cover KD-tree point-cloud method"
    );
    assert_eq!((major, minor), (2, 3), "{fixture} should be v2.3");

    let mut buffer = DecoderBuffer::new(&bytes);
    let mut pc = PointCloud::new();
    let mut decoder = PointCloudDecoder::new();
    let status = decoder.decode(&mut buffer, &mut pc);

    assert!(
        status.is_ok(),
        "point-cloud KD-tree decode failed for {fixture}: {:?}",
        status.err()
    );
    assert!(pc.num_points() > 0, "{fixture} decoded with 0 points");
    assert!(
        pc.num_attributes() > 0,
        "{fixture} decoded with 0 attributes"
    );
}

#[test]
fn decode_generated_legacy_draco_smoke_fixtures() {
    let fixtures = [
        (
            "legacy_draco/cube_att.mesh_seq.1.0.0.drc",
            2,
            0,
            EncodedGeometryType::TriangularMesh,
            0,
        ),
        (
            "legacy_draco/cube_att.mesh_eb.1.0.0.drc",
            2,
            0,
            EncodedGeometryType::TriangularMesh,
            1,
        ),
        (
            "legacy_draco/cube_att.mesh_seq.1.1.0.drc",
            2,
            1,
            EncodedGeometryType::TriangularMesh,
            0,
        ),
        (
            "legacy_draco/cube_att.mesh_eb.1.1.0.drc",
            2,
            1,
            EncodedGeometryType::TriangularMesh,
            1,
        ),
        (
            "legacy_draco/point_cloud_pos_norm.seq.1.0.0.drc",
            2,
            0,
            EncodedGeometryType::PointCloud,
            0,
        ),
        (
            "legacy_draco/point_cloud_pos_norm.seq.1.1.0.drc",
            2,
            1,
            EncodedGeometryType::PointCloud,
            0,
        ),
    ];

    for (fixture, expected_major, expected_minor, expected_geometry, expected_method) in fixtures {
        let path = repo_testdata_dir().join(fixture);
        let bytes = read_file_bytes(&path);
        let (major, minor, geometry_type, method) = parse_header(&bytes);

        assert_eq!(major, expected_major, "{fixture} major version mismatch");
        assert_eq!(minor, expected_minor, "{fixture} minor version mismatch");
        assert_eq!(
            geometry_type, expected_geometry,
            "{fixture} geometry mismatch"
        );
        assert_eq!(method, expected_method, "{fixture} method mismatch");

        match geometry_type {
            EncodedGeometryType::TriangularMesh => {
                let mut buffer = DecoderBuffer::new(&bytes);
                let mut mesh = Mesh::new();
                let mut decoder = MeshDecoder::new();
                let status = decoder.decode(&mut buffer, &mut mesh);

                assert!(
                    status.is_ok(),
                    "generated legacy mesh decode failed for {fixture}: {:?}",
                    status.err()
                );
                assert!(mesh.num_points() > 0, "{fixture} decoded with 0 points");
                assert!(mesh.num_faces() > 0, "{fixture} decoded with 0 faces");
                assert!(
                    mesh.num_attributes() > 0,
                    "{fixture} decoded with 0 attributes"
                );
            }
            EncodedGeometryType::PointCloud => {
                let mut buffer = DecoderBuffer::new(&bytes);
                let mut pc = PointCloud::new();
                let mut decoder = PointCloudDecoder::new();
                let status = decoder.decode(&mut buffer, &mut pc);

                assert!(
                    status.is_ok(),
                    "generated legacy point-cloud decode failed for {fixture}: {:?}",
                    status.err()
                );
                assert!(pc.num_points() > 0, "{fixture} decoded with 0 points");
                assert!(
                    pc.num_attributes() > 0,
                    "{fixture} decoded with 0 attributes"
                );
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn generated_legacy_cube_attributes_match_cpp_decoder() {
    let fixtures = [
        (
            "legacy_draco/cube_att.mesh_seq.1.0.0.drc",
            LegacyDecoderVersion::V1_0_0,
        ),
        (
            "legacy_draco/cube_att.mesh_eb.1.0.0.drc",
            LegacyDecoderVersion::V1_0_0,
        ),
        (
            "legacy_draco/cube_att.mesh_seq.1.1.0.drc",
            LegacyDecoderVersion::V1_1_0,
        ),
        (
            "legacy_draco/cube_att.mesh_eb.1.1.0.drc",
            LegacyDecoderVersion::V1_1_0,
        ),
    ];

    for (fixture, decoder_version) in fixtures {
        let Some(decoder_path) = legacy_decoder_path(decoder_version) else {
            eprintln!(
                "Skipping {fixture}: set a matching DRACO_LEGACY_DECODER_* env var to enable legacy decoder comparison"
            );
            continue;
        };
        let drc_path = repo_testdata_dir().join(fixture);
        let obj_path = std::env::temp_dir().join(format!(
            "draco_legacy_attr_{}_{}.obj",
            decoder_version.label(),
            fixture.replace(['/', '\\', '.'], "_")
        ));

        let output = Command::new(decoder_path)
            .arg("-i")
            .arg(&drc_path)
            .arg("-o")
            .arg(&obj_path)
            .output()
            .unwrap_or_else(|e| {
                panic!(
                    "{fixture}: failed to run legacy Draco decoder {}: {e}",
                    decoder_version.label()
                )
            });
        assert!(
            output.status.success(),
            "{fixture}: legacy Draco decoder {} failed\nstdout:\n{}\nstderr:\n{}",
            decoder_version.label(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let cpp_obj = fs::read_to_string(&obj_path)
            .unwrap_or_else(|e| panic!("{fixture}: failed to read C++ decoded OBJ: {e}"));
        let cpp_records = parse_cpp_obj_corner_records(&cpp_obj);
        assert_eq!(
            cpp_records.len(),
            36,
            "{fixture}: expected 12 triangular faces from C++ decoder"
        );

        let bytes = read_file_bytes(&drc_path);
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();
        decoder
            .decode(&mut buffer, &mut mesh)
            .unwrap_or_else(|e| panic!("{fixture}: Rust decode failed: {e:?}"));
        assert_eq!(mesh.num_faces(), 12, "{fixture}: Rust face count mismatch");

        let rust_records = rust_corner_records(&mesh);
        assert_legacy_corner_records_match(fixture, &cpp_records, &rust_records);

        let _ = fs::remove_file(obj_path);
    }
}

#[test]
fn inventory_skipped_testdata_drc_fixtures() {
    let dir = repo_testdata_dir();
    let mut drc_files = collect_drc_files_recursive(&dir);
    drc_files.sort();
    assert!(!drc_files.is_empty(), "no .drc files found in testdata");

    let actual: Vec<_> = drc_files
        .iter()
        .filter_map(|path| {
            let bytes = read_file_bytes(path);
            skipped_fixture_for_current_decoder(path, &bytes)
        })
        .collect();

    let expected = vec![
        skipped(
            "cube_att.drc",
            1,
            1,
            GeometryKind::Mesh,
            1,
            SkipReason::UnsupportedBitstream,
        ),
        skipped(
            "cube_pc.drc",
            1,
            1,
            GeometryKind::PointCloud,
            0,
            SkipReason::UnsupportedBitstream,
        ),
        skipped(
            "test_nm.obj.edgebreaker.0.10.0.drc",
            1,
            2,
            GeometryKind::Mesh,
            1,
            SkipReason::UnsupportedBitstream,
        ),
        skipped(
            "test_nm.obj.edgebreaker.0.9.1.drc",
            1,
            1,
            GeometryKind::Mesh,
            1,
            SkipReason::UnsupportedBitstream,
        ),
        skipped(
            "test_nm.obj.sequential.0.10.0.drc",
            1,
            2,
            GeometryKind::Mesh,
            0,
            SkipReason::UnsupportedBitstream,
        ),
        skipped(
            "test_nm.obj.sequential.0.9.1.drc",
            1,
            1,
            GeometryKind::Mesh,
            0,
            SkipReason::UnsupportedBitstream,
        ),
        skipped(
            "test_nm_quant.0.9.0.drc",
            1,
            2,
            GeometryKind::Mesh,
            1,
            SkipReason::UnsupportedBitstream,
        ),
    ];

    assert_eq!(actual, expected);
}

#[test]
fn decode_all_testdata_top_level_drc_files() {
    let dir = repo_testdata_dir();
    let mut drc_files = collect_drc_files_recursive(&dir);

    drc_files.sort();
    assert!(!drc_files.is_empty(), "no .drc files found in testdata");

    let mut decoded_any = false;
    for path in drc_files {
        let bytes = read_file_bytes(&path);
        let (major, minor, geometry_type, method) = parse_header(&bytes);

        // Only decode files for bitstream variants we currently support.
        // This still exercises real shipped .drc assets without forcing us
        // to immediately implement all legacy layouts.
        match geometry_type {
            EncodedGeometryType::TriangularMesh => {
                if !supports_mesh_bitstream(major, minor) {
                    continue;
                }
                let mut buffer = DecoderBuffer::new(&bytes);
                let mut mesh = Mesh::new();
                let mut decoder = MeshDecoder::new();
                let status = decoder.decode(&mut buffer, &mut mesh);

                if let Err(DracoError::DracoError(ref msg)) = status {
                    if msg.starts_with("Unsupported Edgebreaker traversal decoder type") {
                        println!(
                            "Skipping {} due to unsupported traversal: {}",
                            path.display(),
                            msg
                        );
                        continue;
                    }
                }

                assert!(
                    status.is_ok(),
                    "mesh decode failed for {} (v{}.{}): {:?}",
                    path.display(),
                    major,
                    minor,
                    status.err()
                );
                decoded_any = true;
                assert!(
                    mesh.num_points() > 0,
                    "{} decoded with 0 points",
                    path.display()
                );
            }
            EncodedGeometryType::PointCloud => {
                if !supports_point_cloud_bitstream(major, minor, method) {
                    continue;
                }
                let mut buffer = DecoderBuffer::new(&bytes);
                let mut pc = PointCloud::new();
                let mut decoder = PointCloudDecoder::new();
                let status = decoder.decode(&mut buffer, &mut pc);
                assert!(
                    status.is_ok(),
                    "point cloud decode failed for {} (v{}.{} method={}): {:?}",
                    path.display(),
                    major,
                    minor,
                    method,
                    status.err()
                );
                decoded_any = true;
                assert!(
                    pc.num_points() > 0,
                    "{} decoded with 0 points",
                    path.display()
                );
            }
            _ => unreachable!(),
        }
    }

    assert!(
        decoded_any,
        "no supported .drc files were decoded; update supports_*() or add compatible fixtures"
    );
}

#[test]
fn roundtrip_encode_decode_mesh_from_testdata() {
    // Pick a v2.2 mesh that the current MeshDecoder supports.
    let path = repo_testdata_dir().join("test_nm.obj.edgebreaker.cl4.2.2.drc");
    let bytes = read_file_bytes(&path);
    let (geometry_type, mesh, _) = decode_drc(&bytes);
    assert_eq!(geometry_type, EncodedGeometryType::TriangularMesh);

    let original = mesh.expect("mesh missing");
    assert!(original.num_points() > 0);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(original.clone());

    // Use sequential encoding and quantization for reliable roundtrip
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 0); // Sequential encoding
    for i in 0..original.num_attributes() {
        options.set_attribute_int(i, "quantization_bits", 14);
    }
    // Keep defaults; this is primarily an integration sanity check.
    let mut enc = EncoderBuffer::new();
    let status = encoder.encode(&options, &mut enc);
    assert!(status.is_ok(), "re-encode failed: {:?}", status.err());

    let mut buffer = DecoderBuffer::new(enc.data());
    let mut decoded = Mesh::new();
    let mut decoder = MeshDecoder::new();
    let status = decoder.decode(&mut buffer, &mut decoded);
    assert!(status.is_ok(), "re-decode failed: {:?}", status.err());

    assert_eq!(decoded.num_faces(), original.num_faces());
    assert_eq!(decoded.num_points(), original.num_points());
    assert_eq!(decoded.num_attributes(), original.num_attributes());
}

#[test]
fn decode_point_cloud_kdtree_from_testdata() {
    let path = repo_testdata_dir().join("pc_kd_color.drc");
    let bytes = read_file_bytes(&path);
    let (geometry_type, _, pc) = decode_drc(&bytes);
    assert_eq!(geometry_type, EncodedGeometryType::PointCloud);

    let original = pc.expect("point cloud missing");
    assert!(original.num_points() > 0);

    // Minimal invariants.
    assert!(original.num_attributes() >= 1);
}
