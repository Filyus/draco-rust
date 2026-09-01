//! Randomized parity sweep against the C++ encoder.
//!
//! The rest of the parity suite pins hand-picked shapes. This generates them
//! instead, from a fixed seed, so a run is reproducible but the input space is
//! not limited to what someone thought to write down. It found six defects,
//! in growing sample sizes:
//!
//! 1. An encode-side panic on degenerate meshes carrying texture coordinates.
//! 2. A vertex reachable only through a degenerate (zero-area) face getting
//!    written an attribute value the header's vertex count never accounted
//!    for, corrupting every byte after it while `encode()` still returned
//!    `Ok`.
//! 3. A mesh whose every face is degenerate reaching code that assumes at
//!    least one encoded point and panicking, where C++ rejects the same input
//!    outright.
//! 4. The constrained-multi-parallelogram predictor's per-vertex configuration
//!    search visited candidates in bitmask order; C++ visits them by
//!    increasing parallelogram count, and within each count in
//!    `std::next_permutation` order. Both searches are exhaustive over the
//!    same set and pick a genuinely optimal configuration, so this never
//!    produced a wrong *value* -- but whenever two configurations tied on
//!    cost, "first visited wins" picked a different tied winner, and which
//!    configuration is chosen is itself part of the encoded stream (crease
//!    flags per parallelogram edge). Matching the visiting order fixed the
//!    dominant cause of divergence in the two categories below.
//! 5. A defect in **upstream C++**, not here, found by tracing the one case
//!    defect 4 left behind. `ApproximateRAnsFrequencyTableBits` took its
//!    `max_value` as `int32_t`, but the caller derives it from a symbol that
//!    can reach `UINT32_MAX` -- a zigzag-encoded residual near `INT32_MIN`,
//!    which high quantization plus a bad parallelogram average produces. The
//!    value arrived reinterpreted as negative, so upstream's estimate of a
//!    catastrophically expensive configuration came out *cheaper* than every
//!    sane one, and C++ picked it. Rust, computing the same estimate in
//!    `i64`/`u64` throughout, correctly rejected that configuration -- so the
//!    two disagreed by 22 bytes on a mesh where Rust was right. The parameter
//!    is still `int32_t` in released 1.5.7 and in upstream `main`, which have
//!    identical entropy sources, so this defect is present in whatever C++
//!    build the sweep is run against.
//! 6. The encoding half of the portable texcoord predictor was missing the
//!    three overflow guards upstream applies before its scaled-space
//!    multiplications. Upstream shares one predictor between encoder and
//!    decoder so both are guarded; this port has separate halves and only the
//!    decoding one checked. Rust therefore wrapped silently and produced a
//!    stream for non-manifold input C++ refuses to encode at all.
//!
//! With all six fixed, every well-formed, non-manifold and degenerate mesh
//! sampled in a 6000-iteration run matches C++ byte for byte, and the two
//! agree on every input either one rejects. `is_asserted` says so: below
//! upstream's maximum quantization it now carves out nothing.
//!
//! Three things are asserted, in decreasing strength:
//!
//! 1. **No panics, ever.** A library whose API returns `Result` must not abort
//!    on input it dislikes, whatever the geometry.
//! 2. **Both encoders agree on whether an input is encodable at all.**
//! 3. **Byte parity on well-formed meshes below 30-bit quantization** --
//!    including non-manifold and degenerate ones.
//!
//! One region is measured and reported rather than asserted: **30-bit
//! quantization**, upstream's own documented maximum. Prediction residuals
//! there exceed `int32` and both implementations are in overflow territory;
//! C++ Draco reads out of bounds in `ShannonEntropyTracker` on some of these
//! inputs, which is a defect of upstream's rather than behaviour to reproduce.
//! Neither it nor the `int32` defect above has been reported to upstream.
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use draco_cpp_test_bridge::CppMeshAttributes;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 16
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
    fn unit(&mut self) -> f32 {
        (self.below(1_000_000) as f32) / 1_000_000.0
    }
}

