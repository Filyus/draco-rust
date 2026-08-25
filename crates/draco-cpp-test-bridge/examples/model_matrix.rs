//! Compress and decompress timings for named models, both implementations, one
//! process, interleaved.
//!
//! The timed regions are matched to the C++ shim's on purpose, because that is
//! where a comparison like this usually goes wrong. `draco_profile_reencode_mesh`
//! brackets `EncodeMeshToBuffer` alone -- the encoder, its options and the output
//! buffer are constructed outside the clock -- and `draco_profile_decode`
//! brackets `DecodeMeshFromBuffer` alone. The Rust side below does the same,
//! which in particular means the `Mesh` clone that `set_mesh` needs is paid
//! outside the timer: C++ hands its encoder a `const Mesh&` and never copies,
//! so timing the copy here would charge one side for work the other does not do.
//!
//! Both sides start from the same `.drc`: C++ decodes it internally as setup,
//! and the mesh that decode produces is what this hands the Rust encoder. The
//! decode column then times both on the same bytes.
//!
//! ```text
//! DRACO_CPP_BUILD_DIR=... cargo run --release -p draco-cpp-test-bridge \
//!   --example model_matrix -- <rounds> <iters> <speed> <qp> <name=file.drc|file.obj>...
//! ```
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;

fn options(mesh: &Mesh, speed: i32, qp: i32) -> EncoderOptions {
    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", speed);
    options.set_global_int("decoding_speed", speed);
    // The *position* attribute, by name rather than by index, which is what the
    // C++ shim's `SetAttributeQuantization(POSITION, ...)` means. Attribute 0 is
    // not always position -- in `car.drc` it is the normal -- and quantizing the
    // wrong one is a difference between the two sides rather than a setting.
    // Anything but position is left unquantized on both sides.
    let position =
        mesh.named_attribute_id(draco_core::geometry_attribute::GeometryAttributeType::Position);
    assert!(position >= 0, "the model has no position attribute");
    options.set_attribute_int(position, "quantization_bits", qp);
    options
}

fn decode_mesh(bytes: &[u8]) -> Mesh {
    let mut buffer = DecoderBuffer::new(bytes);
    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut buffer, &mut mesh)
        .expect("decode");
    mesh
}

/// One `.drc` for the model, whatever it arrived as.
fn source_stream(path: &str, speed: i32, qp: i32) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    if path.ends_with(".drc") {
        return bytes;
    }
    let mesh = draco_io::obj_reader::ObjReader::read_from_bytes(&bytes).expect("parse obj");
    let encoder_options = options(&mesh, speed, qp);
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut buffer = EncoderBuffer::new();
    encoder
        .encode(&encoder_options, &mut buffer)
        .expect("encode obj");
    buffer.data().to_vec()
}

fn rust_encode_us(mesh: &Mesh, options: &EncoderOptions, iters: u32) -> (f64, Vec<u8>) {
    let mut total = 0f64;
    let mut out = Vec::new();
    for _ in 0..iters {
        // Setup, outside the clock, mirroring the shim.
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh.clone());
        let mut buffer = EncoderBuffer::new();
        let start = std::time::Instant::now();
        encoder.encode(options, &mut buffer).expect("encode");
        total += start.elapsed().as_secs_f64();
        out = buffer.data().to_vec();
    }
    (total * 1e6 / f64::from(iters), out)
}

fn rust_decode_us(stream: &[u8], iters: u32) -> f64 {
    let mut total = 0f64;
    for _ in 0..iters {
        let mut buffer = DecoderBuffer::new(stream);
        let mut mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();
        let start = std::time::Instant::now();
        decoder.decode(&mut buffer, &mut mesh).expect("decode");
        total += start.elapsed().as_secs_f64();
    }
    total * 1e6 / f64::from(iters)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    values[values.len() / 2]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let rounds: u32 = args.next().expect("rounds").parse().expect("rounds");
    let iters: u32 = args.next().expect("iters").parse().expect("iters");
    let speed: i32 = args.next().expect("speed").parse().expect("speed");
    let qp: i32 = args.next().expect("qp").parse().expect("qp");
    let models: Vec<String> = args.collect();
    assert!(!models.is_empty(), "give at least one name=path");

    println!("speed {speed}, position quantization {qp} bits, {rounds} rounds of {iters}");
    println!(
        "{:<8} {:>7} {:>9} {:>11} {:>11} {:>7} {:>11} {:>11} {:>7}",
        "model",
        "faces",
        "bytes",
        "cpp enc us",
        "rust enc us",
        "enc x",
        "cpp dec us",
        "rust dec us",
        "dec x"
    );

    for model in &models {
        let (name, path) = model.split_once('=').expect("name=path");
        let source = source_stream(path, speed, qp);
        let mesh = decode_mesh(&source);
        let encoder_options = options(&mesh, speed, qp);

        let mut cpp_enc = Vec::new();
        let mut rust_enc = Vec::new();
        let mut cpp_dec = Vec::new();
        let mut rust_dec = Vec::new();
        let mut rust_bytes = Vec::new();
        let mut cpp_bytes = 0usize;

        // One untimed pass so neither side is measured cold.
        let _ = rust_encode_us(&mesh, &encoder_options, 1);
        let _ = draco_cpp_test_bridge::profile_cpp_reencode_mesh(&source, speed, speed, qp, 1);

        for _ in 0..rounds {
            let cpp =
                draco_cpp_test_bridge::profile_cpp_reencode_mesh(&source, speed, speed, qp, iters)
                    .expect("C++ re-encode");
            cpp_enc.push(cpp.encode_time_us as f64);
            cpp_bytes = cpp.output_size;

            let (us, bytes) = rust_encode_us(&mesh, &encoder_options, iters);
            rust_enc.push(us);
            rust_bytes = bytes;

            let cpp_d =
                draco_cpp_test_bridge::profile_cpp_decode(&rust_bytes, iters).expect("C++ decode");
            cpp_dec.push(cpp_d.decode_time_us as f64);
            rust_dec.push(rust_decode_us(&rust_bytes, iters));
        }

        let ce = median(&mut cpp_enc);
        let re = median(&mut rust_enc);
        let cd = median(&mut cpp_dec);
        let rd = median(&mut rust_dec);
        println!(
            "{:<8} {:>7} {:>9} {:>11.1} {:>11.1} {:>6.2}x {:>11.1} {:>11.1} {:>6.2}x",
            name,
            mesh.num_faces(),
            rust_bytes.len(),
            ce,
            re,
            ce / re,
            cd,
            rd,
            cd / rd
        );
        println!(
            "         spread: cpp enc {:.1}..{:.1}, rust enc {:.1}..{:.1}, cpp dec {:.1}..{:.1}, rust dec {:.1}..{:.1}; cpp wrote {} bytes, rust {}",
            cpp_enc[0], cpp_enc[cpp_enc.len() - 1],
            rust_enc[0], rust_enc[rust_enc.len() - 1],
            cpp_dec[0], cpp_dec[cpp_dec.len() - 1],
            rust_dec[0], rust_dec[rust_dec.len() - 1],
            cpp_bytes, rust_bytes.len()
        );
    }
}
