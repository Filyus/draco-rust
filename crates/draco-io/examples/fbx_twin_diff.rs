//! Compare an ASCII FBX against its binary twin, field by field.
//!
//! `an_ascii_document_decodes_like_its_binary_twin` in `tests/fbx_corpus.rs`
//! asserts the two spellings of one document decode alike, but a pair listed
//! in its `NOT_COMPARABLE` array cannot be inspected without editing the test.
//! This runs the same comparison on demand, for one pair, and prints what
//! differs -- which is what a person needs before deciding whether an
//! exclusion is a defect, a precision artefact, or two different exports.
//!
//! ```text
//! cargo run --example fbx_twin_diff -- maya_cube_7500_ascii.fbx
//! cargo run --example fbx_twin_diff -- a.fbx b.fbx
//! ```
//!
//! With one argument the twin is found by swapping `_ascii` for `_binary` in
//! the file name.

use std::env;
use std::path::{Path, PathBuf};

use draco_io::{FbxScene, FbxSceneNode};

/// One comparable view of a scene.
///
/// Deliberately the same shape as `SceneSummary` in the corpus test: sorted
/// string lists per family, so a difference names its family and shows the
/// first entry that disagrees. Materials and textures are reduced to names,
/// as the test does, because the two exports of one scene assign different
/// object ids and so list them in a different order.
#[derive(Default)]
struct Summary {
    counts: Vec<String>,
    meshes: Vec<String>,
    transforms: Vec<String>,
    transform_stacks: Vec<String>,
    channels: Vec<String>,
    material_names: Vec<String>,
    texture_names: Vec<String>,
    global_settings: String,
}

fn visit(node: &FbxSceneNode, out: &mut Summary) {
    let name = node.name.clone().unwrap_or_default();
    out.transforms.push(format!(
        "{name}|{:?}",
        node.transform.as_ref().map(|t| t
            .matrix
            .map(|row| row.map(|v| (f64::from(v) * 1e4).round() as i64)))
    ));
    out.transform_stacks
        .push(format!("{name}|{:?}", node.transform_stack));
    for mesh in &node.mesh_instances {
        out.meshes.push(format!(
            "{name}|{:?}|points={}|corners={}|uv={}|colour={}|normal={}|edges={}",
            mesh.name,
            mesh.control_points.len(),
            mesh.polygon_vertex_indices.len(),
            mesh.layers.uv_sets.len(),
            mesh.layers.color_sets.len(),
            mesh.layers.normal_sets.len(),
            mesh.edges.len(),
        ));
    }
    for child in &node.children {
        visit(child, out);
    }
}

fn summarize(scene: &FbxScene) -> Summary {
    let mut out = Summary {
        global_settings: format!("{:?}", scene.global_settings),
        ..Summary::default()
    };
    for root in &scene.root_nodes {
        visit(root, &mut out);
    }
    out.counts = vec![
        format!("materials={}", scene.materials.len()),
        format!("textures={}", scene.textures.len()),
        format!("animations={}", scene.animations.len()),
    ];
    for clip in &scene.animations {
        for channel in &clip.channels {
            let sampler = &channel.sampler;
            out.channels.push(format!(
                "{}|{:?}|{:?}|keys={}|first={:?}|last={:?}|interp={:?}",
                channel.node_name,
                channel.path,
                channel.morph_target_index,
                sampler.input.len(),
                sampler.output.first(),
                sampler.output.last(),
                sampler.interpolation,
            ));
        }
    }
    out.material_names = scene
        .materials
        .iter()
        .map(|material| format!("{:?}", material.name))
        .collect();
    out.texture_names = scene
        .textures
        .iter()
        .map(|texture| format!("{:?}", texture.name))
        .collect();
    for list in [
        &mut out.meshes,
        &mut out.transforms,
        &mut out.transform_stacks,
        &mut out.channels,
        &mut out.material_names,
        &mut out.texture_names,
    ] {
        list.sort();
    }
    out
}

/// Prints every family that differs plus the first disagreeing entry.
fn report(label: &str, left: &[String], right: &[String]) -> usize {
    if left == right {
        return 0;
    }
    print!("{label}: {} vs {} entries", left.len(), right.len());
    match left.iter().zip(right).find(|(a, b)| a != b) {
        Some((a, b)) => println!("\n    left  {a}\n    right {b}"),
        None => println!(" (one is a prefix of the other)"),
    }
    1
}

/// Swaps `_ascii` for `_binary` in the file name, the corpus convention.
fn twin_of(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let twin = if name.contains("_ascii") {
        name.replace("_ascii", "_binary")
    } else {
        name.replace("_binary", "_ascii")
    };
    (twin != name).then(|| path.with_file_name(twin))
}

fn load(path: &Path) -> Result<FbxScene, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let scene =
        FbxScene::from_bytes(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    println!(
        "{}: {} bytes, {} root nodes, {} warnings",
        path.display(),
        bytes.len(),
        scene.root_nodes.len(),
        scene.warnings.len(),
    );
    Ok(scene)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let (left, right) = match args.as_slice() {
        [one] => {
            let left = PathBuf::from(one);
            let right =
                twin_of(&left).ok_or("cannot derive a twin name: pass both files explicitly")?;
            (left, right)
        }
        [one, two] => (PathBuf::from(one), PathBuf::from(two)),
        _ => return Err("usage: fbx_twin_diff <ascii.fbx> [binary.fbx]".into()),
    };

    let left_summary = summarize(&load(&left)?);
    let right_summary = summarize(&load(&right)?);

    let mut differing = report("counts", &left_summary.counts, &right_summary.counts);
    differing += report("meshes", &left_summary.meshes, &right_summary.meshes);
    differing += report(
        "transforms",
        &left_summary.transforms,
        &right_summary.transforms,
    );
    differing += report(
        "transform_stacks",
        &left_summary.transform_stacks,
        &right_summary.transform_stacks,
    );
    differing += report("channels", &left_summary.channels, &right_summary.channels);
    differing += report(
        "material_names",
        &left_summary.material_names,
        &right_summary.material_names,
    );
    differing += report(
        "texture_names",
        &left_summary.texture_names,
        &right_summary.texture_names,
    );
    if left_summary.global_settings != right_summary.global_settings {
        differing += 1;
        println!(
            "global_settings:\n    left  {}\n    right {}",
            left_summary.global_settings, right_summary.global_settings
        );
    }

    if differing == 0 {
        println!("IDENTICAL");
    } else {
        println!("{differing} families differ");
    }
    Ok(())
}
