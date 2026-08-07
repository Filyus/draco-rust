// Profile sequential encoding to identify bottlenecks

mod common;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::prediction_scheme::EntryToPointIdMap;
use draco_core::EncoderOptions;
use std::f32::consts::PI;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

static OUTPUT_LOCK: Mutex<()> = Mutex::new(());

fn duration_to_us(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

fn avg_duration_us(duration: Duration, iterations: u32) -> f64 {
    duration_to_us(duration) / f64::from(iterations)
}

fn create_position_mesh(positions: Vec<f32>, faces: Vec<u32>) -> (Mesh, Vec<f32>, Vec<u32>) {
    debug_assert_eq!(positions.len() % 3, 0);
    debug_assert_eq!(faces.len() % 3, 0);

    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;

    let mut mesh = Mesh::new();
    mesh.set_num_points(num_points);
    mesh.set_num_faces(num_faces);

    let mut pos_attr = PointAttribute::new();
    pos_attr.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        num_points,
    );

    for i in 0..num_points {
        let offset = i * 3 * 4;
        pos_attr
            .buffer_mut()
            .update(&positions[i * 3].to_le_bytes(), Some(offset));
        pos_attr
            .buffer_mut()
            .update(&positions[i * 3 + 1].to_le_bytes(), Some(offset + 4));
        pos_attr
            .buffer_mut()
            .update(&positions[i * 3 + 2].to_le_bytes(), Some(offset + 8));
    }
    mesh.add_attribute(pos_attr);
    mesh.set_faces_from_flat_indices(&faces);

    (mesh, positions, faces)
}

fn create_grid_mesh(grid_size: usize) -> (Mesh, Vec<f32>, Vec<u32>) {
    let num_points = grid_size * grid_size;
    let num_faces = (grid_size - 1) * (grid_size - 1) * 2;

    // Create positions
    let mut positions = Vec::with_capacity(num_points * 3);
    for y in 0..grid_size {
        for x in 0..grid_size {
            let px = x as f32;
            let py = y as f32;
            let pz = (x as f32 * 0.2).sin() * (y as f32 * 0.2).cos() * 2.0;
            positions.push(px);
            positions.push(py);
            positions.push(pz);
        }
    }

    // Create faces
    let mut faces = Vec::with_capacity(num_faces * 3);
    for y in 0..grid_size - 1 {
        for x in 0..grid_size - 1 {
            let p0 = (y * grid_size + x) as u32;
            let p1 = (y * grid_size + x + 1) as u32;
            let p2 = ((y + 1) * grid_size + x) as u32;
            let p3 = ((y + 1) * grid_size + x + 1) as u32;

            faces.push(p0);
            faces.push(p1);
            faces.push(p2);

            faces.push(p1);
            faces.push(p3);
            faces.push(p2);
        }
    }

    create_position_mesh(positions, faces)
}

fn create_bipyramid_fan_mesh(ring_segments: usize) -> (Mesh, Vec<f32>, Vec<u32>) {
    assert!(ring_segments >= 3);

    let mut positions = Vec::with_capacity((ring_segments + 2) * 3);
    positions.extend_from_slice(&[0.0, 0.0, 1.0]);
    positions.extend_from_slice(&[0.0, 0.0, -1.0]);

    let step = 2.0 * PI / ring_segments as f32;
    for i in 0..ring_segments {
        let a = i as f32 * step;
        positions.extend_from_slice(&[a.cos(), a.sin(), 0.0]);
    }

    let mut faces = Vec::with_capacity(ring_segments * 2 * 3);
    for i in 0..ring_segments {
        let current = 2 + i as u32;
        let next = 2 + ((i + 1) % ring_segments) as u32;
        faces.extend_from_slice(&[0, current, next]);
        faces.extend_from_slice(&[1, next, current]);
    }

    create_position_mesh(positions, faces)
}

fn create_boundary_ribbon_mesh(length: usize) -> (Mesh, Vec<f32>, Vec<u32>) {
    assert!(length >= 2);

    let mut positions = Vec::with_capacity(length * 2 * 3);
    for i in 0..length {
        let x = i as f32;
        let z = (i as f32 * 0.07).sin() * 0.1;
        positions.extend_from_slice(&[x, 0.0, z]);
        positions.extend_from_slice(&[x, 1.0, z]);
    }

    let mut faces = Vec::with_capacity((length - 1) * 2 * 3);
    for i in 0..length - 1 {
        let p0 = (i * 2) as u32;
        let p1 = p0 + 1;
        let p2 = p0 + 2;
        let p3 = p0 + 3;

        faces.extend_from_slice(&[p0, p2, p1]);
        faces.extend_from_slice(&[p1, p2, p3]);
    }

    create_position_mesh(positions, faces)
}

fn create_torus_mesh(
    major_segments: usize,
    minor_segments: usize,
    irregular_diagonals: bool,
) -> (Mesh, Vec<f32>, Vec<u32>) {
    assert!(major_segments >= 3);
    assert!(minor_segments >= 3);

    let mut positions = Vec::with_capacity(major_segments * minor_segments * 3);
    for major in 0..major_segments {
        let u = 2.0 * PI * major as f32 / major_segments as f32;
        for minor in 0..minor_segments {
            let v = 2.0 * PI * minor as f32 / minor_segments as f32;
            let tube_radius = if irregular_diagonals {
                0.75 + 0.07 * (u * 3.0 + v * 5.0).sin()
            } else {
                0.75
            };
            let major_radius = 2.0;
            let ring = major_radius + tube_radius * v.cos();
            positions.extend_from_slice(&[ring * u.cos(), ring * u.sin(), tube_radius * v.sin()]);
        }
    }

    let mut faces = Vec::with_capacity(major_segments * minor_segments * 2 * 3);
    for major in 0..major_segments {
        let next_major = (major + 1) % major_segments;
        for minor in 0..minor_segments {
            let next_minor = (minor + 1) % minor_segments;
            let p00 = (major * minor_segments + minor) as u32;
            let p10 = (next_major * minor_segments + minor) as u32;
            let p01 = (major * minor_segments + next_minor) as u32;
            let p11 = (next_major * minor_segments + next_minor) as u32;

            let flip_diagonal =
                irregular_diagonals && ((major * 31 + minor * 17 + (major ^ minor) * 7) % 5 < 2);
            if flip_diagonal {
                faces.extend_from_slice(&[p00, p10, p11]);
                faces.extend_from_slice(&[p00, p11, p01]);
            } else {
                faces.extend_from_slice(&[p00, p10, p01]);
                faces.extend_from_slice(&[p10, p11, p01]);
            }
        }
    }

    create_position_mesh(positions, faces)
}

struct SeededRng {
    state: u64,
}

impl SeededRng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9e37_79b9_7f4a_7c15,
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn next_unit_f64(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
    }

    fn normal_f64(&mut self, mean: f64, stddev: f64) -> f64 {
        let u1 = self.next_unit_f64().max(f64::MIN_POSITIVE);
        let u2 = self.next_unit_f64();
        let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * PI as f64 * u2).cos();
        mean + z0 * stddev
    }

    fn normal_usize(&mut self, mean: f64, stddev: f64, min: usize, max: usize) -> usize {
        self.normal_f64(mean, stddev)
            .round()
            .clamp(min as f64, max as f64) as usize
    }

    fn normal_f32(&mut self, mean: f64, stddev: f64, min: f64, max: f64) -> f32 {
        self.normal_f64(mean, stddev).clamp(min, max) as f32
    }

    fn chance(&mut self, probability: f32) -> bool {
        self.next_unit_f64() < f64::from(probability)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SeededMeshFamily {
    Grid,
    Fan,
    Ribbon,
    Torus,
}

impl SeededMeshFamily {
    const ALL: [Self; 4] = [Self::Grid, Self::Fan, Self::Ribbon, Self::Torus];

    fn name(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::Fan => "fan",
            Self::Ribbon => "ribbon",
            Self::Torus => "torus",
        }
    }
}

struct SeededMeshStats {
    seed: u64,
    family: SeededMeshFamily,
}

fn create_seeded_grid_mesh(seed: u64) -> (Mesh, Vec<f32>, Vec<u32>, SeededMeshStats) {
    let mut rng = SeededRng::new(seed);
    let grid_size = rng.normal_usize(100.0, 12.0, 72, 128);
    let (mesh, positions, faces) = create_grid_mesh(grid_size);
    (
        mesh,
        positions,
        faces,
        SeededMeshStats {
            seed,
            family: SeededMeshFamily::Grid,
        },
    )
}

fn create_seeded_fan_mesh(seed: u64) -> (Mesh, Vec<f32>, Vec<u32>, SeededMeshStats) {
    let mut rng = SeededRng::new(seed);
    let ring_segments = rng.normal_usize(8192.0, 1024.0, 4096, 12_288);
    let (mesh, positions, faces) = create_bipyramid_fan_mesh(ring_segments);
    (
        mesh,
        positions,
        faces,
        SeededMeshStats {
            seed,
            family: SeededMeshFamily::Fan,
        },
    )
}

fn create_seeded_ribbon_mesh(seed: u64) -> (Mesh, Vec<f32>, Vec<u32>, SeededMeshStats) {
    let mut rng = SeededRng::new(seed);
    let length = rng.normal_usize(10_000.0, 1500.0, 6000, 14_000);
    let (mesh, positions, faces) = create_boundary_ribbon_mesh(length);
    (
        mesh,
        positions,
        faces,
        SeededMeshStats {
            seed,
            family: SeededMeshFamily::Ribbon,
        },
    )
}

