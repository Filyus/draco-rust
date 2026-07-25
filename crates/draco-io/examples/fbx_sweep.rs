//! Print a one-line verdict per FBX file under a directory.
//!
//! Used to diff reader behaviour between revisions:
//!
//! ```text
//! cargo run --example fbx_sweep -- <dir> > after.txt
//! ```

use draco_io::{FbxReadOptions, FbxScene};
use std::path::{Path, PathBuf};

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
    let options = if strict {
        FbxReadOptions::strict()
    } else {
        FbxReadOptions::default()
    };

    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();

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
            Ok(scene) => {
                let points: usize = count_points(&scene);
                let (welded, corners) = count_mesh_points(&scene);
                println!(
                    "OK {display} roots={} points={points} welded={welded} corners={corners} warnings={}",
                    scene.root_nodes.len(),
                    scene.warnings.len()
                );
            }
            Err(error) => println!("ERR {display} {error}"),
        }
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
