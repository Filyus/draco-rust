//! The decode side of `encode_matrix`: every payload, every speed, both
//! sides, one process.
//!
//! Decode's stage attribution in `PERFORMANCE.md` predates three landed
//! rounds -- the corner-table access refactor, the dead traversal, the doubled
//! consistency scan -- and has been marked history ever since, because
//! re-taking it meant a process per cell. It does not any more.
//!
//! Each cell encodes once with the Rust encoder at that speed, then decodes
//! the same bytes on both sides, interleaved within each round so a drift that
//! hits one side hits the other. Point and face counts are compared per cell,
//! so a run that decoded two different meshes says so rather than quietly
//! reporting a ratio.
//!
//! `ALLOC=1` adds allocations and bytes per decode. `SAMPLE_ALLOC=1` prints
//! backtraces for the first payload's decode, narrowed by `SAMPLE_ALLOC_MIN`
//! and `SAMPLE_ALLOC_MAX` -- reach for the size histogram it prints first,
//! because a count that scales with the mesh is one size repeated and the
//! backtrace budget is spent before the repetition starts.
//!
//! ```text
//! cargo run --release --example decode_matrix -- <rounds> <speeds> <mesh.obj>...
//! ITERS=40 cargo run --release --example decode_matrix -- 5 0,5,10 seeded_grid_0.obj
//! ```
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_cpp_test_bridge::counting;

#[path = "common/mod.rs"]
mod common;
use common::{load, median_and_spread, options_for, Payload};

// The counting allocator wraps whichever allocator the build selected, so a
// mimalloc run still reports allocation counts. `--features mimalloc` is how
// the "is the gap the allocator" question gets asked without touching
// `draco-core`.
#[cfg(not(feature = "mimalloc"))]
#[global_allocator]
static ALLOC: counting::Counting<std::alloc::System> = counting::Counting(std::alloc::System);

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOC: counting::Counting<mimalloc::MiMalloc> = counting::Counting(mimalloc::MiMalloc);

/// The bytes both sides decode, produced by the Rust encoder at `speed`.
fn encode(payload: &Payload, speed: i32) -> Vec<u8> {
    let options = options_for(&payload.mesh, speed);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(payload.mesh.clone());
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut buffer)
        .expect("rust encode failed");
    buffer.data().to_vec()
}

/// One cell's Rust side: mean decode time, what came out, and allocations per
/// decode.
fn rust_decode_us(encoded: &[u8], iters: u32) -> (f64, (usize, usize), f64, f64) {
    let mut total = 0.0;
    let mut shape = (0, 0);
    counting::reset();
    for _ in 0..iters {
        let mut buffer = DecoderBuffer::new(encoded);
        let mut mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();

        let start = std::time::Instant::now();
        decoder
            .decode(&mut buffer, &mut mesh)
            .expect("rust decode failed");
        total += start.elapsed().as_secs_f64() * 1e6;
        shape = (mesh.num_points(), mesh.num_faces());
    }
    let (count, bytes, _) = counting::totals();
    (
        total / iters as f64,
        shape,
        count as f64 / iters as f64,
        bytes as f64 / iters as f64,
    )
}