fn create_seeded_torus_mesh(seed: u64) -> (Mesh, Vec<f32>, Vec<u32>, SeededMeshStats) {
    let mut rng = SeededRng::new(seed);
    let major_segments = rng.normal_usize(120.0, 12.0, 84, 156);
    let minor_segments = rng.normal_usize(84.0, 8.0, 60, 108);
    let flip_probability = rng.normal_f32(0.40, 0.12, 0.10, 0.70);
    let warp = rng.normal_f32(0.07, 0.025, 0.02, 0.14);
    let phase = (seed as f32 * 0.000_001).sin();

    let mut positions = Vec::with_capacity(major_segments * minor_segments * 3);
    for major in 0..major_segments {
        let u = 2.0 * PI * major as f32 / major_segments as f32;
        for minor in 0..minor_segments {
            let v = 2.0 * PI * minor as f32 / minor_segments as f32;
            let tube_radius = 0.75
                + warp * (u * 3.0 + v * 5.0 + phase).sin()
                + warp * 0.5 * (u * 11.0 - v * 2.0).cos();
            let major_radius = 2.0;
            let ring = major_radius + tube_radius * v.cos();
            positions.extend_from_slice(&[ring * u.cos(), ring * u.sin(), tube_radius * v.sin()]);
        }
    }

    let mut faces = Vec::with_capacity(major_segments * minor_segments * 2 * 3);
    for major in 0..major_segments {
        let next_major = (major + 1) % major_segments;
        for minor in 0..minor_segments {
            let next_minor = (minor + 1) % minor_segments;
            let p00 = (major * minor_segments + minor) as u32;
            let p10 = (next_major * minor_segments + minor) as u32;
            let p01 = (major * minor_segments + next_minor) as u32;
            let p11 = (next_major * minor_segments + next_minor) as u32;

            if rng.chance(flip_probability) {
                faces.extend_from_slice(&[p00, p10, p11]);
                faces.extend_from_slice(&[p00, p11, p01]);
            } else {
                faces.extend_from_slice(&[p00, p10, p01]);
                faces.extend_from_slice(&[p10, p11, p01]);
            }
        }
    }

    let stats = SeededMeshStats {
        seed,
        family: SeededMeshFamily::Torus,
    };
    let (mesh, positions, faces) = create_position_mesh(positions, faces);
    (mesh, positions, faces, stats)
}

fn create_seeded_mesh(
    family: SeededMeshFamily,
    seed: u64,
) -> (Mesh, Vec<f32>, Vec<u32>, SeededMeshStats) {
    match family {
        SeededMeshFamily::Grid => create_seeded_grid_mesh(seed),
        SeededMeshFamily::Fan => create_seeded_fan_mesh(seed),
        SeededMeshFamily::Ribbon => create_seeded_ribbon_mesh(seed),
        SeededMeshFamily::Torus => create_seeded_torus_mesh(seed),
    }
}

struct RealDrcCase {
    label: &'static str,
    path: &'static [&'static str],
}

struct RealMeshCase {
    label: &'static str,
    bytes: Vec<u8>,
    mesh: Mesh,
    num_points: usize,
    num_faces: usize,
    num_attributes: usize,
}

const REAL_DRC_CASES: &[RealDrcCase] = &[
    RealDrcCase {
        label: "annulus-eb",
        path: &["annulus_eb.drc"],
    },
    RealDrcCase {
        label: "annulus",
        path: &["annulus.drc"],
    },
    RealDrcCase {
        label: "ngon12",
        path: &["ngon12.drc"],
    },
    RealDrcCase {
        label: "grid5x5-cpp",
        path: &["grid5x5_cpp.drc"],
    },
    RealDrcCase {
        label: "test-nm-eb",
        path: &["test_nm.obj.edgebreaker.cl4.2.2.drc"],
    },
    RealDrcCase {
        label: "test-nm-seq",
        path: &["test_nm.obj.sequential.cl3.2.2.drc"],
    },
    RealDrcCase {
        label: "legacy-sphere-pos",
        path: &["legacy_draco", "sphere_pos.mesh_eb_cmp.2.2.drc"],
    },
    RealDrcCase {
        label: "legacy-sphere-norm",
        path: &["legacy_draco", "sphere.mesh_eb_norm.2.2.drc"],
    },
    RealDrcCase {
        label: "legacy-color",
        path: &["legacy_draco", "test.mesh_eb_color.2.2.drc"],
    },
    RealDrcCase {
        label: "legacy-bunny",
        path: &["legacy_draco", "bun_zipper.mesh_eb_valence.1.1.0.drc"],
    },
    RealDrcCase {
        label: "prod-pos-color",
        path: &[
            "production_draco",
            "test_pos_color.mesh_eb.v2.2.pos_color.drc",
        ],
    },
    RealDrcCase {
        label: "prod-cube",
        path: &["production_draco", "cube_att.mesh_eb.v2.2.pos_norm_uv.drc"],
    },
    RealDrcCase {
        label: "prod-blender",
        path: &[
            "production_draco",
            "blender_multi_color.mesh_eb.v2.2.pos_norm_uv_color012.drc",
        ],
    },
    RealDrcCase {
        label: "car",
        path: &["car.drc"],
    },
    RealDrcCase {
        label: "lamp",
        path: &["lamp_cpp_std.drc"],
    },
    RealDrcCase {
        label: "bunny",
        path: &["bunny_cpp_standard.drc"],
    },
];

fn testdata_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
}

fn decode_rust_mesh_once(encoded_data: &[u8]) -> Option<Mesh> {
    let mut decoder_buffer = DecoderBuffer::new(encoded_data);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder.decode(&mut decoder_buffer, &mut mesh).ok()?;
    Some(mesh)
}

fn profile_rust_reencode_mesh(mesh: &Mesh, speed: i32, iterations: u32) -> Option<(f64, usize)> {
    let mut rust_encode_us = 0.0;
    let mut rust_output_size = 0;

    for _ in 0..iterations {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        options.set_attribute_int(0, "quantization_bits", 10);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();

        let start = Instant::now();
        encoder.encode(&options, &mut encoder_buffer).ok()?;
        rust_encode_us += duration_to_us(start.elapsed());
        rust_output_size = encoder_buffer.data().len();
    }

    Some((rust_encode_us / f64::from(iterations), rust_output_size))
}

fn load_real_mesh_corpus() -> Vec<RealMeshCase> {
    let base = testdata_dir();
    let mut cases = Vec::new();

    for case in REAL_DRC_CASES {
        let mut path = base.clone();
        for segment in case.path {
            path = path.join(segment);
        }

        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };

        let Some(mesh) = decode_rust_mesh_once(&bytes) else {
            continue;
        };
        if mesh.num_faces() == 0 {
            continue;
        }

        let Some(cpp_decode) = draco_cpp_test_bridge::profile_cpp_decode(&bytes, 1) else {
            continue;
        };
        if cpp_decode.num_points as usize != mesh.num_points()
            || cpp_decode.num_faces as usize != mesh.num_faces()
        {
            continue;
        }

        let Some(cpp_encode) =
            draco_cpp_test_bridge::profile_cpp_reencode_mesh(&bytes, 5, 5, 10, 1)
        else {
            continue;
        };
        if cpp_encode.num_points as usize != mesh.num_points()
            || cpp_encode.num_faces as usize != mesh.num_faces()
            || cpp_encode.num_attributes as usize != mesh.num_attributes() as usize
        {
            continue;
        }

        if profile_rust_reencode_mesh(&mesh, 5, 1).is_none() {
            continue;
        }

        cases.push(RealMeshCase {
            label: case.label,
            bytes,
            num_points: mesh.num_points(),
            num_faces: mesh.num_faces(),
            num_attributes: mesh.num_attributes() as usize,
            mesh,
        });
    }

    cases.sort_by_key(|case| case.num_faces);
    cases
}

fn gaussian_corpus_index(rng: &mut SeededRng, len: usize) -> usize {
    debug_assert!(len > 0);
    let mean = (len - 1) as f64 * 0.5;
    let stddev = (len as f64 * 0.32).max(1.0);

    for _ in 0..8 {
        let index = rng.normal_f64(mean, stddev).round();
        if index >= 0.0 && index < len as f64 {
            return index as usize;
        }
    }

    rng.normal_f64(mean, stddev)
        .round()
        .clamp(0.0, (len - 1) as f64) as usize
}

#[derive(Clone, Copy)]
struct DistributionSummary {
    mean: f64,
    p10: f64,
    p50: f64,
    p90: f64,
    min: f64,
    max: f64,
}

fn percentile(sorted_values: &[f64], percentile: f64) -> f64 {
    debug_assert!(!sorted_values.is_empty());
    let rank = percentile.clamp(0.0, 1.0) * (sorted_values.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        sorted_values[lower]
    } else {
        let t = rank - lower as f64;
        sorted_values[lower] * (1.0 - t) + sorted_values[upper] * t
    }
}

fn summarize_distribution(values: &[f64]) -> DistributionSummary {
    assert!(!values.is_empty());

    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let mut sorted_values = values.to_vec();
    sorted_values.sort_by(|left, right| left.total_cmp(right));
    let min = sorted_values[0];
    let max = sorted_values[sorted_values.len() - 1];

    DistributionSummary {
        mean,
        p10: percentile(&sorted_values, 0.10),
        p50: percentile(&sorted_values, 0.50),
        p90: percentile(&sorted_values, 0.90),
        min,
        max,
    }
}

#[derive(Clone, Copy)]
enum CleanTopologyCase {
    RegularGrid,
    ClosedFan,
    BoundaryRibbon,
    RegularTorus,
    IrregularTorus,
}

impl CleanTopologyCase {
    const ALL: [Self; 5] = [
        Self::RegularGrid,
        Self::ClosedFan,
        Self::BoundaryRibbon,
        Self::RegularTorus,
        Self::IrregularTorus,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::RegularGrid => "regular-grid-100x100",
            Self::ClosedFan => "closed-bipyramid-fan-8192",
            Self::BoundaryRibbon => "boundary-ribbon-10000",
            Self::RegularTorus => "regular-torus-120x84",
            Self::IrregularTorus => "irregular-torus-120x84",
        }
    }

    fn focus(self) -> &'static str {
        match self {
            Self::RegularGrid => "regular open baseline; easy traversal and prediction",
            Self::ClosedFan => "two high-valence vertices without boundary noise",
            Self::BoundaryRibbon => "large boundary ratio on a clean manifold strip",
            Self::RegularTorus => "closed genus-1 handle with uniform valence",
            Self::IrregularTorus => "closed genus-1 handle with deterministic valence churn",
        }
    }

    fn create_mesh(self) -> (Mesh, Vec<f32>, Vec<u32>) {
        match self {
            Self::RegularGrid => create_grid_mesh(100),
            Self::ClosedFan => create_bipyramid_fan_mesh(8192),
            Self::BoundaryRibbon => create_boundary_ribbon_mesh(10_000),
            Self::RegularTorus => create_torus_mesh(120, 84, false),
            Self::IrregularTorus => create_torus_mesh(120, 84, true),
        }
    }
}

fn profile_rust_encode_only(mesh: &Mesh, speed: i32, iterations: u32) -> (f64, usize) {
    let mut rust_encode_us = 0.0;
    let mut rust_output_size = 0;

    for _ in 0..iterations {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        options.set_attribute_int(0, "quantization_bits", 10);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();

        let start = Instant::now();
        encoder
            .encode(&options, &mut encoder_buffer)
            .expect("Rust encode failed");
        rust_encode_us += duration_to_us(start.elapsed());
        rust_output_size = encoder_buffer.data().len();
    }

    (rust_encode_us / f64::from(iterations), rust_output_size)
}

