//! Print a one-line verdict per FBX file under a directory.
//!
//! Used to diff reader behaviour between revisions:
//!
//! ```text
//! cargo run --example fbx_sweep -- <dir> > after.txt
//! ```
//!
//! `--digest` hashes what was read, `--write-digest` hashes what is written
//! back. Between them they bracket a refactor: a change to either half that
//! was meant to preserve behaviour must leave its digest untouched.

use draco_io::{FbxReadOptions, FbxScene, FbxWriter};
use std::path::{Path, PathBuf};

/// FNV-1a, the same fold both digests use.
fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Writes `scene` twice and folds both results.
///
/// Both compression modes are folded because they are different code paths in
/// the array encoder, and the default is uncompressed -- hashing only the
/// default would leave the deflate branch, including its "keep the compressed
/// form only if it came out shorter" decision, entirely unwatched.
fn write_digest(scene: &FbxScene) -> std::io::Result<(u64, usize, u64, usize)> {
    let plain = {
        let mut writer = FbxWriter::new();
        writer.add_scene(scene)?;
        writer.write_to_vec()?
    };
    let packed = {
        let mut writer = FbxWriter::new().with_compression(true);
        writer.add_scene(scene)?;
        writer.write_to_vec()?
    };
    Ok((
        fnv1a(plain.iter().copied()),
        plain.len(),
        fnv1a(packed.iter().copied()),
        packed.len(),
    ))
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("fbx"))
        {
            out.push(path);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(args.get(1).ok_or("usage: fbx_sweep <dir>")?);
    let skip_fuzz = args.iter().any(|a| a == "--skip-fuzz");
    let strict = args.iter().any(|a| a == "--strict");
    // `--digest` prints a hash of the whole decoded scene, so a refactor can
    // be proven to change nothing.
    let digest = args.iter().any(|a| a == "--digest");
    // `--write-digest` hashes the bytes the writer produces instead. Nothing
    // else in the repository pins the writer's byte layout, so this is the
    // only way to show that a change to the writer was a pure move.
    let write = args.iter().any(|a| a == "--write-digest");
    let options = if strict {
        FbxReadOptions::strict()
    } else {
        FbxReadOptions::default()
    };

    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();

    // One line per file, folded once at the end so a whole corpus reduces to a
    // single number to compare across revisions.
    let mut written: Vec<String> = Vec::new();

    for path in files {
        let display = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if skip_fuzz && path.components().any(|c| c.as_os_str() == "fuzz") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            println!("READFAIL {display}");
            continue;
        };
        if !bytes.starts_with(b"Kaydara FBX Binary") {
            continue; // ASCII container, not this reader's job.
        }
        match FbxScene::from_bytes_with_options(&bytes, options.clone()) {
            Ok(scene) if write => match write_digest(&scene) {
                Ok((plain, plain_len, packed, packed_len)) => {
                    let line = format!(
                        "WRITE {display} plain={plain:016x}:{plain_len} packed={packed:016x}:{packed_len}"
                    );
                    println!("{line}");
                    written.push(line);
                }
                Err(error) => println!("WRITEFAIL {display} {error}"),
            },
            Ok(scene) if digest => {
                // Derived `Debug` prints each struct's declared name, which a
                // type alias does not preserve. Those names are not semantics,
                // so normalize them out; otherwise renaming a type looks
                // exactly like changing every decoded value.
                let text = format!("{scene:?}")
                    .replace("FbxUvSet", "LayerSet")
                    .replace("FbxNormalSet", "LayerSet")
                    .replace("FbxColorSet", "LayerSet")
                    .replace("FbxLayerSet", "LayerSet");
                println!("DIGEST {display} {:016x}", fnv1a(text.bytes()));
            }
            Ok(scene) => {
                let points: usize = count_points(&scene);
                let (welded, corners) = count_mesh_points(&scene);
                println!(
                    "OK {display} roots={} points={points} welded={welded} corners={corners} colors={} uvsets={} edges={} warnings={}",
                    scene.root_nodes.len(),
                    count_color_sets(&scene),
                    max_uv_sets(&scene),
                    total_edges(&scene),
                    scene.warnings.len()
                );
            }
            Err(error) => println!("ERR {display} {error}"),
        }
    }

    if write {
        println!(
            "WRITE TOTAL {} files {:016x}",
            written.len(),
            fnv1a(written.join("\n").bytes())
        );
    }
    Ok(())
}

fn count_points(scene: &FbxScene) -> usize {
    fn visit(node: &draco_io::FbxSceneNode) -> usize {
        node.mesh_instances
            .iter()
            .map(|m| m.control_points.len())
            .sum::<usize>()
            + node.children.iter().map(visit).sum::<usize>()
    }
    scene.root_nodes.iter().map(visit).sum()
}

fn total_edges(scene: &FbxScene) -> usize {
    fn visit(node: &draco_io::FbxSceneNode) -> usize {
        node.mesh_instances
            .iter()
            .map(|m| m.edges.len())
            .sum::<usize>()
            + node.children.iter().map(visit).sum::<usize>()
    }
    scene.root_nodes.iter().map(visit).sum()
}

fn max_uv_sets(scene: &FbxScene) -> usize {
    fn visit(node: &draco_io::FbxSceneNode) -> usize {
        node.mesh_instances
            .iter()
            .map(|m| m.layers.uv_sets.len())
            .chain(node.children.iter().map(visit))
            .max()
            .unwrap_or(0)
    }
    scene.root_nodes.iter().map(visit).max().unwrap_or(0)
}

fn count_color_sets(scene: &FbxScene) -> usize {
    fn visit(node: &draco_io::FbxSceneNode) -> usize {
        node.mesh_instances
            .iter()
            .map(|m| m.layers.color_sets.len())
            .sum::<usize>()
            + node.children.iter().map(visit).sum::<usize>()
    }
    scene.root_nodes.iter().map(visit).sum()
}

/// Draco mesh points and corner count, to show the cost of seam preservation.
fn count_mesh_points(scene: &FbxScene) -> (usize, usize) {
    fn visit(node: &draco_io::FbxSceneNode) -> (usize, usize) {
        let mut welded = 0;
        let mut corners = 0;
        for mesh in &node.mesh_instances {
            welded += mesh.mesh.num_points();
            corners += mesh.to_render_mesh().corner_count();
        }
        for child in &node.children {
            let (w, c) = visit(child);
            welded += w;
            corners += c;
        }
        (welded, corners)
    }
    let mut welded = 0;
    let mut corners = 0;
    for root in &scene.root_nodes {
        let (w, c) = visit(root);
        welded += w;
        corners += c;
    }
    (welded, corners)
}
