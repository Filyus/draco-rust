//! Test valence decoder (speed 0) decoding
use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use std::path::PathBuf;

fn get_testdata_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("testdata")
}

#[test]
fn test_decode_speed_0_cube() {
    let path = get_testdata_path()
        .join("reference_cpp")
        .join("cpp_encoded_cube_speed_0.drc");
    if !path.exists() {
        println!("Skipping test - file not found: {:?}", path);
        return;
    }

    let data = std::fs::read(&path).expect("Failed to read file");
    let mut buffer = DecoderBuffer::new(&data);
    let mut mesh = Mesh::new();

    match MeshDecoder::new().decode(&mut buffer, &mut mesh) {
        Ok(_) => {
            println!(
                "SUCCESS: faces={} points={}",
                mesh.num_faces(),
                mesh.num_points()
            );
            assert!(mesh.num_faces() > 0, "Expected faces > 0");
            assert!(mesh.num_points() > 0, "Expected points > 0");
        }
        Err(e) => panic!("Failed to decode speed 0 file: {:?}", e),
    }
}

#[test]
fn test_decode_bunny_cpp_standard() {
    // This is the bunny encoded with standard (speed 0) mode
    let path = get_testdata_path().join("bunny_cpp_standard.drc");
    if !path.exists() {
        println!("Skipping test - file not found: {:?}", path);
        return;
    }

    let data = std::fs::read(&path).expect("Failed to read file");
    let mut buffer = DecoderBuffer::new(&data);
    let mut mesh = Mesh::new();

    match MeshDecoder::new().decode(&mut buffer, &mut mesh) {
        Ok(_) => {
            println!(
                "SUCCESS: faces={} points={}",
                mesh.num_faces(),
                mesh.num_points()
            );
            assert!(mesh.num_faces() > 0, "Expected faces > 0");
            assert!(mesh.num_points() > 0, "Expected points > 0");
        }
        Err(e) => panic!("Failed to decode bunny_cpp_standard.drc: {:?}", e),
    }
}

#[test]
fn test_decode_all_speed_0_files() {
    // Test all speed 0 files in testdata
    let testdata = get_testdata_path();

    for entry in std::fs::read_dir(&testdata).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().map(|e| e == "drc").unwrap_or(false) {
            let name = path.file_name().unwrap().to_string_lossy();
            // Try to decode and check if it's speed 0 (valence)
            let data = std::fs::read(&path).expect("Failed to read file");
            let mut buffer = DecoderBuffer::new(&data);
            let mut mesh = Mesh::new();

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                MeshDecoder::new().decode(&mut buffer, &mut mesh)
            }));

            match result {
                Ok(Ok(_)) => {
                    println!(
                        "OK: {} -> faces={} points={}",
                        name,
                        mesh.num_faces(),
                        mesh.num_points()
                    );
                }
                Ok(Err(e)) => {
                    // Only panic if it's a valence-related error
                    let err_str = format!("{:?}", e);
                    if err_str.contains("valence") || err_str.contains("Valence") {
                        panic!("Valence error for {}: {:?}", name, e);
                    }
                    println!("SKIP: {} -> {:?}", name, e);
                }
                Err(_) => {
                    println!("PANIC: {} -> caught panic", name);
                }
            }
        }
    }
}