fn encode_mesh_once(mesh: &Mesh, speed: i32) -> Vec<u8> {
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);
    options.set_attribute_int(0, "quantization_bits", 10);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut encoder_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoder_buffer)
        .expect("Rust encode failed");
    encoder_buffer.data().to_vec()
}

fn profile_rust_decode_only(encoded_data: &[u8], iterations: u32) -> (f64, usize, usize) {
    let mut rust_decode_us = 0.0;
    let mut rust_num_points = 0;
    let mut rust_num_faces = 0;

    for _ in 0..iterations {
        let mut decoder_buffer = DecoderBuffer::new(encoded_data);
        let mut out_mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();

        let start = Instant::now();
        decoder
            .decode(&mut decoder_buffer, &mut out_mesh)
            .expect("Rust decode failed");
        rust_decode_us += duration_to_us(start.elapsed());
        rust_num_points = out_mesh.num_points();
        rust_num_faces = out_mesh.num_faces();
    }

    (
        rust_decode_us / f64::from(iterations),
        rust_num_points,
        rust_num_faces,
    )
}

#[test]
fn profile_sequential_pipeline() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }

    println!("\n=== Profiling Sequential Encoding (Speed 10) ===\n");

    for grid_size in [50, 100] {
        let (mesh, positions, faces) = create_grid_mesh(grid_size);
        let num_points = positions.len() / 3;
        let num_faces = faces.len() / 3;

        println!(
            "Grid {}x{}: {} points, {} faces",
            grid_size, grid_size, num_points, num_faces
        );

        // Warm up
        for _ in 0..3 {
            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", 10);
            options.set_global_int("decoding_speed", 10);
            options.set_attribute_int(0, "quantization_bits", 10);

            let mut encoder = MeshEncoder::new();
            encoder.set_mesh(mesh.clone());
            let mut encoder_buffer = EncoderBuffer::new();
            let _ = encoder.encode(&options, &mut encoder_buffer);
        }

        // Time encoding
        let iterations = 20;
        let mut times = Vec::new();

        for _ in 0..iterations {
            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", 10);
            options.set_global_int("decoding_speed", 10);
            options.set_attribute_int(0, "quantization_bits", 10);

            let mut encoder = MeshEncoder::new();
            encoder.set_mesh(mesh.clone());
            let mut encoder_buffer = EncoderBuffer::new();

            let start = Instant::now();
            let _ = encoder.encode(&options, &mut encoder_buffer);
            let elapsed = start.elapsed();
            times.push(duration_to_us(elapsed) / 1000.0);
        }

        let avg: f64 = times.iter().sum::<f64>() / times.len() as f64;
        let min: f64 = times.iter().cloned().fold(f64::MAX, f64::min);
        let max: f64 = times.iter().cloned().fold(f64::MIN, f64::max);

        println!(
            "  Rust avg: {:.2}ms  min: {:.2}ms  max: {:.2}ms",
            avg, min, max
        );

        // C++ comparison
        let cpp_time = unsafe {
            let mut output_size = 0usize;
            draco_cpp_test_bridge::draco_benchmark_encode_mesh(
                num_points as u32,
                positions.as_ptr(),
                num_faces as u32,
                faces.as_ptr(),
                10,
                10,
                10,
                iterations,
                &mut output_size as *mut usize,
            )
        };

        if cpp_time >= 0 {
            let cpp_ms = cpp_time as f64 / 1000.0;
            println!("  C++  avg: {:.2}ms", cpp_ms);
            println!("  Speedup (C++/Rust): {:.2}x", cpp_ms / avg);
        }

        println!();
    }
}

#[test]
fn profile_detailed_breakdown() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }

    println!("\n=== Detailed Encoding Breakdown (Speed 10) ===\n");

    // Use 100x100 grid for meaningful measurements
    let grid_size = 100;
    let (mesh, positions, faces) = create_grid_mesh(grid_size);
    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;

    println!(
        "Grid {}x{}: {} points, {} faces\n",
        grid_size, grid_size, num_points, num_faces
    );

    // Warm up
    for _ in 0..3 {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", 10);
        options.set_global_int("decoding_speed", 10);
        options.set_attribute_int(0, "quantization_bits", 10);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();
        let _ = encoder.encode(&options, &mut encoder_buffer);
    }

    let iterations = 50;

    // Profile individual components

    // 1. Mesh clone
    let start = Instant::now();
    for _ in 0..iterations {
        let _cloned = mesh.clone();
    }
    let mesh_clone_us = avg_duration_us(start.elapsed(), iterations);

    // 2. Options setup
    let start = Instant::now();
    for _ in 0..iterations {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", 10);
        options.set_global_int("decoding_speed", 10);
        options.set_attribute_int(0, "quantization_bits", 10);
        std::hint::black_box(&options);
    }
    let options_us = avg_duration_us(start.elapsed(), iterations);

    // 3. Encoder creation + set_mesh
    let start = Instant::now();
    for _ in 0..iterations {
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        std::hint::black_box(&encoder);
    }
    let encoder_setup_us = avg_duration_us(start.elapsed(), iterations);

    // 4. Full encode
    let start = Instant::now();
    for _ in 0..iterations {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", 10);
        options.set_global_int("decoding_speed", 10);
        options.set_attribute_int(0, "quantization_bits", 10);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();
        let _ = encoder.encode(&options, &mut encoder_buffer);
    }
    let total_us = avg_duration_us(start.elapsed(), iterations);

    // Estimate time spent in actual encoding (exclude clone/setup)
    let encoding_core_us = total_us - mesh_clone_us - options_us;

    println!("Component breakdown (avg over {} iterations):", iterations);
    println!(
        "  Mesh clone:          {:7.1} µs ({:5.1}%)",
        mesh_clone_us,
        mesh_clone_us / total_us * 100.0
    );
    println!(
        "  Options setup:       {:7.1} µs ({:5.1}%)",
        options_us,
        options_us / total_us * 100.0
    );
    println!(
        "  Encoder setup:       {:7.1} µs ({:5.1}%)",
        encoder_setup_us - mesh_clone_us,
        (encoder_setup_us - mesh_clone_us) / total_us * 100.0
    );
    println!(
        "  Encoding (core):     {:7.1} µs ({:5.1}%)",
        encoding_core_us,
        encoding_core_us / total_us * 100.0
    );
    println!("  ─────────────────────────────");
    println!("  TOTAL:               {:7.1} µs", total_us);
    println!();

    // C++ comparison
    let cpp_time = unsafe {
        let mut output_size = 0usize;
        draco_cpp_test_bridge::draco_benchmark_encode_mesh(
            num_points as u32,
            positions.as_ptr(),
            num_faces as u32,
            faces.as_ptr(),
            10,
            10,
            10,
            iterations,
            &mut output_size as *mut usize,
        )
    };

    if cpp_time >= 0 {
        let cpp_us = cpp_time as f64;
        println!("C++ avg:               {:7.1} µs", cpp_us);
        println!("C++/Rust speedup:      {:7.2}x", cpp_us / total_us);
        println!();
        println!("If Rust matched C++ at encoding core, total would be:");
        let hypothetical = mesh_clone_us + options_us + cpp_us;
        println!(
            "  {:7.1} µs (speedup: {:.2}x)",
            hypothetical,
            cpp_us / hypothetical
        );
    }
}

use draco_core::attribute_quantization_transform::AttributeQuantizationTransform;
use draco_core::attribute_transform::AttributeTransform;
use draco_core::symbol_encoding::{encode_symbols, SymbolEncodingOptions};