struct Case {
    kind: &'static str,
    positions: Vec<f32>,
    faces: Vec<u32>,
    normals: Option<Vec<f32>>,
    uvs: Option<Vec<f32>>,
    colors: Option<Vec<u8>>,
    position_bits: i32,
    normal_bits: i32,
    uv_bits: i32,
    color_bits: i32,
    speed: i32,
}

impl Case {
    /// Whether byte parity is a defined expectation for this input.
    ///
    /// Below upstream's maximum quantization, this covers every shape this
    /// file generates -- `soup` and `degenerate` included. At 30 bits the
    /// prediction residuals leave `int32` and both implementations overflow,
    /// which is measured separately rather than asserted here.
    fn is_asserted(&self) -> bool {
        const MAX_DEFINED_QUANTIZATION: i32 = 29;
        self.position_bits <= MAX_DEFINED_QUANTIZATION
            && (self.normals.is_none() || self.normal_bits <= MAX_DEFINED_QUANTIZATION)
            && (self.uvs.is_none() || self.uv_bits <= MAX_DEFINED_QUANTIZATION)
            && (self.colors.is_none() || self.color_bits <= MAX_DEFINED_QUANTIZATION)
    }
}

fn make_case(rng: &mut Rng) -> Case {
    let kind_roll = rng.below(100);
    let (kind, positions, faces) = if kind_roll < 55 {
        // Grid, sometimes large enough to cross the 1000-face edgebreaker
        // threshold and the bigger symbol tables that come with it.
        let w_max = if rng.chance(25) { 40 } else { 10 };
        let w = rng.range(2, w_max) as usize;
        let h_max = if rng.chance(25) { 40 } else { 10 };
        let h = rng.range(2, h_max) as usize;
        let scale: f32 = match rng.below(4) {
            0 => 1e-4,
            1 => 1e4,
            _ => 1.0,
        };
        let mut positions = Vec::new();
        for y in 0..h {
            for x in 0..w {
                positions.extend([
                    x as f32 * scale,
                    y as f32 * scale,
                    (rng.unit() - 0.5) * scale,
                ]);
            }
        }
        let mut faces = Vec::new();
        for y in 0..h - 1 {
            for x in 0..w - 1 {
                let i = (y * w + x) as u32;
                faces.extend([i, i + 1, i + w as u32]);
                faces.extend([i + 1, i + w as u32 + 1, i + w as u32]);
            }
        }
        ("grid", positions, faces)
    } else if kind_roll < 80 {
        // Random triangle soup: non-manifold, duplicated and degenerate
        // triangles all reachable.
        let n = rng.range(3, 120) as usize;
        let m = rng.range(1, 200) as usize;
        let mut positions = Vec::new();
        for _ in 0..n {
            positions.extend([
                rng.unit() * 10.0 - 5.0,
                rng.unit() * 10.0 - 5.0,
                rng.unit() * 10.0 - 5.0,
            ]);
        }
        let mut faces = Vec::new();
        for _ in 0..m {
            faces.extend([
                rng.below(n as u64) as u32,
                rng.below(n as u64) as u32,
                rng.below(n as u64) as u32,
            ]);
        }
        ("soup", positions, faces)
    } else {
        // Degenerate on purpose: collapsed vertices, collinear runs, zero-area
        // triangles.
        let n = rng.range(3, 60) as usize;
        let mut positions = Vec::new();
        for i in 0..n {
            match rng.below(3) {
                0 => positions.extend([1.0, 2.0, 3.0]),      // all identical
                1 => positions.extend([i as f32, 0.0, 0.0]), // collinear
                _ => positions.extend([rng.unit(), rng.unit(), rng.unit()]),
            }
        }
        let m = rng.range(1, 80) as usize;
        let mut faces = Vec::new();
        for _ in 0..m {
            let a = rng.below(n as u64) as u32;
            if rng.chance(30) {
                faces.extend([a, a, a]); // zero-area
            } else {
                faces.extend([a, rng.below(n as u64) as u32, rng.below(n as u64) as u32]);
            }
        }
        ("degenerate", positions, faces)
    };

    let num_points = positions.len() / 3;
    let normals = rng.chance(50).then(|| {
        (0..num_points)
            .flat_map(|_| {
                let (x, y, z) = (rng.unit() - 0.5, rng.unit() - 0.5, rng.unit() - 0.5);
                let len = (x * x + y * y + z * z).sqrt().max(1e-6);
                [x / len, y / len, z / len]
            })
            .collect::<Vec<f32>>()
    });
    let uvs = rng.chance(50).then(|| {
        (0..num_points)
            .flat_map(|_| [rng.unit(), rng.unit()])
            .collect::<Vec<f32>>()
    });
    let colors = rng.chance(50).then(|| {
        (0..num_points)
            .flat_map(|_| {
                [
                    rng.below(256) as u8,
                    rng.below(256) as u8,
                    rng.below(256) as u8,
                    255u8,
                ]
            })
            .collect::<Vec<u8>>()
    });

    Case {
        kind,
        positions,
        faces,
        normals,
        uvs,
        colors,
        position_bits: rng.range(1, 30) as i32,
        normal_bits: rng.range(1, 30) as i32,
        uv_bits: rng.range(1, 30) as i32,
        color_bits: rng.range(1, 30) as i32,
        speed: rng.below(11) as i32,
    }
}