/// Prints the per-decode phase split when `DECODE_PHASES=1` -- see
/// `draco_core::decode_phase_probe`. `setup`/`values`/`mapfix` are subsets of
/// `attrs`, not siblings.
fn report_phases(label: &str, speed: i32, iters: u32) {
    if !draco_core::decode_phase_probe::enabled() {
        return;
    }
    let nanos = draco_core::decode_phase_probe::take();
    let line: Vec<String> = draco_core::decode_phase_probe::PHASE_NAMES
        .iter()
        .zip(nanos)
        .map(|(name, n)| format!("{name} {:.0}", n as f64 / 1000.0 / iters as f64))
        .collect();
    eprintln!(
        "PHASES {label} speed {speed}: {} us/decode",
        line.join(", ")
    );
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

    // Encoded once, outside the rounds: the payload both sides decode has to
    // be the same bytes every round, not re-encoded between them.
    let encoded: Vec<Vec<Vec<u8>>> = payloads
        .iter()
        .map(|payload| speeds.iter().map(|&s| encode(payload, s)).collect())
        .collect();

    let mut cpp = vec![vec![Vec::new(); speeds.len()]; payloads.len()];
    let mut rust = vec![vec![Vec::new(); speeds.len()]; payloads.len()];
    let mut shapes = vec![vec![((0u32, 0u32), (0usize, 0usize)); speeds.len()]; payloads.len()];
    let mut allocs = vec![vec![(0.0f64, 0.0f64); speeds.len()]; payloads.len()];

    for round in 0..rounds {
        eprintln!("round {}/{rounds}", round + 1);
        for (p, payload) in payloads.iter().enumerate() {
            let _ = payload;
            for s in 0..speeds.len() {
                let bytes = &encoded[p][s];
                let (cpp_allocs_before, cpp_bytes_before) =
                    draco_cpp_test_bridge::cpp_alloc_counters();
                let result = draco_cpp_test_bridge::profile_cpp_decode(bytes, iters)
                    .expect("C++ decode failed");
                cpp[p][s].push(result.decode_time_us as f64);
                let (cpp_allocs_after, cpp_bytes_after) =
                    draco_cpp_test_bridge::cpp_alloc_counters();
                // Nonzero only in a DRACO_BRIDGE_COUNT_ALLOCS build; a
                // counting binary is for counting, so the print rides on it.
                if cpp_allocs_after > cpp_allocs_before {
                    eprintln!(
                        "CPP_ALLOC {} speed {}: {:.0} allocs, {:.0} KB per decode",
                        paths[p],
                        speeds[s],
                        (cpp_allocs_after - cpp_allocs_before) as f64 / iters as f64,
                        (cpp_bytes_after - cpp_bytes_before) as f64 / iters as f64 / 1024.0
                    );
                }

                let (us, shape, count, alloc_bytes) = rust_decode_us(bytes, iters);
                report_phases(&paths[p], speeds[s], iters);
                rust[p][s].push(us);
                shapes[p][s] = ((result.num_points, result.num_faces), shape);
                allocs[p][s] = (count, alloc_bytes);
            }
        }
    }

    let want_alloc = std::env::var("ALLOC").is_ok();
    let alloc_header = if want_alloc {
        format!("{:>9}{:>10}", "allocs", "alloc KB")
    } else {
        String::new()
    };
    println!(
        "\n{:<20} {:>5} {:>12} {:>7} {:>12} {:>7} {:>8}{alloc_header}  points/faces",
        "payload", "speed", "cpp us", "spread", "rust us", "spread", "ratio"
    );
    for (p, payload) in payloads.iter().enumerate() {
        for (s, &speed) in speeds.iter().enumerate() {
            let (cpp_us, cpp_spread) = median_and_spread(&mut cpp[p][s]);
            let (rust_us, rust_spread) = median_and_spread(&mut rust[p][s]);
            let ((cpp_points, cpp_faces), (rust_points, rust_faces)) = shapes[p][s];
            let (count, alloc_bytes) = allocs[p][s];
            let agrees = cpp_points as usize == rust_points && cpp_faces as usize == rust_faces;
            println!(
                "{:<20} {speed:>5} {cpp_us:>12.0} {cpp_spread:>6.1}% {rust_us:>12.0} \
                 {rust_spread:>6.1}% {:>7.2}x{}  {rust_points}/{rust_faces}{}",
                payload.name,
                cpp_us / rust_us,
                if want_alloc {
                    format!("{count:>9.0}{:>10.0}", alloc_bytes / 1024.0)
                } else {
                    String::new()
                },
                if agrees {
                    ""
                } else {
                    "  DECODED SHAPES DIFFER"
                }
            );
        }
    }

    #[cfg(feature = "count_table_loads")]
    {
        use draco_core::corner_table::table_loads::{self, Accessor};

        // Loads, not calls: this port fuses `Opposite(Next(c))` into one
        // lookup where C++ makes three calls. One cold decode per payload at
        // the first speed given -- the counts are exact.
        println!(
            "
CornerTable array loads, one decode at speed {}
",
            speeds[0]
        );
        print!("{:<20}", "payload");
        for accessor in Accessor::ALL {
            print!("{:>17}", accessor.name());
        }
        println!();
        for (p, payload) in payloads.iter().enumerate() {
            table_loads::reset();
            rust_decode_us(&encoded[p][0], 1);
            print!("{:<20}", payload.name);
            for accessor in Accessor::ALL {
                print!("{:>17}", table_loads::count(*accessor));
            }
            println!();
        }
    }

    if std::env::var("SAMPLE_ALLOC").is_ok() {
        // Outside the timed rounds: a backtrace per allocation costs far more
        // than the decode it describes.
        use std::sync::atomic::Ordering::Relaxed;
        if let Ok(min) = std::env::var("SAMPLE_ALLOC_MIN") {
            counting::SAMPLE_MIN.store(min.parse().expect("SAMPLE_ALLOC_MIN"), Relaxed);
        }
        if let Ok(max) = std::env::var("SAMPLE_ALLOC_MAX") {
            counting::SAMPLE_MAX.store(max.parse().expect("SAMPLE_ALLOC_MAX"), Relaxed);
        }
        counting::reset();
        counting::SAMPLING.store(true, Relaxed);
        rust_decode_us(&encoded[0][0], 1);
        counting::SAMPLING.store(false, Relaxed);
        let samples = counting::SAMPLES.lock().expect("samples").clone();
        let (total, _, _) = counting::totals();
        println!(
            "\n{} of {total} allocations of {} bytes or more sampled, \
             one decode of {} at speed {}\n",
            samples.len(),
            counting::SAMPLE_MIN.load(Relaxed),
            payloads[0].name,
            speeds[0]
        );
        println!("{:>12}  {:>10}", "size", "count");
        for (size, count) in counting::sizes_by_count().into_iter().take(12) {
            println!("{size:>12}  {count:>10}");
        }
        println!();
        for sample in samples {
            println!("=== {sample}");
        }
    }
}
