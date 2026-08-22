//! Every payload, every speed, both sides -- in one process, one build.
//!
//! `encode_loop` takes one mesh, one side and one speed per process, so a
//! per-family table costs a process per cell and the two sides never see the
//! same machine conditions. That matters here: the C++ side has been measured
//! moving 10% between runs of one binary on one payload, which is enough on
//! its own to invent or erase the gaps this comparison is looking for.
//!
//! This runs the whole matrix in one process, interleaving the sides within
//! each round so a drift that hits one side hits the other, and reports the
//! median of the rounds with the spread beside it. A cell whose spread is
//! wider than the gap it claims is a cell that resolved nothing, and printing
//! both is what makes that visible rather than assumed.
//!
//! The bridge is in-process, so "interleaved" here means adjacent calls rather
//! than adjacent processes.
//!
//! ```text
//! cargo run --release --example encode_matrix -- <rounds> <speeds> <mesh.obj>...
//! cargo run --release --example encode_matrix -- 3 0,5,10 seeded_*.obj
//! ```
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;

/// One payload, parsed once and held in both sides' input forms so neither
/// pays a conversion the other does not.
struct Payload {
    name: String,
    mesh: Mesh,
    positions: Vec<f32>,
    faces: Vec<u32>,
}

fn load(path: &str) -> Payload {
    let bytes = std::fs::read(path).expect("read mesh");
    let mesh = draco_io::obj_reader::ObjReader::read_from_bytes(&bytes).expect("parse obj");
    let positions: Vec<f32> = mesh.attribute(0).read_f32s(mesh.num_points(), 3);
    let faces: Vec<u32> = (0..mesh.num_faces())
        .flat_map(|f| {
            let face = mesh.face(FaceIndex(f as u32));
            [face[0].0, face[1].0, face[2].0]
        })
        .collect();
    let name = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    Payload {
        name,
        mesh,
        positions,
        faces,
    }
}

fn options_for(mesh: &Mesh, speed: i32) -> EncoderOptions {
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);
    options.set_attribute_int(0, "quantization_bits", 11);
    if mesh.num_attributes() > 1 {
        options.set_attribute_int(1, "quantization_bits", 8);
    }
    options
}

fn rust_encode_us(mesh: &Mesh, options: &EncoderOptions, iters: u32) -> (f64, usize) {
    let mut total = 0.0;
    let mut size = 0;
    for _ in 0..iters {
        // The mesh clone is setup, not encode, and is charged outside the
        // timed region so the two sides time the same thing.
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut buffer = EncoderBuffer::new();

        let start = std::time::Instant::now();
        encoder
            .encode(options, &mut buffer)
            .expect("rust encode failed");
        total += start.elapsed().as_secs_f64() * 1e6;
        size = buffer.data().len();
    }
    (total / iters as f64, size)
}

/// Median, and the spread as a percentage of it -- the number that says
/// whether the median means anything.
fn median_and_spread(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    let median = samples[samples.len() / 2];
    let spread = if median > 0.0 {
        (samples[samples.len() - 1] - samples[0]) / median * 100.0
    } else {
        0.0
    };
    (median, spread)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let rounds: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(3);
    let speeds: Vec<i32> = args
        .next()
        .expect("speeds, comma separated")
        .split(',')
        .map(|s| s.trim().parse().expect("speed"))
        .collect();
    let paths: Vec<String> = args.collect();
    assert!(!paths.is_empty(), "at least one mesh path");
    let iters: u32 = std::env::var("ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    let payloads: Vec<Payload> = paths.iter().map(|p| load(p)).collect();

    // [payload][speed] -> one sample per round, per side.
    let mut cpp = vec![vec![Vec::new(); speeds.len()]; payloads.len()];
    let mut rust = vec![vec![Vec::new(); speeds.len()]; payloads.len()];
    let mut sizes = vec![vec![(0usize, 0usize); speeds.len()]; payloads.len()];

    for round in 0..rounds {
        eprintln!("round {}/{rounds}", round + 1);
        for (p, payload) in payloads.iter().enumerate() {
            for (s, &speed) in speeds.iter().enumerate() {
                let result = draco_cpp_test_bridge::profile_cpp_encode(
                    &payload.positions,
                    &payload.faces,
                    speed,
                    speed,
                    11,
                    iters,
                )
                .expect("C++ encode failed");
                cpp[p][s].push(result.encode_time_us as f64);

                let options = options_for(&payload.mesh, speed);
                let (us, size) = rust_encode_us(&payload.mesh, &options, iters);
                rust[p][s].push(us);
                sizes[p][s] = (result.output_size as usize, size);
            }
        }
    }

    println!(
        "\n{:<20} {:>5} {:>12} {:>7} {:>12} {:>7} {:>8}  bytes",
        "payload", "speed", "cpp us", "spread", "rust us", "spread", "ratio"
    );
    for (p, payload) in payloads.iter().enumerate() {
        for (s, &speed) in speeds.iter().enumerate() {
            let (cpp_us, cpp_spread) = median_and_spread(&mut cpp[p][s]);
            let (rust_us, rust_spread) = median_and_spread(&mut rust[p][s]);
            let (cpp_bytes, rust_bytes) = sizes[p][s];
            println!(
                "{:<20} {speed:>5} {cpp_us:>12.0} {cpp_spread:>6.1}% {rust_us:>12.0} \
                 {rust_spread:>6.1}% {:>7.2}x  {cpp_bytes}/{rust_bytes}{}",
                payload.name,
                cpp_us / rust_us,
                if cpp_bytes == rust_bytes {
                    ""
                } else {
                    "  SIZES DIFFER"
                }
            );
        }
    }
}
