//! Integration test: decode all .drc test files and verify point/face counts.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;

fn testdata_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
}

fn decode_file(path: &std::path::Path) -> Result<(usize, usize), String> {
    let mut f = File::open(path).map_err(|e| format!("open: {e}"))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).map_err(|e| format!("read: {e}"))?;

    let mut decoder_buffer = DecoderBuffer::new(&buf);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder
        .decode(&mut decoder_buffer, &mut mesh)
        .map_err(|e| format!("{e:?}"))?;

    Ok((mesh.num_points(), mesh.num_faces()))
}

/// Expected (num_points, num_faces) for each test file.
const EXPECTED: &[(&str, usize, usize)] = &[
    ("annulus_eb.drc", 8, 8),
    ("annulus.drc", 8, 8),
    ("bunny_cpp_standard.drc", 34834, 69451),
    ("bunny_cpp.drc", 34834, 69451),
    ("bunny_gltf.drc", 34834, 69451),
    ("car.drc", 1856, 1744),
    ("cube_att_sub_o_2.drc", 26, 12),
    ("cube_att_sub_o_no_metadata.drc", 26, 12),
    ("cube_att.drc", 24, 12),
    ("cube_att.obj.edgebreaker.cl10.2.2.drc", 24, 12),
    ("cube_att.obj.edgebreaker.cl4.2.2.drc", 24, 12),
    ("cube_att.obj.sequential.cl3.2.2.drc", 24, 12),
    ("cube_pc.drc", 24, 0),
    ("grid5x5_cpp.drc", 25, 32),
    ("lamp_cpp_std.drc", 7036, 12082),
    ("ngon12.drc", 12, 10),
    ("octagon_preserved.drc", 16, 6),
    ("pc_color.drc", 7733, 0),
    ("pc_kd_color.drc", 7733, 0),
    ("point_cloud_no_qp.drc", 21, 0),
    ("quad_test_cpp.drc", 4, 2),
    ("test_nm.obj.edgebreaker.0.9.1.drc", 99, 170),
    ("test_nm.obj.edgebreaker.0.10.0.drc", 99, 170),
    ("test_nm.obj.edgebreaker.1.0.0.drc", 99, 170),
    ("test_nm.obj.edgebreaker.1.1.0.drc", 99, 170),
    ("test_nm.obj.edgebreaker.cl10.2.2.drc", 99, 170),
    ("test_nm.obj.edgebreaker.cl4.2.2.drc", 99, 170),
    ("test_nm.obj.sequential.0.9.1.drc", 97, 170),
    ("test_nm.obj.sequential.0.10.0.drc", 97, 170),
    ("test_nm.obj.sequential.1.0.0.drc", 97, 170),
    ("test_nm.obj.sequential.1.1.0.drc", 97, 170),
    ("test_nm.obj.sequential.cl3.2.2.drc", 97, 170),
    ("test_nm_quant.0.9.0.drc", 99, 170),
];

#[test]
fn decode_all_drc_files() {
    let dir = testdata_dir();
    let mut failures = Vec::new();

    for &(name, expected_points, expected_faces) in EXPECTED {
        let path = dir.join(name);
        if !path.exists() {
            failures.push(format!("{name}: file not found"));
            continue;
        }
        match decode_file(&path) {
            Ok((pts, faces)) => {
                if pts != expected_points || faces != expected_faces {
                    failures.push(format!(
                        "{name}: expected ({expected_points}, {expected_faces}), got ({pts}, {faces})"
                    ));
                }
            }
            Err(e) => {
                failures.push(format!("{name}: decode error: {e}"));
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} file(s) failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
    }
}
