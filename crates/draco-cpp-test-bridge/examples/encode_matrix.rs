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
//! Three opt-in emitters ride along, so a diagnostic question does not cost
//! its own sweep. `STAGES=1` splits `CornerTable::init` per payload (the
//! table is built once per encode, so this is speed-independent and prints
//! one row per payload). `ALLOC=1` adds allocations and bytes per encode.
//! `SAMPLE_ALLOC=1` additionally prints a backtrace per allocation of 64 KB
//! or more, from a single encode of the first payload;
//! `SAMPLE_ALLOC_MIN`/`SAMPLE_ALLOC_MAX` move that window -- `MIN=0` reaches
//! the small allocations a per-call cost is made of -- and
//! `SAMPLE_ALLOC_LIMIT` raises the 64-backtrace budget, which a fixed per-call
//! count wants above that count so the reading covers all of it.
//!
//! The counting allocator is always installed -- one relaxed atomic per
//! allocation, far below this benchmark's spread -- but capturing backtraces
//! is not cheap, so `SAMPLE_ALLOC` runs outside the timed rounds.
//!
//! ```text
//! cargo run --release --example encode_matrix -- <rounds> <speeds> <mesh.obj>...
//! cargo run --release --example encode_matrix -- 3 0,5,10 seeded_*.obj
//! STAGES=1 ALLOC=1 cargo run --release --example encode_matrix -- 3 5 seeded_fan_0.obj
//! ```
use draco_core::corner_table::{CornerTable, InitStage};
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;
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

/// The `init` stage split for one payload, medianed over `iters` builds.
///
/// The faces are the ones `MeshEncoder` hands the table -- position attribute
/// value indices -- so this builds the same table the encode does.
fn stage_split(payload: &Payload, iters: u32) -> [f64; InitStage::COUNT] {
    let pos = payload.mesh.attribute(0);
    let faces: Vec<[draco_core::geometry_indices::VertexIndex; 3]> = (0..payload.mesh.num_faces())
        .map(|f| {
            let face = payload.mesh.face(FaceIndex(f as u32));
            [
                draco_core::geometry_indices::VertexIndex(pos.mapped_index(face[0]).0),
                draco_core::geometry_indices::VertexIndex(pos.mapped_index(face[1]).0),
                draco_core::geometry_indices::VertexIndex(pos.mapped_index(face[2]).0),
            ]
        })
        .collect();

    let mut samples = vec![[0.0f64; InitStage::COUNT]; iters as usize];
    for sample in samples.iter_mut() {
        let mut table = CornerTable::new(faces.len());
        assert!(table.init_with_stage_timings(&faces, sample));
    }

    let mut out = [0.0; InitStage::COUNT];
    for (stage, slot) in out.iter_mut().enumerate() {
        let mut column: Vec<f64> = samples.iter().map(|s| s[stage]).collect();
        *slot = median_and_spread(&mut column).0;
    }
    out
}

