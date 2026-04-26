use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use draco_core::mesh_encoder::MeshEncoder;
use draco_core::EncoderOptions;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use tempfile::Builder;

fn read_ply_header(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    // PLY header is always ASCII and terminated by end_header\n.
    let marker = b"end_header\n";
    let end = bytes
        .windows(marker.len())
        .position(|w| w == marker)
        .map(|pos| pos + marker.len())
        .unwrap_or(bytes.len());
    Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

fn get_cpp_tools_path() -> Option<std::path::PathBuf> {
    let path = Path::new("../../build/Debug");
    if path.exists() {
        Some(path.to_path_buf())
    } else {
        // Try Release
        let path = Path::new("../../build/Release");
        if path.exists() {
            Some(path.to_path_buf())
        } else {
            None
        }
    }
}

fn create_torus_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    mesh.set_num_points(4);
    mesh.set_num_faces(2);

    let mut pos_attr = PointAttribute::new();
    pos_attr.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        4,
    );

    let coords: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];

    for i in 0..4 {
        let offset = i * 3 * 4;
        pos_attr
            .buffer_mut()
            .update(&coords[i * 3].to_le_bytes(), Some(offset));
        pos_attr
            .buffer_mut()
            .update(&coords[i * 3 + 1].to_le_bytes(), Some(offset + 4));
        pos_attr
            .buffer_mut()
            .update(&coords[i * 3 + 2].to_le_bytes(), Some(offset + 8));
    }
    mesh.add_attribute(pos_attr);

    // f 0 1 2
    // f 0 2 3
    mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
    mesh.set_face(FaceIndex(1), [PointIndex(0), PointIndex(2), PointIndex(3)]);

    mesh
}

#[test]
fn test_rust_encode_cpp_decode() {
    let tools_path = match get_cpp_tools_path() {
        Some(p) => p,
        None => {
            println!("Skipping compatibility test: C++ tools not found");
            return;
        }
    };
    let decoder_path = tools_path.join("draco_decoder.exe");
    let encoder_path = tools_path.join("draco_encoder.exe");
    if !decoder_path.exists() || !encoder_path.exists() {
        println!("Skipping compatibility test: tools not found");
        return;
    }

    let temp_dir = Builder::new()
        .prefix("draco_test")
        .tempdir()
        .expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    let mesh = create_torus_mesh();

    // Write OBJ for C++ encoder
    let obj_path = temp_path.join("torus.obj");
    draco_io::obj_writer::write_obj_mesh(&obj_path, &mesh).expect("Failed to write OBJ");

    // Run C++ encoder
    let cpp_drc_path = temp_path.join("cpp_encoded.drc");
    let status = Command::new(&encoder_path)
        .arg("-i")
        .arg(&obj_path)
        .arg("-o")
        .arg(&cpp_drc_path)
        .arg("-method")
        .arg("edgebreaker")
        .arg("-qp")
        .arg("10")
        .status()
        .expect("Failed to run draco_encoder");
    assert!(status.success(), "C++ encoder failed");

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1); // Edgebreaker
    options.set_attribute_int(0, "quantization_bits", 10);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut encoder_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoder_buffer)
        .expect("Encode failed");

    let drc_path = temp_path.join("rust_encoded.drc");
    let mut file = File::create(&drc_path).expect("Failed to create drc file");
    file.write_all(encoder_buffer.data())
        .expect("Failed to write drc file");

    // Compare files
    // We don't compare binary data directly because different encoders might produce different valid streams.
    // Instead we verify the C++ decoder can decode our output.

    let output = Command::new(&decoder_path)
        .arg("-i")
        .arg(&drc_path)
        .output()
        .expect("Failed to run draco_decoder");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    println!("Decoder stdout: {}", stdout);
    println!("Decoder stderr: {}", stderr);

    assert!(output.status.success(), "C++ decoder failed");

    // Verify decoded geometry by reading the generated PLY file
    // Note: draco_decoder saves decoded mesh to .ply file with the same name as input + .ply
    // when no output file is specified? Or we need to check stdout.
    // draco_decoder behavior: "Decoded mesh saved to ..."
    // By default it might save to current dir if we don't specify output?
    // Actually, looking at original test: it checked `rust_encoded.drc.ply`.
    // We should check file existence in current dir?
    // Wait, if I run draco_decoder on a file in temp dir, where does it save the output?
    // It usually saves to the same directory as input or current directory.
    // Let's check `ply_path`.

    // To be safe, we should specify output path for decoder if possible, but draco_decoder
    // might not expose it easily via CLI args we used (it usually auto-names).
    // If it saves to CWD, we are still polluting.
    // Let's specify output file if possible. `draco_decoder -i <input> -o <output>`

    let decoded_ply_path = temp_path.join("rust_encoded.decoded.ply");
    let output = Command::new(&decoder_path)
        .arg("-i")
        .arg(&drc_path)
        .arg("-o")
        .arg(&decoded_ply_path)
        .output()
        .expect("Failed to run draco_decoder again");

    assert!(output.status.success(), "C++ decoder failed 2nd run");

    if decoded_ply_path.exists() {
        let ply_content =
            read_ply_header(&decoded_ply_path).expect("Failed to read PLY file header");
        assert!(
            ply_content.contains("element vertex 4"),
            "Decoded mesh has incorrect number of points"
        );
        assert!(
            ply_content.contains("element face 2"),
            "Decoded mesh has incorrect number of faces"
        );
    } else {
        // Fallback or failure
        println!(
            "Decoder output files not found at {}",
            decoded_ply_path.display()
        );
        panic!("PLY file not found");
    }
}