#[test]
fn profile_encoding_stages() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    println!("\n=== Profiling Individual Encoding Stages (Speed 10) ===\n");

    let grid_size = 100;
    let (mesh, _, _) = create_grid_mesh(grid_size);
    let num_points = mesh.num_points();
    let num_components = 3;

    println!(
        "Grid {}x{}: {} points, {} components\n",
        grid_size, grid_size, num_points, num_components
    );

    let iterations = 100;

    // Get the position attribute
    let pos_att = mesh.attribute(0);
    let point_ids: Vec<PointIndex> = (0..num_points).map(|i| PointIndex(i as u32)).collect();

    // Stage 1: Quantization transform computation
    let start = Instant::now();
    for _ in 0..iterations {
        let mut q_transform = AttributeQuantizationTransform::new();
        q_transform
            .compute_parameters(pos_att, 10)
            .expect("compute_parameters");
        std::hint::black_box(&q_transform);
    }
    let quant_compute_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 2: Quantization transform application
    let mut q_transform = AttributeQuantizationTransform::new();
    q_transform
        .compute_parameters(pos_att, 10)
        .expect("compute_parameters");

    let start = Instant::now();
    for _ in 0..iterations {
        let mut portable = PointAttribute::default();
        q_transform
            .transform_attribute(
                pos_att,
                EntryToPointIdMap::from_point_indices(&point_ids),
                &mut portable,
            )
            .expect("transform_attribute");
        std::hint::black_box(&portable);
    }
    let quant_apply_us = avg_duration_us(start.elapsed(), iterations);

    // Get quantized values for symbol encoding test
    let mut portable = PointAttribute::default();
    q_transform
        .transform_attribute(
            pos_att,
            EntryToPointIdMap::from_point_indices(&point_ids),
            &mut portable,
        )
        .expect("transform_attribute");

    // Stage 3: Value gathering from portable attribute
    let start = Instant::now();
    for _ in 0..iterations {
        let mut values = Vec::with_capacity(num_points * 3);
        let data = portable.buffer().data();
        let byte_stride = portable.byte_stride() as usize;
        for i in 0..num_points {
            let offset = i * byte_stride;
            let x = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            let y = u32::from_le_bytes([
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            let z = u32::from_le_bytes([
                data[offset + 8],
                data[offset + 9],
                data[offset + 10],
                data[offset + 11],
            ]);
            values.push(x as i32);
            values.push(y as i32);
            values.push(z as i32);
        }
        std::hint::black_box(&values);
    }
    let gather_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 4: Delta prediction + wrap transform (simulating what happens)
    let mut values: Vec<i32> = Vec::with_capacity(num_points * 3);
    {
        let data = portable.buffer().data();
        let byte_stride = portable.byte_stride() as usize;
        for i in 0..num_points {
            let offset = i * byte_stride;
            let x = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            let y = u32::from_le_bytes([
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            let z = u32::from_le_bytes([
                data[offset + 8],
                data[offset + 9],
                data[offset + 10],
                data[offset + 11],
            ]);
            values.push(x as i32);
            values.push(y as i32);
            values.push(z as i32);
        }
    }

    let start = Instant::now();
    for _ in 0..iterations {
        let num_values = values.len();
        let mut corrections = vec![0i32; num_values];

        // Compute min/max
        let mut min_val = values[0];
        let mut max_val = values[0];
        for &val in &values[1..] {
            if val < min_val {
                min_val = val;
            }
            if val > max_val {
                max_val = val;
            }
        }

        let dif = (max_val as i64) - (min_val as i64);
        let max_dif = (1 + dif) as i32;
        let max_correction = max_dif / 2;
        let min_correction = -max_correction - if (max_dif & 1) == 0 { 0 } else { -1 };
        let max_correction_adj = if (max_dif & 1) == 0 {
            max_correction - 1
        } else {
            max_correction
        };

        // Delta + wrap
        let mut i = num_values - 3;
        while i >= 3 {
            let orig_x = values[i];
            let orig_y = values[i + 1];
            let orig_z = values[i + 2];

            let pred_x = values[i - 3].clamp(min_val, max_val);
            let pred_y = values[i - 2].clamp(min_val, max_val);
            let pred_z = values[i - 1].clamp(min_val, max_val);

            let mut corr_x = orig_x.wrapping_sub(pred_x);
            let mut corr_y = orig_y.wrapping_sub(pred_y);
            let mut corr_z = orig_z.wrapping_sub(pred_z);

            if corr_x < min_correction {
                corr_x = corr_x.wrapping_add(max_dif);
            } else if corr_x > max_correction_adj {
                corr_x = corr_x.wrapping_sub(max_dif);
            }
            if corr_y < min_correction {
                corr_y = corr_y.wrapping_add(max_dif);
            } else if corr_y > max_correction_adj {
                corr_y = corr_y.wrapping_sub(max_dif);
            }
            if corr_z < min_correction {
                corr_z = corr_z.wrapping_add(max_dif);
            } else if corr_z > max_correction_adj {
                corr_z = corr_z.wrapping_sub(max_dif);
            }

            corrections[i] = corr_x;
            corrections[i + 1] = corr_y;
            corrections[i + 2] = corr_z;

            i -= 3;
        }
        corrections[0] = values[0];
        corrections[1] = values[1];
        corrections[2] = values[2];

        std::hint::black_box(&corrections);
    }
    let delta_wrap_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 5: Zigzag encoding (convert signed to unsigned)
    let start = Instant::now();
    for _ in 0..iterations {
        let mut symbols = Vec::with_capacity(values.len());
        for &val in &values {
            let zigzag = if val < 0 {
                ((-val as u32) << 1) - 1
            } else {
                (val as u32) << 1
            };
            symbols.push(zigzag);
        }
        std::hint::black_box(&symbols);
    }
    let zigzag_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 6: Symbol encoding (the entropy coding stage)
    // Prepare symbols (zigzag encoded)
    let symbols: Vec<u32> = values
        .iter()
        .map(|&val| {
            if val < 0 {
                ((-val as u32) << 1) - 1
            } else {
                (val as u32) << 1
            }
        })
        .collect();

    let start = Instant::now();
    for _ in 0..iterations {
        let mut buffer = EncoderBuffer::new();
        let options = SymbolEncodingOptions {
            compression_level: 7,
        };
        encode_symbols(&symbols, 3, &options, &mut buffer);
        std::hint::black_box(&buffer);
    }
    let symbol_encode_us = avg_duration_us(start.elapsed(), iterations);

    // Print results
    let total_staged = quant_compute_us
        + quant_apply_us
        + gather_us
        + delta_wrap_us
        + zigzag_us
        + symbol_encode_us;

    println!("Stage breakdown (avg over {} iterations):", iterations);
    println!(
        "  1. Quantization compute:  {:7.1} µs ({:5.1}%)",
        quant_compute_us,
        quant_compute_us / total_staged * 100.0
    );
    println!(
        "  2. Quantization apply:    {:7.1} µs ({:5.1}%)",
        quant_apply_us,
        quant_apply_us / total_staged * 100.0
    );
    println!(
        "  3. Value gathering:       {:7.1} µs ({:5.1}%)",
        gather_us,
        gather_us / total_staged * 100.0
    );
    println!(
        "  4. Delta + wrap:          {:7.1} µs ({:5.1}%)",
        delta_wrap_us,
        delta_wrap_us / total_staged * 100.0
    );
    println!(
        "  5. Zigzag encoding:       {:7.1} µs ({:5.1}%)",
        zigzag_us,
        zigzag_us / total_staged * 100.0
    );
    println!(
        "  6. Symbol encoding:       {:7.1} µs ({:5.1}%)",
        symbol_encode_us,
        symbol_encode_us / total_staged * 100.0
    );
    println!("  ─────────────────────────────────────");
    println!("  Staged total:             {:7.1} µs", total_staged);
    println!();
    println!("Note: Full encode includes header, connectivity, attribute metadata,");
    println!("      and other bookkeeping not measured in individual stages.");
}

#[test]
fn profile_symbol_encoding_details() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    println!("\n=== Symbol Encoding Breakdown ===\n");

    // Create test data similar to what we have in a 100x100 mesh encode
    let num_points = 10201;
    let num_components = 3;
    let num_values = num_points * num_components;

    // Simulate quantized position values (11 bits for 100x100 @ speed 10)
    let max_value = 2047u32; // 11 bits
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let symbols: Vec<u32> = (0..num_values)
        .map(|i| {
            // Create somewhat realistic distribution - zigzag of deltas
            let mut h = DefaultHasher::new();
            i.hash(&mut h);
            (h.finish() as u32) % (max_value + 1)
        })
        .collect();

    let iterations = 100;

    // Stage A: Compute bit lengths
    let start = Instant::now();
    for _ in 0..iterations {
        let mut bit_lengths = Vec::with_capacity(symbols.len() / num_components);
        for chunk in symbols.chunks(num_components) {
            let mut max_component_value = chunk[0];
            for &val in &chunk[1..] {
                if val > max_component_value {
                    max_component_value = val;
                }
            }
            let bit_length = if max_component_value > 0 {
                32 - max_component_value.leading_zeros()
            } else {
                1
            };
            bit_lengths.push(bit_length);
        }
        std::hint::black_box(&bit_lengths);
    }
    let bit_lengths_us = avg_duration_us(start.elapsed(), iterations);

    // Prepare bit_lengths for reuse
    let _bit_lengths: Vec<u32> = symbols
        .chunks(num_components)
        .map(|chunk| {
            let max_comp = *chunk.iter().max().unwrap();
            if max_comp > 0 {
                32 - max_comp.leading_zeros()
            } else {
                1
            }
        })
        .collect();

    // Stage B: Compute frequencies for raw scheme
    let start = Instant::now();
    for _ in 0..iterations {
        let mut frequencies = vec![0u64; (max_value + 1) as usize];
        for &s in &symbols {
            frequencies[s as usize] += 1;
        }
        let mut num_unique: u32 = 0;
        for &f in &frequencies {
            if f > 0 {
                num_unique += 1;
            }
        }
        std::hint::black_box((frequencies, num_unique));
    }
    let freq_count_us = avg_duration_us(start.elapsed(), iterations);

    // Prepare frequencies
    let mut frequencies = vec![0u64; (max_value + 1) as usize];
    for &s in &symbols {
        frequencies[s as usize] += 1;
    }

    // Stage C: rANS table creation (probability normalization)
    let start = Instant::now();
    for _ in 0..iterations {
        // Simulate what RAnsSymbolEncoder::create does
        let rans_precision: u32 = 1 << 15; // typical precision
        let total_freq: u64 = symbols.len() as u64;
        let total_freq_d = total_freq as f64;
        let rans_precision_d = rans_precision as f64;

        let mut probs: Vec<u32> = Vec::with_capacity(frequencies.len());
        let mut total_rans_prob = 0u32;
        for &freq in &frequencies {
            let prob = freq as f64 / total_freq_d;
            let mut rans_prob = (prob * rans_precision_d + 0.5) as u32;
            if rans_prob == 0 && freq > 0 {
                rans_prob = 1;
            }
            probs.push(rans_prob);
            total_rans_prob += rans_prob;
        }
        std::hint::black_box((probs, total_rans_prob));
    }
    let table_create_us = avg_duration_us(start.elapsed(), iterations);

    // Stage D: Full rANS encoding loop (the hot path)
    // Build actual probability table
    use draco_core::rans_symbol_coding::RAnsSymbol;

    let rans_precision: u32 = 1 << 15;
    let total_freq_d = symbols.len() as f64;
    let rans_precision_d = rans_precision as f64;

    let mut prob_table: Vec<RAnsSymbol> = frequencies
        .iter()
        .map(|&freq| {
            let prob = freq as f64 / total_freq_d;
            let mut rans_prob = (prob * rans_precision_d + 0.5) as u32;
            if rans_prob == 0 && freq > 0 {
                rans_prob = 1;
            }
            RAnsSymbol {
                prob: rans_prob,
                cum_prob: 0,
            }
        })
        .collect();

    // Normalize and compute cumulative
    let mut total_prob = 0u32;
    for sym in &mut prob_table {
        sym.cum_prob = total_prob;
        total_prob += sym.prob;
    }

    let l_rans_base = rans_precision * 4;

    let start = Instant::now();
    for _ in 0..iterations {
        let mut state = l_rans_base;
        let mut buf: Vec<u8> = Vec::with_capacity(symbols.len() * 2);

        for &symbol in symbols.iter().rev() {
            let sym = prob_table[symbol as usize];
            let p = sym.prob;
            let renorm_bound = (l_rans_base / rans_precision) * 256 * p;

            while state >= renorm_bound {
                buf.push((state & 0xFF) as u8);
                state >>= 8;
            }

            let quot = state / p;
            let rem = state - quot * p;
            state = quot * rans_precision + rem + sym.cum_prob;
        }

        std::hint::black_box((buf, state));
    }
    let rans_loop_us = avg_duration_us(start.elapsed(), iterations);

    // Now profile the actual encode_symbols call for comparison
    let start = Instant::now();
    for _ in 0..iterations {
        let mut buffer = EncoderBuffer::new();
        let options = SymbolEncodingOptions {
            compression_level: 7,
        };
        encode_symbols(&symbols, num_components, &options, &mut buffer);
        std::hint::black_box(&buffer);
    }
    let full_encode_us = avg_duration_us(start.elapsed(), iterations);

    let total_measured = bit_lengths_us + freq_count_us + table_create_us + rans_loop_us;

    println!(
        "Symbol encoding breakdown (avg over {} iterations, {} symbols):",
        iterations,
        symbols.len()
    );
    println!(
        "  A. Compute bit lengths:   {:7.1} µs ({:5.1}%)",
        bit_lengths_us,
        bit_lengths_us / total_measured * 100.0
    );
    println!(
        "  B. Compute frequencies:   {:7.1} µs ({:5.1}%)",
        freq_count_us,
        freq_count_us / total_measured * 100.0
    );
    println!(
        "  C. rANS table creation:   {:7.1} µs ({:5.1}%)",
        table_create_us,
        table_create_us / total_measured * 100.0
    );
    println!(
        "  D. rANS encoding loop:    {:7.1} µs ({:5.1}%)",
        rans_loop_us,
        rans_loop_us / total_measured * 100.0
    );
    println!("  ─────────────────────────────────────");
    println!("  Isolated total:           {:7.1} µs", total_measured);
    println!("  Full encode_symbols():    {:7.1} µs", full_encode_us);
    println!();
    println!(
        "Overhead in encode_symbols: {:.1} µs ({:.1}%)",
        full_encode_us - total_measured,
        (full_encode_us - total_measured) / full_encode_us * 100.0
    );
}

#[test]
fn profile_rans_loop_micro() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    println!("\n=== rANS Loop Micro-benchmark ===\n");

    // Compare different approaches to the rANS encoding loop
    let num_symbols = 30603;
    let max_value = 2047u32;

    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let symbols: Vec<u32> = (0..num_symbols)
        .map(|i| {
            let mut h = DefaultHasher::new();
            i.hash(&mut h);
            (h.finish() as u32) % (max_value + 1)
        })
        .collect();

    // Build frequency table
    let mut frequencies = vec![0u64; (max_value + 1) as usize];
    for &s in &symbols {
        frequencies[s as usize] += 1;
    }

    // Build probability table (simplified)
    use draco_core::rans_symbol_coding::RAnsSymbol;
    let rans_precision: u32 = 1 << 15;
    let total_freq_d = symbols.len() as f64;
    let rans_precision_d = rans_precision as f64;

    let mut prob_table: Vec<RAnsSymbol> = frequencies
        .iter()
        .map(|&freq| {
            let prob = freq as f64 / total_freq_d;
            let mut rans_prob = (prob * rans_precision_d + 0.5) as u32;
            if rans_prob == 0 && freq > 0 {
                rans_prob = 1;
            }
            RAnsSymbol {
                prob: rans_prob,
                cum_prob: 0,
            }
        })
        .collect();

    let mut total_prob = 0u32;
    for sym in &mut prob_table {
        sym.cum_prob = total_prob;
        total_prob += sym.prob;
    }

    let l_rans_base = rans_precision * 4;
    let iterations = 100;

    // Approach 1: Current Rust implementation (Vec::push)
    let start = Instant::now();
    for _ in 0..iterations {
        let mut state = l_rans_base;
        let mut buf: Vec<u8> = Vec::with_capacity(num_symbols * 2);

        for &symbol in symbols.iter().rev() {
            let sym = prob_table[symbol as usize];
            let p = sym.prob;
            let renorm_bound = (l_rans_base / rans_precision) * 256 * p;

            while state >= renorm_bound {
                buf.push((state & 0xFF) as u8);
                state >>= 8;
            }

            let quot = state / p;
            let rem = state - quot * p;
            state = quot * rans_precision + rem + sym.cum_prob;
        }
        std::hint::black_box((buf, state));
    }
    let vec_push_us = avg_duration_us(start.elapsed(), iterations);

    // Approach 2: Pre-allocated buffer with index
    let start = Instant::now();
    for _ in 0..iterations {
        let mut state = l_rans_base;
        let mut buf = vec![0u8; num_symbols * 2];
        let mut buf_offset = 0usize;

        for &symbol in symbols.iter().rev() {
            let sym = prob_table[symbol as usize];
            let p = sym.prob;
            let renorm_bound = (l_rans_base / rans_precision) * 256 * p;

            while state >= renorm_bound {
                buf[buf_offset] = (state & 0xFF) as u8;
                buf_offset += 1;
                state >>= 8;
            }

            let quot = state / p;
            let rem = state - quot * p;
            state = quot * rans_precision + rem + sym.cum_prob;
        }
        std::hint::black_box((buf, state, buf_offset));
    }
    let prealloc_idx_us = avg_duration_us(start.elapsed(), iterations);

    // Approach 3: Unchecked index access
    let start = Instant::now();
    for _ in 0..iterations {
        let mut state = l_rans_base;
        let mut buf = vec![0u8; num_symbols * 2];
        let mut buf_offset = 0usize;

        for &symbol in symbols.iter().rev() {
            let sym = unsafe { *prob_table.get_unchecked(symbol as usize) };
            let p = sym.prob;
            let renorm_bound = (l_rans_base / rans_precision) * 256 * p;

            while state >= renorm_bound {
                unsafe {
                    *buf.get_unchecked_mut(buf_offset) = (state & 0xFF) as u8;
                }
                buf_offset += 1;
                state >>= 8;
            }

            let quot = state / p;
            let rem = state - quot * p;
            state = quot * rans_precision + rem + sym.cum_prob;
        }
        std::hint::black_box((buf, state, buf_offset));
    }
    let unchecked_us = avg_duration_us(start.elapsed(), iterations);

    // Approach 4: Compute renorm_bound outside with /4 factor
    // l_rans_base / rans_precision = 4, so renorm_bound = 4 * 256 * p = 1024 * p
    let start = Instant::now();
    for _ in 0..iterations {
        let mut state = l_rans_base;
        let mut buf = vec![0u8; num_symbols * 2];
        let mut buf_offset = 0usize;

        for &symbol in symbols.iter().rev() {
            let sym = unsafe { *prob_table.get_unchecked(symbol as usize) };
            let p = sym.prob;
            let renorm_bound = 1024 * p; // Simplified

            while state >= renorm_bound {
                unsafe {
                    *buf.get_unchecked_mut(buf_offset) = (state & 0xFF) as u8;
                }
                buf_offset += 1;
                state >>= 8;
            }

            let quot = state / p;
            let rem = state - quot * p;
            state = quot * rans_precision + rem + sym.cum_prob;
        }
        std::hint::black_box((buf, state, buf_offset));
    }
    let simplified_bound_us = avg_duration_us(start.elapsed(), iterations);

    println!(
        "rANS loop approaches ({} symbols, {} iterations):",
        num_symbols, iterations
    );
    println!("  1. Vec::push:             {:7.1} µs", vec_push_us);
    println!("  2. Pre-alloc + index:     {:7.1} µs", prealloc_idx_us);
    println!("  3. Unchecked access:      {:7.1} µs", unchecked_us);
    println!("  4. Simplified bound:      {:7.1} µs", simplified_bound_us);
    println!();
    println!(
        "Speedup from Vec::push to unchecked: {:.2}x",
        vec_push_us / unchecked_us
    );
}

#[test]
fn profile_full_encode_breakdown() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }

    println!("\n=== Full Encode Breakdown ===\n");

    // Profile the actual full encoding pipeline including connectivity
    let grid_size = 100;
    let (mesh, positions, faces) = create_grid_mesh(grid_size);
    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;

    let iterations = 50;

    // Stage 1: Mesh clone
    let start = Instant::now();
    for _ in 0..iterations {
        let cloned = mesh.clone();
        std::hint::black_box(cloned);
    }
    let mesh_clone_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 2: CornerTable init (connectivity processing)
    use draco_core::corner_table::CornerTable;
    use draco_core::geometry_indices::VertexIndex;

    // Create face data in the format CornerTable expects
    let face_data: Vec<[VertexIndex; 3]> = (0..num_faces)
        .map(|i| {
            let f = mesh.face(FaceIndex(i as u32));
            [
                VertexIndex(f[0].0),
                VertexIndex(f[1].0),
                VertexIndex(f[2].0),
            ]
        })
        .collect();

    let start = Instant::now();
    for _ in 0..iterations {
        let mut ct = CornerTable::new(num_faces);
        ct.init(&face_data);
        std::hint::black_box(&ct);
    }
    let corner_table_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 3: Full Rust encoding
    let start = Instant::now();
    for _ in 0..iterations {
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", 10);
        options.set_global_int("decoding_speed", 10);
        options.set_attribute_int(0, "quantization_bits", 10);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();
        let _ = encoder.encode(&options, &mut encoder_buffer);
        std::hint::black_box(encoder_buffer);
    }
    let full_encode_us = avg_duration_us(start.elapsed(), iterations);

    // Stage 4: C++ encode for comparison
    let cpp_avg = unsafe {
        let mut output_size = 0usize;
        draco_cpp_test_bridge::draco_benchmark_encode_mesh(
            num_points as u32,
            positions.as_ptr(),
            num_faces as u32,
            faces.as_ptr(),
            10,
            10,
            10,
            iterations,
            &mut output_size as *mut usize,
        ) as f64
    };

    println!(
        "Full encode breakdown ({}x{} mesh, {} iterations):",
        grid_size, grid_size, iterations
    );
    println!("  1. Mesh clone:            {:7.1} µs", mesh_clone_us);
    println!("  2. CornerTable init:      {:7.1} µs", corner_table_us);
    println!("  3. Full Rust encode:      {:7.1} µs", full_encode_us);
    println!("  4. Full C++ encode:       {:7.1} µs", cpp_avg);
    println!();
    println!("C++/Rust speedup: {:.2}x", cpp_avg / full_encode_us);
    println!();
    println!(
        "CornerTable as % of full: {:.1}%",
        corner_table_us / full_encode_us * 100.0
    );
    println!(
        "Mesh clone as % of full: {:.1}%",
        mesh_clone_us / full_encode_us * 100.0
    );

    // Now let's break down CornerTable init
    println!("\nCornerTable init sub-stages:");

    // Stage A: Just corner_to_vertex mapping
    let start = Instant::now();
    for _ in 0..iterations {
        let mut corner_to_vertex =
            vec![draco_core::geometry_indices::INVALID_VERTEX_INDEX; num_faces * 3];
        for (fi, face) in face_data.iter().enumerate() {
            for i in 0..3 {
                corner_to_vertex[fi * 3 + i] = face[i];
            }
        }
        std::hint::black_box(&corner_to_vertex);
    }
    let init_map_us = avg_duration_us(start.elapsed(), iterations);
    println!("  A. Init corner_to_vertex: {:7.1} µs", init_map_us);
    println!(
        "  B. Rest of init:          {:7.1} µs ({:.1}%)",
        corner_table_us - init_map_us,
        (corner_table_us - init_map_us) / corner_table_us * 100.0
    );
}

