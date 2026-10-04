//! Every Draco stream in a directory of someone else's files, decoded here and
//! by C++ Draco, and every glTF there imported and decoded through draco-gltf.
//!
//! A corpus no test can carry -- files a user reports, a sample-model
//! repository, what another tool writes -- checked in one command:
//!
//! ```text
//! DRACO_CORPUS_DIR=<dir> cargo test --release -p draco-cpp-test-bridge \
//!   --test external_draco_corpus_probe -- --ignored --nocapture
//! ```
//!
//! The directory is walked whole. A `.drc` is one stream; a `.gltf` or `.glb`
//! gives each `KHR_draco_mesh_compression` primitive's stream, and is also
//! imported under both validation profiles with every Draco primitive decoded.
//! A stream passes when this crate's decode has C++'s fingerprint: the counts,
//! the faces in order, and every attribute's bytes for every point. The probe
//! fails if any stream or glTF does not pass, after listing all of them.
//!
//! `TESTING.md` has how to build a corpus from glTF-Transform's encoder.

mod fingerprint;
use fingerprint::*;

use std::path::{Path, PathBuf};

use draco_gltf::{import_slice_with_options, ExternalFilePolicy, ImportOptions, ValidationProfile};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// The streams of a glTF's Draco primitives, named by mesh and primitive.
fn gltf_streams(import: &draco_gltf::Import) -> Vec<(String, Vec<u8>)> {
    let json = import.document.as_value();
    let mut streams = Vec::new();
    for (m, mesh) in json["meshes"].as_array().into_iter().flatten().enumerate() {
        for (p, primitive) in mesh["primitives"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some(view) =
                primitive["extensions"]["KHR_draco_mesh_compression"]["bufferView"].as_u64()
            else {
                continue;
            };
            let view = &json["bufferViews"][view as usize];
            let buffer = &import.resources.buffers[view["buffer"].as_u64().unwrap() as usize];
            let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
            let end = start + view["byteLength"].as_u64().unwrap() as usize;
            streams.push((format!("m{m}p{p}"), buffer[start..end].to_vec()));
        }
    }
    streams
}

/// `None` where the stream decodes here as C++ decodes it, and what differs
/// otherwise.
fn check_stream(data: &[u8]) -> Option<String> {
    let point_cloud = data.get(7) == Some(&0);
    let rust = std::panic::catch_unwind(|| {
        if point_cloud {
            rust_decode_point_cloud_fingerprint(data)
        } else {
            rust_decode_fingerprint(data)
        }
    });
    let cpp = if point_cloud {
        draco_cpp_test_bridge::decode_cpp_point_cloud_fingerprint(data)
    } else {
        draco_cpp_test_bridge::decode_cpp_mesh_fingerprint(data)
    };
    match (rust, cpp) {
        (Ok(r), Some(c))
            if (
                r.num_points,
                r.num_faces,
                r.num_attributes,
                r.face_hash,
                r.attribute_hash,
            ) == (
                c.num_points,
                c.num_faces,
                c.num_attributes,
                c.face_hash,
                c.attribute_hash,
            ) =>
        {
            None
        }
        (Ok(r), Some(c)) => Some(format!("decodes differently: here {r:?}, C++ {c:?}")),
        (Ok(_), None) => Some("C++ refuses it, this crate decodes it".into()),
        (Err(_), Some(_)) => Some("this crate refuses it, C++ decodes it".into()),
        (Err(_), None) => None,
    }
}

#[test]
#[ignore = "PROBE: set DRACO_CORPUS_DIR"]
fn every_stream_decodes_as_cpp_decodes_it() {
    let Some(dir) = std::env::var_os("DRACO_CORPUS_DIR").map(PathBuf::from) else {
        eprintln!("set DRACO_CORPUS_DIR to the directory to check");
        return;
    };
    assert!(
        draco_cpp_test_bridge::is_available(),
        "the probe compares against C++ Draco; build the bridge with DRACO_CPP_BUILD_DIR"
    );
    let mut paths = Vec::new();
    files(&dir, &mut paths);
    let (mut streams, mut documents, mut problems) = (0, 0, Vec::new());
    for path in paths {
        let name = path
            .strip_prefix(&dir)
            .unwrap_or(&path)
            .display()
            .to_string();
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("drc") => {
                streams += 1;
                if let Some(problem) = check_stream(&std::fs::read(&path).unwrap()) {
                    problems.push(format!("{name}: {problem}"));
                }
            }
            Some("gltf" | "glb") => {
                documents += 1;
                let bytes = std::fs::read(&path).unwrap();
                for profile in [ValidationProfile::Gltf20, ValidationProfile::Gltf21Draft] {
                    let options = ImportOptions {
                        base_path: path.parent(),
                        external_file_policy: ExternalFilePolicy::ConfineToBase,
                        profile,
                        ..ImportOptions::default()
                    };
                    let import = match import_slice_with_options(&bytes, &options) {
                        Ok(import) => import,
                        Err(e) => {
                            problems.push(format!("{name} ({profile:?}): does not import: {e}"));
                            continue;
                        }
                    };
                    for primitive in import.draco_primitives() {
                        if let Err(e) = import.decode_draco_primitive(primitive) {
                            problems.push(format!(
                                "{name} ({profile:?}): a primitive does not decode: {e}"
                            ));
                        }
                    }
                    if profile == ValidationProfile::Gltf20 {
                        for (label, stream) in gltf_streams(&import) {
                            streams += 1;
                            if let Some(problem) = check_stream(&stream) {
                                problems.push(format!("{name} {label}: {problem}"));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    for problem in &problems {
        println!("{problem}");
    }
    println!(
        "{streams} streams and {documents} glTF files, {} problems",
        problems.len()
    );
    assert!(problems.is_empty());
}