fn add_float_attribute(
    mesh: &mut Mesh,
    kind: GeometryAttributeType,
    components: usize,
    values: &[f32],
    num_points: usize,
) {
    let mut attribute = PointAttribute::new();
    attribute.init(kind, components as u8, DataType::Float32, false, num_points);
    for (i, value) in values.iter().enumerate() {
        attribute
            .buffer_mut()
            .update(&value.to_le_bytes(), Some(i * 4));
    }
    mesh.add_attribute(attribute);
}

fn encode_rust(case: &Case) -> Option<Vec<u8>> {
    let num_points = case.positions.len() / 3;
    let num_faces = case.faces.len() / 3;
    let mut mesh = Mesh::new();
    mesh.set_num_points(num_points);
    mesh.set_num_faces(num_faces);

    add_float_attribute(
        &mut mesh,
        GeometryAttributeType::Position,
        3,
        &case.positions,
        num_points,
    );
    if let Some(v) = &case.normals {
        add_float_attribute(&mut mesh, GeometryAttributeType::Normal, 3, v, num_points);
    }
    if let Some(v) = &case.uvs {
        add_float_attribute(&mut mesh, GeometryAttributeType::TexCoord, 2, v, num_points);
    }
    if let Some(colors) = &case.colors {
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Color,
            4,
            DataType::Uint8,
            true,
            num_points,
        );
        for point in 0..num_points {
            attribute
                .buffer_mut()
                .update(&colors[point * 4..point * 4 + 4], Some(point * 4));
        }
        mesh.add_attribute(attribute);
    }
    for face in 0..num_faces {
        mesh.set_face(
            FaceIndex(face as u32),
            [
                PointIndex(case.faces[face * 3]),
                PointIndex(case.faces[face * 3 + 1]),
                PointIndex(case.faces[face * 3 + 2]),
            ],
        );
    }

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", case.speed);
    options.set_global_int("decoding_speed", case.speed);
    let mut id = 0;
    options.set_attribute_int(id, "quantization_bits", case.position_bits);
    if case.normals.is_some() {
        id += 1;
        options.set_attribute_int(id, "quantization_bits", case.normal_bits);
    }
    if case.uvs.is_some() {
        id += 1;
        options.set_attribute_int(id, "quantization_bits", case.uv_bits);
    }
    if case.colors.is_some() {
        id += 1;
        options.set_attribute_int(id, "quantization_bits", case.color_bits);
    }

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options, &mut buffer).ok()?;
    Some(buffer.data().to_vec())
}

fn encode_cpp(case: &Case) -> Option<Vec<u8>> {
    draco_cpp_test_bridge::encode_cpp_mesh_attributed(
        &case.positions,
        &case.faces,
        CppMeshAttributes {
            normals: case.normals.as_deref(),
            uvs: case.uvs.as_deref(),
            colors: case.colors.as_deref(),
            normal_bits: case.normal_bits,
            uv_bits: case.uv_bits,
            color_bits: case.color_bits,
        },
        case.speed,
        case.speed,
        case.position_bits,
    )
}