#[test]
fn profile_clean_topologies() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }

    println!("\n=== Clean Topology Cases ===\n");

    let iterations = 20;
    for case in CleanTopologyCase::ALL {
        let (mesh, positions, faces) = case.create_mesh();
        let num_points = positions.len() / 3;
        let num_faces = faces.len() / 3;

        println!(
            "{}: {} points, {} faces",
            case.name(),
            num_points,
            num_faces
        );
        println!("  Focus: {}", case.focus());

        for speed in 0..=10 {
            let cpp_profile = draco_cpp_test_bridge::profile_cpp_encode(
                &positions, &faces, speed, speed, 10, iterations,
            )
            .expect("C++ profile failed");
            let (rust_encode_us, rust_output_size) =
                profile_rust_encode_only(&mesh, speed, iterations);
            let cpp_encode_us = cpp_profile.encode_time_us as f64;

            println!(
                "  speed {speed}: C++ encode {:7.1} µs, Rust encode {:7.1} µs, \
                 speedup {:4.2}x, bytes C++={} Rust={} {}",
                cpp_encode_us,
                rust_encode_us,
                cpp_encode_us / rust_encode_us,
                cpp_profile.output_size,
                rust_output_size,
                if rust_output_size == cpp_profile.output_size {
                    "match"
                } else {
                    "mismatch"
                }
            );
        }

        println!();
    }
}

