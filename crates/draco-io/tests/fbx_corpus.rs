//! Opt-in sweep over a local corpus of real FBX files.
//!
//! The repository ships no large FBX corpus, so this test is inert unless
//! `DRACO_FBX_CORPUS` points at a directory to walk. It cannot assert exact
//! geometry without ground truth; instead it asserts the properties that must
//! hold for *any* input: no panic, no hang, bounded output, and a clean
//! `Result` either way.
//!
//! ```text
//! DRACO_FBX_CORPUS=dev/fbx/ufbx/data cargo test -p draco-io \
//!     --features test --test fbx_corpus -- --nocapture
//! ```

#![cfg(feature = "test")]

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use draco_io::{FbxByteOrder, FbxReadOptions, FbxScene};

/// A single file may not take longer than this to reach a verdict.
const PER_FILE_BUDGET: Duration = Duration::from_secs(10);

fn corpus_dir() -> Option<PathBuf> {
    let raw = std::env::var("DRACO_FBX_CORPUS").ok()?;
    let path = PathBuf::from(raw);
    path.is_dir().then_some(path)
}

fn collect_fbx(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_fbx(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("fbx"))
        {
            out.push(path);
        }
    }
}

#[derive(Debug, Default)]
struct Summary {
    parsed: usize,
    rejected: usize,
    binary_seen: usize,
    total_control_points: usize,
}

#[test]
fn corpus_never_panics_and_stays_bounded() {
    let Some(dir) = corpus_dir() else {
        eprintln!("skipping: set DRACO_FBX_CORPUS to a directory of .fbx files");
        return;
    };

    let mut files = Vec::new();
    collect_fbx(&dir, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no .fbx files under {}", dir.display());

    let options = FbxReadOptions::default();
    let mut summary = Summary::default();
    let mut rejections: BTreeMap<String, usize> = BTreeMap::new();
    let mut slow = Vec::new();

    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        // ASCII FBX is a different container; it is expected to be rejected.
        let is_binary = bytes.starts_with(b"Kaydara FBX Binary");
        if is_binary {
            summary.binary_seen += 1;
        }

        let started = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            FbxScene::from_bytes_with_options(&bytes, options.clone())
        }));
        let elapsed = started.elapsed();
        if elapsed > PER_FILE_BUDGET {
            slow.push((path.clone(), elapsed));
        }

        match outcome {
            Err(_) => panic!("panicked while reading {}", path.display()),
            Ok(Ok(scene)) => {
                summary.parsed += 1;
                summary.total_control_points += control_point_total(&scene);
            }
            Ok(Err(error)) => {
                summary.rejected += 1;
                *rejections.entry(error.kind().to_string()).or_default() += 1;
                assert!(
                    !is_binary || !error.to_string().is_empty(),
                    "{} was rejected without a message",
                    path.display()
                );
            }
        }
    }

    println!("corpus: {}", dir.display());
    println!("  files            {}", files.len());
    println!("  binary container {}", summary.binary_seen);
    println!("  parsed           {}", summary.parsed);
    println!("  rejected         {}", summary.rejected);
    println!("  control points   {}", summary.total_control_points);
    for (kind, count) in &rejections {
        println!("  rejected[{kind}] {count}");
    }

    assert!(slow.is_empty(), "files exceeded the time budget: {slow:?}");
    assert!(
        summary.parsed > 0,
        "no file in the corpus parsed; the reader is likely broken"
    );
}

/// Big-endian files must decode to the same scene as their little-endian twin.
///
/// This is the one corpus assertion that checks correctness rather than
/// robustness, so it is worth the special-casing: the ufbx corpus ships
/// `maya_cube_big_endian_<version>_binary.fbx` beside `maya_cube_<version>_binary.fbx`.
#[test]
fn big_endian_matches_its_little_endian_twin() {
    let Some(dir) = corpus_dir() else {
        eprintln!("skipping: set DRACO_FBX_CORPUS to a directory of .fbx files");
        return;
    };

    let mut compared = 0;
    for version in ["6100", "7100", "7400", "7500"] {
        let little = dir.join(format!("maya_cube_{version}_binary.fbx"));
        let big = dir.join(format!("maya_cube_big_endian_{version}_binary.fbx"));
        if !little.exists() || !big.exists() {
            continue;
        }

        let little_bytes = std::fs::read(&little).expect("read little-endian twin");
        let big_bytes = std::fs::read(&big).expect("read big-endian twin");

        let little_reader =
            draco_io::FbxMemoryReader::from_bytes(little_bytes.clone()).expect("open little");
        let big_reader =
            draco_io::FbxMemoryReader::from_bytes(big_bytes.clone()).expect("open big");
        assert_eq!(little_reader.byte_order(), FbxByteOrder::Little);
        assert_eq!(big_reader.byte_order(), FbxByteOrder::Big);
        assert_eq!(little_reader.version(), big_reader.version());

        let little_scene = FbxScene::from_bytes(&little_bytes).expect("parse little");
        let big_scene = FbxScene::from_bytes(&big_bytes).expect("parse big");

        assert_eq!(
            format!("{:?}", little_scene),
            format!("{:?}", big_scene),
            "endian twins decoded differently at version {version}"
        );
        compared += 1;
    }

    if compared == 0 {
        eprintln!("skipping: corpus has no maya_cube endian twins");
    } else {
        println!("compared {compared} big-endian/little-endian pairs");
    }
}

fn control_point_total(scene: &FbxScene) -> usize {
    fn visit(node: &draco_io::FbxSceneNode) -> usize {
        node.mesh_instances
            .iter()
            .map(|mesh| mesh.control_points.len())
            .sum::<usize>()
            + node.children.iter().map(visit).sum::<usize>()
    }
    scene.root_nodes.iter().map(visit).sum()
}
