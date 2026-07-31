//! Encode-then-decode timing on real models, C++ Draco vs this port.
//!
//! Every mesh here comes from an actual asset rather than a generator, and the
//! stream that gets decoded is one this benchmark produced a moment earlier at
//! Draco's default settings. That matters: the `.drc` files carried in testdata
//! were written by assorted encoder versions at assorted settings, so timing a
//! decode against them measures the fixtures as much as the decoder. Encoding
//! first pins the settings, and lets the same run report compression as well.
//!
//! ```sh
//! cargo test --manifest-path crates/Cargo.toml -p draco-cpp-test-bridge \
//!   --test bench_real_models --release -- --nocapture
//! ```
mod common;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use std::path::PathBuf;
use std::time::Instant;

/// Draco's own default speed, called out in the output so the numbers a caller
/// gets without tuning anything are easy to find in the matrix.
const DEFAULT_SPEED: i32 = 5;
const QUANTIZATION: i32 = 10;

fn testdata_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("testdata"))
        .expect("testdata directory")
}

struct Model {
    label: &'static str,
    path: &'static [&'static str],
}

/// Real assets, largest first, and every one of them tracked in the repository
/// so this benchmark reproduces for anyone who clones it. Tiny fixtures are
/// deliberately absent: below a few thousand faces the timings are dominated by
/// fixed setup and say nothing about either implementation's geometry work.
const MODELS: &[Model] = &[
    Model {
        label: "Stanford bunny",
        path: &["bun_zipper.ply"],
    },
    Model {
        label: "bunny (drc)",
        path: &["bunny_cpp_standard.drc"],
    },
    Model {
        label: "car",
        path: &["car.drc"],
    },
    Model {
        label: "lamp",
        path: &["lamp_cpp_std.drc"],
    },
];

fn load_mesh(path: &PathBuf) -> Option<Mesh> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "obj" => draco_io::obj_reader::ObjReader::open(path)
            .ok()?
            .read_mesh()
            .ok(),
        "ply" => draco_io::ply_reader::PlyReader::open(path)
            .ok()?
            .read_mesh()
            .ok(),
        "drc" => {
            let bytes = std::fs::read(path).ok()?;
            let mut buffer = DecoderBuffer::new(&bytes);
            let mut mesh = Mesh::new();
            MeshDecoder::new().decode(&mut buffer, &mut mesh).ok()?;
            Some(mesh)
        }
        _ => None,
    }
}

fn options(speed: i32) -> EncoderOptions {
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);
    options.set_attribute_int(0, "quantization_bits", QUANTIZATION);
    options
}

fn encode_rust(mesh: &Mesh, speed: i32) -> Option<Vec<u8>> {
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut buffer = EncoderBuffer::new();
    encoder.encode(&options(speed), &mut buffer).ok()?;
    Some(buffer.data().to_vec())
}