#[test]
fn profile_seeded_mesh_sweep() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }

    println!("\n=== Seeded Mesh Sweep ===\n");

    let seeds_per_family = 3;
    let samples = SeededMeshFamily::ALL.len() * seeds_per_family;
    let iterations = 10;
    let base_seed = 0xd1ac_0c0d_ebba_5eed_u64;
    let seed_step = 0x517c_c1b7_2722_0a95_u64;
    let mut cases = Vec::with_capacity(samples);

    for (family_index, family) in SeededMeshFamily::ALL.iter().copied().enumerate() {
        for seed_index in 0..seeds_per_family {
            let sample_index = family_index * seeds_per_family + seed_index;
            let seed = base_seed.wrapping_add((sample_index as u64).wrapping_mul(seed_step));
            cases.push(create_seeded_mesh(family, seed));
        }
    }

    let points: Vec<f64> = cases
        .iter()
        .map(|(_, positions, _, _)| (positions.len() / 3) as f64)
        .collect();
    let faces: Vec<f64> = cases
        .iter()
        .map(|(_, _, faces, _)| (faces.len() / 3) as f64)
        .collect();

    let point_counts = summarize_distribution(&points);
    let face_counts = summarize_distribution(&faces);

    let first_seed = cases.first().map(|(_, _, _, stats)| stats.seed).unwrap();
    let last_seed = cases.last().map(|(_, _, _, stats)| stats.seed).unwrap();
    println!(
        "samples: {samples}, iterations/sample: {iterations}, seeds: {first_seed:#018x}..{last_seed:#018x}"
    );
    for family in SeededMeshFamily::ALL {
        let count = cases
            .iter()
            .filter(|(_, _, _, stats)| stats.family == family)
            .count();
        println!("{} samples: {}", family.name(), count);
    }
    println!(
        "points: avg {:.0}, p50 {:.0}, p10..p90 [{:.0}..{:.0}], min..max [{:.0}..{:.0}]",
        point_counts.mean,
        point_counts.p50,
        point_counts.p10,
        point_counts.p90,
        point_counts.min,
        point_counts.max
    );
    println!(
        "faces: avg {:.0}, p50 {:.0}, p10..p90 [{:.0}..{:.0}], min..max [{:.0}..{:.0}]\n",
        face_counts.mean,
        face_counts.p50,
        face_counts.p10,
        face_counts.p90,
        face_counts.min,
        face_counts.max
    );

    for speed in 0..=10 {
        let mut cpp_times = Vec::with_capacity(samples);
        let mut cpp_us_per_k_faces = Vec::with_capacity(samples);
        let mut rust_times = Vec::with_capacity(samples);
        let mut rust_us_per_k_faces = Vec::with_capacity(samples);
        let mut speedups = Vec::with_capacity(samples);
        let mut bytes_matches = 0;

        for (mesh, positions, faces, _) in &cases {
            let num_faces = faces.len() / 3;
            let cpp_profile = draco_cpp_test_bridge::profile_cpp_encode(
                positions,
                faces,
                speed,
                speed,
                10,
                iterations as u32,
            )
            .expect("C++ profile failed");
            let (rust_encode_us, rust_output_size) =
                profile_rust_encode_only(mesh, speed, iterations as u32);
            let cpp_encode_us = cpp_profile.encode_time_us as f64;

            cpp_times.push(cpp_encode_us);
            cpp_us_per_k_faces.push(cpp_encode_us / num_faces as f64 * 1000.0);
            rust_times.push(rust_encode_us);
            rust_us_per_k_faces.push(rust_encode_us / num_faces as f64 * 1000.0);
            speedups.push(cpp_encode_us / rust_encode_us);
            if rust_output_size == cpp_profile.output_size {
                bytes_matches += 1;
            }
        }

        let cpp = summarize_distribution(&cpp_times);
        let cpp_per_k = summarize_distribution(&cpp_us_per_k_faces);
        let rust = summarize_distribution(&rust_times);
        let rust_per_k = summarize_distribution(&rust_us_per_k_faces);
        let speedup = summarize_distribution(&speedups);
        println!(
            "speed {speed}: raw us C++ avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}], \
             Rust avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}]",
            cpp.mean, cpp.p50, cpp.p10, cpp.p90, rust.mean, rust.p50, rust.p10, rust.p90,
        );
        println!(
            "speed {speed}: us/1k faces C++ avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}], \
             Rust avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}]",
            cpp_per_k.mean,
            cpp_per_k.p50,
            cpp_per_k.p10,
            cpp_per_k.p90,
            rust_per_k.mean,
            rust_per_k.p50,
            rust_per_k.p10,
            rust_per_k.p90,
        );
        println!(
            "speed {speed}: speedup avg {:.2}x, p50 {:.2}x, p10..p90 [{:.2}..{:.2}], \
             bytes match {}/{}",
            speedup.mean, speedup.p50, speedup.p10, speedup.p90, bytes_matches, samples
        );
    }

    println!();

    for speed in 0..=10 {
        let mut cpp_times = Vec::with_capacity(samples);
        let mut cpp_us_per_k_faces = Vec::with_capacity(samples);
        let mut rust_times = Vec::with_capacity(samples);
        let mut rust_us_per_k_faces = Vec::with_capacity(samples);
        let mut speedups = Vec::with_capacity(samples);
        let mut decoded_matches = 0;

        for (mesh, _, faces, _) in &cases {
            let num_faces = faces.len() / 3;
            let encoded_data = encode_mesh_once(mesh, speed);
            let cpp_result =
                draco_cpp_test_bridge::profile_cpp_decode(&encoded_data, iterations as u32)
                    .expect("C++ decode failed");
            let (rust_decode_us, rust_num_points, rust_num_faces) =
                profile_rust_decode_only(&encoded_data, iterations as u32);
            let cpp_decode_us = cpp_result.decode_time_us as f64;

            cpp_times.push(cpp_decode_us);
            cpp_us_per_k_faces.push(cpp_decode_us / num_faces as f64 * 1000.0);
            rust_times.push(rust_decode_us);
            rust_us_per_k_faces.push(rust_decode_us / num_faces as f64 * 1000.0);
            speedups.push(cpp_decode_us / rust_decode_us);

            if cpp_result.num_points as usize == rust_num_points
                && cpp_result.num_faces as usize == rust_num_faces
            {
                decoded_matches += 1;
            }
        }

        let cpp = summarize_distribution(&cpp_times);
        let cpp_per_k = summarize_distribution(&cpp_us_per_k_faces);
        let rust = summarize_distribution(&rust_times);
        let rust_per_k = summarize_distribution(&rust_us_per_k_faces);
        let speedup = summarize_distribution(&speedups);
        println!(
            "decode speed {speed}: raw us C++ avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}], \
             Rust avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}]",
            cpp.mean, cpp.p50, cpp.p10, cpp.p90, rust.mean, rust.p50, rust.p10, rust.p90,
        );
        println!(
            "decode speed {speed}: us/1k faces C++ avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}], \
             Rust avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}]",
            cpp_per_k.mean,
            cpp_per_k.p50,
            cpp_per_k.p10,
            cpp_per_k.p90,
            rust_per_k.mean,
            rust_per_k.p50,
            rust_per_k.p10,
            rust_per_k.p90,
        );
        println!(
            "decode speed {speed}: speedup avg {:.2}x, p50 {:.2}x, p10..p90 [{:.2}..{:.2}], \
             decoded size match {}/{}",
            speedup.mean, speedup.p50, speedup.p10, speedup.p90, decoded_matches, samples
        );
    }
}