#[test]
fn test_rust_encode_rust_decode() {
    let mesh = create_torus_mesh();

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 0); // Sequential
    options.set_attribute_int(0, "quantization_bits", 10);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut encoder_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoder_buffer)
        .expect("Encode failed");

    let data = encoder_buffer.data();
    let mut decoder_buffer = DecoderBuffer::new(data);

    let mut decoder = MeshDecoder::new();
    let mut decoded_mesh = Mesh::new();
    let status = decoder.decode(&mut decoder_buffer, &mut decoded_mesh);

    assert!(status.is_ok(), "Rust decoder failed: {:?}", status);
    assert_eq!(decoded_mesh.num_points(), 4);
    assert_eq!(decoded_mesh.num_faces(), 2);
}

#[test]
fn test_cpp_encode_rust_decode() {
    let tools_path = match get_cpp_tools_path() {
        Some(p) => p,
        None => {
            println!("Skipping compatibility test: C++ tools not found");
            return;
        }
    };
    let encoder_path = tools_path.join("draco_encoder.exe");
    if !encoder_path.exists() {
        println!("Skipping compatibility test: draco_encoder.exe not found");
        return;
    }

    let temp_dir = Builder::new()
        .prefix("draco_test_cpp")
        .tempdir()
        .expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    let mesh = create_torus_mesh();
    let obj_path = temp_path.join("temp_input.obj");
    draco_io::obj_writer::write_obj_mesh(&obj_path, &mesh).expect("Failed to write obj");

    let drc_path = temp_path.join("temp_cpp_out.drc");

    let output = Command::new(&encoder_path)
        .arg("-i")
        .arg(&obj_path)
        .arg("-o")
        .arg(&drc_path)
        .arg("-method")
        .arg("edgebreaker") // or "1"
        .arg("-cl")
        .arg("0")
        .arg("-qp")
        .arg("10") // quantization bits
        .output()
        .expect("Failed to run draco_encoder");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("Encoder stdout: {}", stdout);
    println!("Encoder stderr: {}", stderr);

    assert!(output.status.success(), "C++ encoder failed");

    let metadata = std::fs::metadata(&drc_path).unwrap();
    println!("C++ encoded size: {}", metadata.len());

    // Decode with Rust
    let mut file = File::open(&drc_path).expect("Failed to open drc file");
    let mut buffer = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut buffer).expect("Failed to read drc file");

    let mut decoder = MeshDecoder::new();
    let mut decoder_buffer = DecoderBuffer::new(&buffer);
    let mut decoded_mesh = Mesh::new();

    let status = decoder.decode(&mut decoder_buffer, &mut decoded_mesh);
    match status {
        Ok(_) => {
            assert_eq!(decoded_mesh.num_faces(), mesh.num_faces());
            assert_eq!(decoded_mesh.num_points(), mesh.num_points());
            println!("Rust decoder successfully decoded C++ stream");
        }
        Err(e) => {
            panic!("Rust decoder failed: {:?}", e);
        }
    }
}

#[test]
fn test_rust_encode_rust_decode_edgebreaker() {
    let mesh = create_torus_mesh();

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_method", 1); // Edgebreaker
    options.set_attribute_int(0, "quantization_bits", 10);

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());
    let mut encoder_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut encoder_buffer)
        .expect("Encode failed");

    let data = encoder_buffer.data();
    let mut decoder_buffer = DecoderBuffer::new(data);

    let mut decoder = MeshDecoder::new();
    let mut decoded_mesh = Mesh::new();
    let status = decoder.decode(&mut decoder_buffer, &mut decoded_mesh);

    assert!(status.is_ok(), "Rust decoder failed: {:?}", status);
    assert_eq!(decoded_mesh.num_points(), 4);
    assert_eq!(decoded_mesh.num_faces(), 2);
}