/// Median of `iters` runs. A median rather than a mean because one descheduled
/// run would otherwise set the number, and it is the typical run we care about.
fn median_us(iters: u32, mut run: impl FnMut()) -> f64 {
    let samples: Vec<f64> = (0..iters)
        .map(|_| {
            let t = Instant::now();
            run();
            t.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    median(samples)
}

/// Median where the closure reports its own timed region, so setup the other
/// side does not pay for can be left out of the measurement.
fn median_us_inner(iters: u32, mut run: impl FnMut() -> f64) -> f64 {
    median((0..iters).map(|_| run()).collect())
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

/// Median of what the C++ side reports for itself.
///
/// Wall-clocking the bridge call instead would charge C++ for crossing the FFI
/// and, in the encode case, for decoding the input stream to get a mesh to
/// encode -- work the Rust side is handed for free here. Both profilers already
/// time just the codec call and average over their own iterations, so taking
/// their number is what makes the two columns comparable.
fn median_reported_us(iters: u32, mut run: impl FnMut() -> Option<i64>) -> Option<f64> {
    let samples: Vec<f64> = (0..iters).filter_map(|_| run()).map(|v| v as f64).collect();
    if samples.is_empty() {
        return None;
    }
    Some(median(samples))
}

#[test]
fn bench_real_models() {
    common::disable_noisy_debug_env();
    if common::skip_if_cpp_bridge_unavailable() {
        return;
    }

    println!("\n=== Real Models: compress, then decompress ===\n");
    println!(
        "{QUANTIZATION}-bit position quantization, median of repeated runs. \
         Speed {DEFAULT_SPEED} is Draco's default.\n"
    );

    let base = testdata_dir();
    let mut measured = 0;

    for model in MODELS {
        let mut path = base.clone();
        for segment in model.path {
            path = path.join(segment);
        }
        let Some(mesh) = load_mesh(&path) else {
            println!("{}: unreadable, skipped\n", model.label);
            continue;
        };
        let faces = mesh.num_faces();
        if faces == 0 {
            continue;
        }

        println!(
            "{} -- {} faces, {} points",
            model.label,
            faces,
            mesh.num_points()
        );
        println!(
            "{:>6}{:>11}{:>11}{:>9}{:>11}{:>11}{:>9}{:>11}",
            "speed", "enc C++", "enc Rust", "ratio", "dec C++", "dec Rust", "ratio", "size"
        );

        // Repeat count scales with mesh size: enough runs for a stable median
        // on the small models without the big ones taking minutes.
        let iters = if faces > 30_000 { 5 } else { 21 };

        for speed in 0..=10 {
            // Compress once up front at this speed. Both implementations are
            // byte-identical, so the decoders are handed the same stream.
            let Some(stream) = encode_rust(&mesh, speed) else {
                println!("{speed:>6}   rust encode failed");
                continue;
            };
            if draco_cpp_test_bridge::profile_cpp_reencode_mesh(
                &stream,
                speed,
                speed,
                QUANTIZATION,
                1,
            )
            .is_none()
            {
                println!("{speed:>6}   cpp encode failed");
                continue;
            }

            // Rust: time the codec call only. The mesh is cloned outside the
            // timed region for the same reason the C++ decode happens outside
            // its own -- neither is the work being compared.
            let opts = options(speed);
            let enc_rust = median_us_inner(iters, || {
                let mut encoder = MeshEncoder::new();
                encoder.set_mesh(mesh.clone());
                let mut buffer = EncoderBuffer::new();
                let t = Instant::now();
                let _ = encoder.encode(&opts, &mut buffer);
                t.elapsed().as_secs_f64() * 1e6
            });
            let dec_rust = median_us(iters, || {
                let mut buffer = DecoderBuffer::new(&stream);
                let mut out = Mesh::new();
                let _ = MeshDecoder::new().decode(&mut buffer, &mut out);
            });

            let Some(enc_cpp) = median_reported_us(iters, || {
                draco_cpp_test_bridge::profile_cpp_reencode_mesh(
                    &stream,
                    speed,
                    speed,
                    QUANTIZATION,
                    1,
                )
                .map(|r| r.encode_time_us)
            }) else {
                println!("{speed:>6}   cpp encode profile unavailable");
                continue;
            };
            let Some(dec_cpp) = median_reported_us(iters, || {
                draco_cpp_test_bridge::profile_cpp_decode(&stream, 1).map(|r| r.decode_time_us)
            }) else {
                println!("{speed:>6}   cpp decode profile unavailable");
                continue;
            };

            let mark = if speed == DEFAULT_SPEED {
                " <- default"
            } else {
                ""
            };
            println!(
                "{:>6}{:>10.2}ms{:>10.2}ms{:>8.1}x{:>10.2}ms{:>10.2}ms{:>8.1}x{:>10}B{}",
                speed,
                enc_cpp / 1000.0,
                enc_rust / 1000.0,
                enc_cpp / enc_rust,
                dec_cpp / 1000.0,
                dec_rust / 1000.0,
                dec_cpp / dec_rust,
                stream.len(),
                mark,
            );
            measured += 1;
        }
        println!();
    }

    assert!(measured > 0, "no real model could be measured");
}