#[test]
fn random_sweep() {
    if !draco_cpp_test_bridge::is_available() {
        println!("SKIP: no C++ bridge");
        return;
    }
    let iterations: u64 = std::env::var("SWEEP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);

    // Each encode is wrapped in `catch_unwind`, and a panicking case is a
    // finding to be collected rather than a crash to be watched. Silence the
    // hook while the sweep runs so the transcript stays readable -- but restore
    // it before the assertions below, or this test would swallow its own
    // failure message and report nothing.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut rng = Rng(0x5eed_1234_abcd_0001);
    let mut compared = 0;
    let mut both_rejected = 0;
    let mut panics: Vec<String> = Vec::new();
    let mut mismatches: Vec<String> = Vec::new();
    let mut reported: Vec<String> = Vec::new();
    let mut by_kind: std::collections::BTreeMap<&str, usize> = Default::default();

    for i in 0..iterations {
        let case = make_case(&mut rng);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| encode_rust(&case)));
        let cpp = encode_cpp(&case);
        let rust = match caught {
            Ok(v) => v,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "?".into());
                panics.push(format!(
                    "#{i} {} pts={} faces={} n={} uv={} c={} qp={} speed={} cpp_ok={} :: {msg}",
                    case.kind,
                    case.positions.len() / 3,
                    case.faces.len() / 3,
                    case.normals.is_some() as u8,
                    case.uvs.is_some() as u8,
                    case.colors.is_some() as u8,
                    case.position_bits,
                    case.speed,
                    cpp.is_some(),
                ));
                continue;
            }
        };
        let label = format!(
            "#{i} {} pts={} faces={} n={} uv={} c={} qp={} qn={} qt={} qc={} speed={}",
            case.kind,
            case.positions.len() / 3,
            case.faces.len() / 3,
            case.normals.is_some() as u8,
            case.uvs.is_some() as u8,
            case.colors.is_some() as u8,
            case.position_bits,
            case.normal_bits,
            case.uv_bits,
            case.color_bits,
            case.speed
        );
        let (rust_accepted, cpp_accepted) = (rust.is_some(), cpp.is_some());
        match (rust, cpp) {
            (None, None) => both_rejected += 1,
            (None, Some(_)) | (Some(_), None) => {
                let note = format!(
                    "{label}: only one encoder accepted this input (Rust {}, C++ {})",
                    rust_accepted, cpp_accepted
                );
                if case.is_asserted() {
                    mismatches.push(note);
                } else {
                    reported.push(note);
                }
            }
            (Some(r), Some(c)) => {
                compared += 1;
                *by_kind.entry(case.kind).or_default() += 1;
                let shared = r.len().min(c.len());
                let diff = (0..shared)
                    .find(|&k| r[k] != c[k])
                    .or(if r.len() == c.len() {
                        None
                    } else {
                        Some(shared)
                    });
                if let Some(off) = diff {
                    let note = format!(
                        "{label}: C++ {} bytes, Rust {} bytes, first difference at {off}",
                        c.len(),
                        r.len()
                    );
                    if case.is_asserted() {
                        mismatches.push(note);
                    } else {
                        reported.push(note);
                    }
                }
            }
        }
    }

    std::panic::set_hook(default_hook);

    println!("compared {compared}, both rejected {both_rejected}, by kind {by_kind:?}");
    if !reported.is_empty() {
        println!(
            "known open, not asserted ({} cases, all at 30-bit quantization):",
            reported.len()
        );
        for note in reported.iter().take(5) {
            println!("  {note}");
        }
    }

    assert!(
        panics.is_empty(),
        "the encoder panicked instead of returning an error on {} input(s):
{}",
        panics.len(),
        panics.join(
            "
"
        )
    );
    assert!(
        mismatches.is_empty(),
        "encoder output differs from C++ Draco on well-formed meshes:
{}",
        mismatches.join(
            "
"
        )
    );
    // A sweep that silently stopped generating the well-formed meshes it
    // asserts on would pass while checking nothing.
    assert!(
        compared > iterations as usize / 4,
        "expected most cases to reach a byte comparison, got {compared} of {iterations}"
    );
}