#[test]
fn profile_real_corpus_gaussian_sweep() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }

    println!("\n=== Real Corpus Gaussian Sweep ===\n");

    let corpus = load_real_mesh_corpus();
    assert!(!corpus.is_empty(), "real mesh corpus is empty");

    let samples = 24;
    let iterations = 5;
    let decode_iterations = 10;
    let seed = 0x6a75_7373_7265_616c_u64;
    let mut rng = SeededRng::new(seed);
    let sample_indices: Vec<usize> = (0..samples)
        .map(|_| gaussian_corpus_index(&mut rng, corpus.len()))
        .collect();

    let selected_points: Vec<f64> = sample_indices
        .iter()
        .map(|&index| corpus[index].num_points as f64)
        .collect();
    let selected_faces: Vec<f64> = sample_indices
        .iter()
        .map(|&index| corpus[index].num_faces as f64)
        .collect();
    let point_counts = summarize_distribution(&selected_points);
    let face_counts = summarize_distribution(&selected_faces);

    println!(
        "corpus meshes: {}, samples: {}, iterations/sample: {}, decode iterations/sample: {}, seed: {seed:#018x}",
        corpus.len(),
        samples,
        iterations,
        decode_iterations
    );
    println!(
        "sampled points: avg {:.0}, p50 {:.0}, p10..p90 [{:.0}..{:.0}], min..max [{:.0}..{:.0}]",
        point_counts.mean,
        point_counts.p50,
        point_counts.p10,
        point_counts.p90,
        point_counts.min,
        point_counts.max
    );
    println!(
        "sampled faces: avg {:.0}, p50 {:.0}, p10..p90 [{:.0}..{:.0}], min..max [{:.0}..{:.0}]",
        face_counts.mean,
        face_counts.p50,
        face_counts.p10,
        face_counts.p90,
        face_counts.min,
        face_counts.max
    );
    println!("sampled files:");
    for (sample, &index) in sample_indices.iter().enumerate() {
        let case = &corpus[index];
        println!(
            "  {sample:02}: {:<18} {:>6} points {:>6} faces {:>2} attrs",
            case.label, case.num_points, case.num_faces, case.num_attributes
        );
    }
    println!();

    let mut source_cpp_times = Vec::with_capacity(samples);
    let mut source_cpp_us_per_k_faces = Vec::with_capacity(samples);
    let mut source_rust_times = Vec::with_capacity(samples);
    let mut source_rust_us_per_k_faces = Vec::with_capacity(samples);
    let mut source_speedups = Vec::with_capacity(samples);
    let mut source_decoded_matches = 0;

    for &index in &sample_indices {
        let case = &corpus[index];
        let cpp_decode =
            draco_cpp_test_bridge::profile_cpp_decode(&case.bytes, decode_iterations as u32)
                .expect("C++ source decode failed");
        let (rust_decode_us, rust_num_points, rust_num_faces) =
            profile_rust_decode_only(&case.bytes, decode_iterations as u32);
        let cpp_decode_us = cpp_decode.decode_time_us as f64;

        source_cpp_times.push(cpp_decode_us);
        source_cpp_us_per_k_faces.push(cpp_decode_us / case.num_faces as f64 * 1000.0);
        source_rust_times.push(rust_decode_us);
        source_rust_us_per_k_faces.push(rust_decode_us / case.num_faces as f64 * 1000.0);
        source_speedups.push(cpp_decode_us / rust_decode_us);

        if cpp_decode.num_points as usize == rust_num_points
            && cpp_decode.num_faces as usize == rust_num_faces
        {
            source_decoded_matches += 1;
        }
    }

    let cpp = summarize_distribution(&source_cpp_times);
    let cpp_per_k = summarize_distribution(&source_cpp_us_per_k_faces);
    let rust = summarize_distribution(&source_rust_times);
    let rust_per_k = summarize_distribution(&source_rust_us_per_k_faces);
    let speedup = summarize_distribution(&source_speedups);
    println!(
        "source decode: raw us C++ avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}], \
         Rust avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}]",
        cpp.mean, cpp.p50, cpp.p10, cpp.p90, rust.mean, rust.p50, rust.p10, rust.p90,
    );
    println!(
        "source decode: us/1k faces C++ avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}], \
         Rust avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}]",
        cpp_per_k.mean,
        cpp_per_k.p50,
        cpp_per_k.p10,
        cpp_per_k.p90,
        rust_per_k.mean,
        rust_per_k.p50,
        rust_per_k.p10,
        rust_per_k.p90,
    );
    println!(
        "source decode: speedup avg {:.2}x, p50 {:.2}x, p10..p90 [{:.2}..{:.2}], \
         decoded size match {}/{}\n",
        speedup.mean, speedup.p50, speedup.p10, speedup.p90, source_decoded_matches, samples
    );

    for speed in 0..=10 {
        let mut cpp_times = Vec::with_capacity(samples);
        let mut cpp_us_per_k_faces = Vec::with_capacity(samples);
        let mut rust_times = Vec::with_capacity(samples);
        let mut rust_us_per_k_faces = Vec::with_capacity(samples);
        let mut speedups = Vec::with_capacity(samples);
        let mut attr_matches = 0;

        for &index in &sample_indices {
            let case = &corpus[index];
            let cpp_profile = draco_cpp_test_bridge::profile_cpp_reencode_mesh(
                &case.bytes,
                speed,
                speed,
                10,
                iterations as u32,
            )
            .expect("C++ real re-encode profile failed");
            let (rust_encode_us, _) =
                profile_rust_reencode_mesh(&case.mesh, speed, iterations as u32)
                    .expect("Rust real re-encode failed");
            let cpp_encode_us = cpp_profile.encode_time_us as f64;

            cpp_times.push(cpp_encode_us);
            cpp_us_per_k_faces.push(cpp_encode_us / case.num_faces as f64 * 1000.0);
            rust_times.push(rust_encode_us);
            rust_us_per_k_faces.push(rust_encode_us / case.num_faces as f64 * 1000.0);
            speedups.push(cpp_encode_us / rust_encode_us);
            if cpp_profile.num_attributes as usize == case.num_attributes {
                attr_matches += 1;
            }
        }

        let cpp = summarize_distribution(&cpp_times);
        let cpp_per_k = summarize_distribution(&cpp_us_per_k_faces);
        let rust = summarize_distribution(&rust_times);
        let rust_per_k = summarize_distribution(&rust_us_per_k_faces);
        let speedup = summarize_distribution(&speedups);
        println!(
            "real encode speed {speed}: raw us C++ avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}], \
             Rust avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}]",
            cpp.mean, cpp.p50, cpp.p10, cpp.p90, rust.mean, rust.p50, rust.p10, rust.p90,
        );
        println!(
            "real encode speed {speed}: us/1k faces C++ avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}], \
             Rust avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}]",
            cpp_per_k.mean,
            cpp_per_k.p50,
            cpp_per_k.p10,
            cpp_per_k.p90,
            rust_per_k.mean,
            rust_per_k.p50,
            rust_per_k.p10,
            rust_per_k.p90,
        );
        println!(
            "real encode speed {speed}: speedup avg {:.2}x, p50 {:.2}x, p10..p90 [{:.2}..{:.2}], \
             attr count match {}/{}",
            speedup.mean, speedup.p50, speedup.p10, speedup.p90, attr_matches, samples
        );
    }

    println!();

    for speed in 0..=10 {
        let mut cpp_times = Vec::with_capacity(samples);
        let mut cpp_us_per_k_faces = Vec::with_capacity(samples);
        let mut rust_times = Vec::with_capacity(samples);
        let mut rust_us_per_k_faces = Vec::with_capacity(samples);
        let mut speedups = Vec::with_capacity(samples);
        let mut decoded_matches = 0;
        let mut decode_failures = 0;

        for &index in &sample_indices {
            let case = &corpus[index];
            let encoded_data = encode_mesh_once(&case.mesh, speed);
            let Some(cpp_result) =
                draco_cpp_test_bridge::profile_cpp_decode(&encoded_data, decode_iterations as u32)
            else {
                decode_failures += 1;
                continue;
            };
            let (rust_decode_us, rust_num_points, rust_num_faces) =
                profile_rust_decode_only(&encoded_data, decode_iterations as u32);
            let cpp_decode_us = cpp_result.decode_time_us as f64;

            cpp_times.push(cpp_decode_us);
            cpp_us_per_k_faces.push(cpp_decode_us / case.num_faces as f64 * 1000.0);
            rust_times.push(rust_decode_us);
            rust_us_per_k_faces.push(rust_decode_us / case.num_faces as f64 * 1000.0);
            speedups.push(cpp_decode_us / rust_decode_us);

            if cpp_result.num_points as usize == rust_num_points
                && cpp_result.num_faces as usize == rust_num_faces
            {
                decoded_matches += 1;
            }
        }

        if cpp_times.is_empty() {
            println!(
                "real decode speed {speed}: no C++-decodable Rust outputs, decode failures {decode_failures}/{samples}"
            );
            continue;
        }

        let cpp = summarize_distribution(&cpp_times);
        let cpp_per_k = summarize_distribution(&cpp_us_per_k_faces);
        let rust = summarize_distribution(&rust_times);
        let rust_per_k = summarize_distribution(&rust_us_per_k_faces);
        let speedup = summarize_distribution(&speedups);
        let comparable_samples = cpp_times.len();
        println!(
            "real decode speed {speed}: raw us C++ avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}], \
             Rust avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}]",
            cpp.mean, cpp.p50, cpp.p10, cpp.p90, rust.mean, rust.p50, rust.p10, rust.p90,
        );
        println!(
            "real decode speed {speed}: us/1k faces C++ avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}], \
             Rust avg {:.1}, p50 {:.1}, p10..p90 [{:.1}..{:.1}]",
            cpp_per_k.mean,
            cpp_per_k.p50,
            cpp_per_k.p10,
            cpp_per_k.p90,
            rust_per_k.mean,
            rust_per_k.p50,
            rust_per_k.p10,
            rust_per_k.p90,
        );
        println!(
            "real decode speed {speed}: speedup avg {:.2}x, p50 {:.2}x, p10..p90 [{:.2}..{:.2}], \
             decoded size match {}/{}, decode failures {}/{}",
            speedup.mean,
            speedup.p50,
            speedup.p10,
            speedup.p90,
            decoded_matches,
            comparable_samples,
            decode_failures,
            samples
        );
    }
}

#[test]
fn profile_mesh_clone_overhead() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    println!("\n=== Profiling Mesh Clone Overhead ===\n");

    let (mesh, _, _) = create_grid_mesh(200);

    // Time mesh clone
    let iterations = 100;
    let start = Instant::now();
    for _ in 0..iterations {
        let _cloned = mesh.clone();
    }
    let elapsed = start.elapsed();
    let avg_clone = avg_duration_us(elapsed, iterations) / 1000.0;

    println!("Mesh clone (200x200): {:.3}ms", avg_clone);
}

#[test]
fn profile_point_ids_creation() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    println!("\n=== Profiling point_ids creation ===\n");

    for num_points in [2500, 10000, 40000] {
        let iterations = 1000;
        let start = Instant::now();
        for _ in 0..iterations {
            let point_ids: Vec<PointIndex> =
                (0..num_points).map(|i| PointIndex(i as u32)).collect();
            std::hint::black_box(&point_ids);
        }
        let elapsed = start.elapsed();
        let avg = avg_duration_us(elapsed, iterations) / 1000.0;

        println!("point_ids creation ({} points): {:.4}ms", num_points, avg);
    }
}