/// One cell's Rust side: mean encode time, output size, and allocations per
/// encode.
fn rust_encode_us(mesh: &Mesh, options: &EncoderOptions, iters: u32) -> (f64, usize, f64, f64) {
    let mut total = 0.0;
    let mut size = 0;
    counting::reset();
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
    let (count, bytes, _) = counting::totals();
    (
        total / iters as f64,
        size,
        count as f64 / iters as f64,
        bytes as f64 / iters as f64,
    )
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
    let mut allocs = vec![vec![(0.0f64, 0.0f64); speeds.len()]; payloads.len()];

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
                let (us, size, count, bytes) = rust_encode_us(&payload.mesh, &options, iters);
                rust[p][s].push(us);
                sizes[p][s] = (result.output_size as usize, size);
                allocs[p][s] = (count, bytes);
            }
        }
    }

    if std::env::var("STAGES").is_ok() {
        // The table is built once per encode, so this split is a property of
        // the payload and not of the speed.
        println!("\nCornerTable::init, per payload\n");
        print!("{:<20} {:>10}", "payload", "total us");
        for stage in InitStage::ALL {
            print!("{:>19}", stage.name());
        }
        println!();
        for payload in &payloads {
            let split = stage_split(payload, iters);
            let total: f64 = split.iter().sum();
            print!("{:<20} {total:>10.0}", payload.name);
            for us in split {
                let share = if total > 0.0 { us / total * 100.0 } else { 0.0 };
                print!("{us:>12.0} {share:>5.1}%");
            }
            println!();
        }
    }

    let want_alloc = std::env::var("ALLOC").is_ok();
    let alloc_header = if want_alloc {
        format!("{:>9}{:>10}", "allocs", "alloc KB")
    } else {
        String::new()
    };
    println!(
        "\n{:<20} {:>5} {:>12} {:>7} {:>12} {:>7} {:>8}{alloc_header}  bytes",
        "payload", "speed", "cpp us", "spread", "rust us", "spread", "ratio"
    );
    for (p, payload) in payloads.iter().enumerate() {
        for (s, &speed) in speeds.iter().enumerate() {
            let (cpp_us, cpp_spread) = median_and_spread(&mut cpp[p][s]);
            let (rust_us, rust_spread) = median_and_spread(&mut rust[p][s]);
            let (cpp_bytes, rust_bytes) = sizes[p][s];
            let (count, alloc_bytes) = allocs[p][s];
            println!(
                "{:<20} {speed:>5} {cpp_us:>12.0} {cpp_spread:>6.1}% {rust_us:>12.0} \
                 {rust_spread:>6.1}% {:>7.2}x{}  {cpp_bytes}/{rust_bytes}{}",
                payload.name,
                cpp_us / rust_us,
                if want_alloc {
                    format!("{count:>9.0}{:>10.0}", alloc_bytes / 1024.0)
                } else {
                    String::new()
                },
                if cpp_bytes == rust_bytes {
                    ""
                } else {
                    "  SIZES DIFFER"
                }
            );
        }
    }

    #[cfg(feature = "count_table_loads")]
    {
        use draco_core::corner_table::table_loads::{self, Accessor};

        // Loads, not calls: this port fuses `Opposite(Next(c))` into one
        // lookup where C++ makes three calls, so only the load is the same
        // event on both sides. One cold encode per payload at the first speed
        // given -- the counts are exact, so repeating adds nothing.
        println!(
            "
CornerTable array loads, one encode at speed {}
",
            speeds[0]
        );
        print!("{:<20}", "payload");
        for accessor in Accessor::ALL {
            print!("{:>17}", accessor.name());
        }
        println!();
        for payload in &payloads {
            let options = options_for(&payload.mesh, speeds[0]);
            table_loads::reset();
            rust_encode_us(&payload.mesh, &options, 1);
            print!("{:<20}", payload.name);
            for accessor in Accessor::ALL {
                print!("{:>17}", table_loads::count(*accessor));
            }
            println!();
        }
    }

    if std::env::var("SAMPLE_ALLOC").is_ok() {
        // Outside the timed rounds: a backtrace per large allocation costs far
        // more than the encode it describes.
        use std::sync::atomic::Ordering::Relaxed;
        let payload = &payloads[0];
        let options = options_for(&payload.mesh, speeds[0]);
        if let Ok(min) = std::env::var("SAMPLE_ALLOC_MIN") {
            counting::SAMPLE_MIN.store(min.parse().expect("SAMPLE_ALLOC_MIN"), Relaxed);
        }
        if let Ok(max) = std::env::var("SAMPLE_ALLOC_MAX") {
            counting::SAMPLE_MAX.store(max.parse().expect("SAMPLE_ALLOC_MAX"), Relaxed);
        }
        // The default keeps a per-element allocation from capturing one
        // backtrace per element. A fixed per-call count is the opposite case:
        // it is bounded and the whole of it is the question, so raise this to
        // above that count rather than reading the first 64 of it.
        if let Ok(limit) = std::env::var("SAMPLE_ALLOC_LIMIT") {
            counting::SAMPLE_LIMIT.store(limit.parse().expect("SAMPLE_ALLOC_LIMIT"), Relaxed);
        }
        counting::reset();
        counting::SAMPLING.store(true, Relaxed);
        rust_encode_us(&payload.mesh, &options, 1);
        counting::SAMPLING.store(false, Relaxed);
        let samples = counting::SAMPLES.lock().expect("samples").clone();
        let (total, _, _) = counting::totals();
        println!(
            "\n{} of {total} allocations of {} bytes or more sampled, \
             one encode of {} at speed {}\n",
            samples.len(),
            counting::SAMPLE_MIN.load(Relaxed),
            payload.name,
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