#[test]
fn profile_rust_vs_cpp_breakdown() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    common::disable_noisy_debug_env();

    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    println!("\n=== Detailed C++ vs Rust Profile Breakdown ===\n");

    let grid_size = 100;
    let (mesh, positions, faces) = create_grid_mesh(grid_size);
    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;
    let iterations = 50;

    println!(
        "Grid {}x{}: {} points, {} faces ({} iterations)\n",
        grid_size, grid_size, num_points, num_faces, iterations
    );

    for speed in 0..=10 {
        println!("=== Speed {} ===\n", speed);

        // C++ Profile
        let cpp_profile = draco_cpp_test_bridge::profile_cpp_encode(
            &positions, &faces, speed, speed, 10, iterations,
        )
        .expect("C++ profile failed");

        println!("C++ Breakdown:");
        println!(
            "  Mesh setup:       {:7.1} µs ({:.1}%)",
            cpp_profile.mesh_setup_us as f64,
            cpp_profile.mesh_setup_us as f64 / cpp_profile.total_time_us as f64 * 100.0
        );
        println!(
            "  Encoder setup:    {:7.1} µs ({:.1}%)",
            cpp_profile.encoder_setup_us as f64,
            cpp_profile.encoder_setup_us as f64 / cpp_profile.total_time_us as f64 * 100.0
        );
        println!(
            "  Actual encode:    {:7.1} µs ({:.1}%)",
            cpp_profile.encode_time_us as f64,
            cpp_profile.encode_time_us as f64 / cpp_profile.total_time_us as f64 * 100.0
        );
        println!(
            "  TOTAL:            {:7.1} µs\n",
            cpp_profile.total_time_us as f64
        );

        // Rust Profile
        let mut rust_mesh_setup_us = 0.0;
        let mut rust_encoder_setup_us = 0.0;
        let mut rust_encode_us = 0.0;
        let mut rust_total_us = 0.0;
        let mut rust_output_size = 0;

        for _ in 0..iterations {
            let total_start = Instant::now();

            // Mesh setup (clone since we have pre-built mesh)
            let mesh_start = Instant::now();
            let mesh_copy = mesh.clone();
            let mesh_elapsed = mesh_start.elapsed();

            // Encoder setup
            let encoder_start = Instant::now();
            let mut options = EncoderOptions::new();
            options.set_global_int("encoding_speed", speed);
            options.set_global_int("decoding_speed", speed);
            options.set_attribute_int(0, "quantization_bits", 10);

            let mut encoder = MeshEncoder::new();
            encoder.set_mesh(mesh_copy);
            let encoder_elapsed = encoder_start.elapsed();

            // Actual encoding
            let encode_start = Instant::now();
            let mut encoder_buffer = EncoderBuffer::new();
            encoder
                .encode(&options, &mut encoder_buffer)
                .expect("Rust encode failed");
            let encode_elapsed = encode_start.elapsed();

            let total_elapsed = total_start.elapsed();

            rust_mesh_setup_us += duration_to_us(mesh_elapsed);
            rust_encoder_setup_us += duration_to_us(encoder_elapsed);
            rust_encode_us += duration_to_us(encode_elapsed);
            rust_total_us += duration_to_us(total_elapsed);
            rust_output_size = encoder_buffer.data().len();
        }

        let rust_mesh_setup = rust_mesh_setup_us / f64::from(iterations);
        let rust_encoder_setup = rust_encoder_setup_us / f64::from(iterations);
        let rust_encode = rust_encode_us / f64::from(iterations);
        let rust_total = rust_total_us / f64::from(iterations);

        println!("Rust Breakdown:");
        println!(
            "  Mesh clone:       {:7.1} µs ({:.1}%)",
            rust_mesh_setup,
            rust_mesh_setup / rust_total * 100.0
        );
        println!(
            "  Encoder setup:    {:7.1} µs ({:.1}%)",
            rust_encoder_setup,
            rust_encoder_setup / rust_total * 100.0
        );
        println!(
            "  Actual encode:    {:7.1} µs ({:.1}%)",
            rust_encode,
            rust_encode / rust_total * 100.0
        );
        println!("  TOTAL:            {:7.1} µs\n", rust_total);

        // Comparison
        println!("Comparison (encode only):");
        println!(
            "  C++ encode:   {:7.1} µs",
            cpp_profile.encode_time_us as f64
        );
        println!("  Rust encode:  {:7.1} µs", rust_encode);
        println!(
            "  Speedup:      {:.2}x {}",
            cpp_profile.encode_time_us as f64 / rust_encode,
            if rust_encode < cpp_profile.encode_time_us as f64 {
                "(Rust faster)"
            } else {
                "(C++ faster)"
            }
        );

        println!("\nComparison (total with mesh setup):");
        println!(
            "  C++ total:    {:7.1} µs",
            cpp_profile.total_time_us as f64
        );
        println!("  Rust total:   {:7.1} µs", rust_total);
        println!(
            "  Speedup:      {:.2}x {}",
            cpp_profile.total_time_us as f64 / rust_total,
            if rust_total < cpp_profile.total_time_us as f64 {
                "(Rust faster)"
            } else {
                "(C++ faster)"
            }
        );

        println!(
            "\nOutput sizes: C++={} Rust={} {}\n",
            cpp_profile.output_size,
            rust_output_size,
            if rust_output_size == cpp_profile.output_size {
                "✓"
            } else {
                "✗ MISMATCH"
            }
        );
        println!("{}\n", "-".repeat(50));
    }
}

#[test]
fn profile_decode_rust_vs_cpp() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::mesh::Mesh;
    use draco_core::mesh_decoder::MeshDecoder;

    common::disable_noisy_debug_env();

    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    println!("\n=== Decode Performance: C++ vs Rust ===\n");

    let grid_size = 100;
    let (mesh, positions, faces) = create_grid_mesh(grid_size);
    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;
    let iterations = 50;

    println!(
        "Grid {}x{}: {} points, {} faces ({} iterations)\n",
        grid_size, grid_size, num_points, num_faces, iterations
    );

    for speed in 0..=10 {
        println!("=== Speed {} ===\n", speed);

        // First encode to get data to decode
        let mut options = EncoderOptions::new();
        options.set_global_int("encoding_speed", speed);
        options.set_global_int("decoding_speed", speed);
        options.set_attribute_int(0, "quantization_bits", 10);

        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut encoder_buffer = EncoderBuffer::new();
        encoder
            .encode(&options, &mut encoder_buffer)
            .expect("Encode failed");

        let encoded_data = encoder_buffer.data().to_vec();
        println!("Encoded size: {} bytes\n", encoded_data.len());

        // C++ Decode
        let cpp_result = draco_cpp_test_bridge::profile_cpp_decode(&encoded_data, iterations)
            .expect("C++ decode failed");

        println!("C++ Decode:");
        println!("  Time:       {:7.1} µs", cpp_result.decode_time_us as f64);
        println!("  Points:     {}", cpp_result.num_points);
        println!("  Faces:      {}\n", cpp_result.num_faces);

        // Rust Decode
        let mut rust_decode_us = 0.0;
        let mut rust_num_points = 0;
        let mut rust_num_faces = 0;
        let mut rust_decode_success = true;

        for iter in 0..iterations {
            let mut decoder_buffer = DecoderBuffer::new(&encoded_data);

            let mut out_mesh = Mesh::new();
            let mut decoder = MeshDecoder::new();

            let start = Instant::now();
            match decoder.decode(&mut decoder_buffer, &mut out_mesh) {
                Ok(_) => {
                    rust_decode_us += duration_to_us(start.elapsed());
                    rust_num_points = out_mesh.num_points();
                    rust_num_faces = out_mesh.num_faces();
                }
                Err(e) => {
                    if iter == 0 {
                        println!("Rust Decode: SKIPPED ({})\n", e);
                        rust_decode_success = false;
                        break;
                    }
                }
            }
        }

        if !rust_decode_success {
            println!("{}\n", "-".repeat(50));
            continue;
        }

        let rust_avg = rust_decode_us / f64::from(iterations);

        println!("Rust Decode:");
        println!("  Time:       {:7.1} µs", rust_avg);
        println!("  Points:     {}", rust_num_points);
        println!("  Faces:      {}\n", rust_num_faces);

        // Comparison
        let speedup = cpp_result.decode_time_us as f64 / rust_avg;
        println!("Comparison:");
        println!("  C++:        {:7.1} µs", cpp_result.decode_time_us as f64);
        println!("  Rust:       {:7.1} µs", rust_avg);
        println!(
            "  Speedup:    {:.2}x {}",
            speedup,
            if speedup > 1.0 {
                "(Rust faster)"
            } else {
                "(C++ faster)"
            }
        );

        let points_match = rust_num_points == cpp_result.num_points as usize;
        let faces_match = rust_num_faces == cpp_result.num_faces as usize;
        println!(
            "  Points:     {} vs {} {}",
            cpp_result.num_points,
            rust_num_points,
            if points_match { "✓" } else { "✗" }
        );
        println!(
            "  Faces:      {} vs {} {}",
            cpp_result.num_faces,
            rust_num_faces,
            if faces_match { "✓" } else { "✗" }
        );

        println!("\n{}\n", "-".repeat(50));
    }
}

#[test]
fn profile_decode_sequential_breakdown() {
    let _output_lock = OUTPUT_LOCK.lock().unwrap();
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::mesh::Mesh;
    use draco_core::mesh_decoder::MeshDecoder;

    common::disable_noisy_debug_env();

    if !draco_cpp_test_bridge::is_available() {
        eprintln!("SKIPPING: C++ test bridge not available");
        return;
    }

    println!("\n=== Sequential Decode Breakdown (Speed 10) ===\n");

    let grid_size = 100;
    let (mesh, positions, faces) = create_grid_mesh(grid_size);
    let num_points = positions.len() / 3;
    let num_faces = faces.len() / 3;
    let iterations = 50;

    println!(
        "Grid {}x{}: {} points, {} faces ({} iterations)\n",
        grid_size, grid_size, num_points, num_faces, iterations
    );

    // Encode at speed 10
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", 10);
    options.set_global_int("decoding_speed", 10);
    options.set_attribute_int(0, "quantization_bits", 10);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut encoder_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoder_buffer)
        .expect("Encode failed");

    let encoded_data = encoder_buffer.data().to_vec();
    println!("Encoded size: {} bytes\n", encoded_data.len());

    // C++ Decode
    let cpp_result = draco_cpp_test_bridge::profile_cpp_decode(&encoded_data, iterations)
        .expect("C++ decode failed");

    println!("C++ Decode:   {:7.1} µs", cpp_result.decode_time_us as f64);

    // Profile Rust decode stages
    let mut total_buffer_init = 0u128;
    let mut total_decode = 0u128;

    for _ in 0..iterations {
        // Buffer creation
        let start = Instant::now();
        let mut decoder_buffer = DecoderBuffer::new(&encoded_data);
        total_buffer_init += start.elapsed().as_nanos();

        let mut out_mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();

        // Decode
        let start = Instant::now();
        decoder
            .decode(&mut decoder_buffer, &mut out_mesh)
            .expect("Decode failed");
        total_decode += start.elapsed().as_nanos();
    }

    let buf_init_us = total_buffer_init as f64 / iterations as f64 / 1000.0;
    let decode_us = total_decode as f64 / iterations as f64 / 1000.0;

    println!("\nRust Breakdown:");
    println!("  Buffer init:    {:7.2} µs", buf_init_us);
    println!("  Decode:       {:7.1} µs", decode_us);
    println!("  TOTAL:        {:7.1} µs", buf_init_us + decode_us);

    let speedup = cpp_result.decode_time_us as f64 / decode_us;
    println!(
        "\nSpeedup: {:.2}x {}",
        speedup,
        if speedup > 1.0 {
            "(Rust faster)"
        } else {
            "(C++ faster)"
        }
    );
}
