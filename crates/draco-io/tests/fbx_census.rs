//! A feature census over a local corpus of real FBX files.
//!
//! "How much of FBX do we support" has no single number, but a corpus makes
//! the question measurable: walk the directory the `DRACO_FBX_CORPUS`
//! environment variable points at, and report two views of the same walk.
//!
//! *Acceptance* — how many files parse, bucketed by the rejection kind when
//! they do not, and by container and declared version when they do. A corpus
//! deliberately holding truncated or corrupted files shows up here as its own
//! rejection buckets rather than as a support gap.
//!
//! *Feature frequency* — over the accepted files, how often each capability
//! the reader surfaces appears: layer element sets on meshes, deformers,
//! animation, embedded textures, cameras and lights. Read against the reader's
//! own surface, this is the honest denominator for "how much of what files
//! carry do we handle": a capability no file in the corpus uses costs nothing
//! to not support, and one that half the corpus uses is load-bearing.
//!
//! ```text
//! DRACO_FBX_CORPUS=path/to/fbx/corpus cargo test -p draco-io \
//!     --features test --test fbx_census -- --nocapture
//! ```
//!
//! The census reports; it asserts only what must hold for any input — that
//! the corpus was walked and that at least some of it parsed, which is what
//! tells a broken reader apart from a broken environment.

#![cfg(feature = "test")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use draco_io::{FbxNodeKind, FbxScene};

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
        } else if path.extension().is_some_and(|e| e == "fbx") {
            out.push(path);
        }
    }
}

/// The rejection kind, as the caller would see it from `io::ErrorKind`.
fn rejection_kind(error: &std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof => "unexpected end of file",
        std::io::ErrorKind::OutOfMemory => "out of memory",
        _ => "invalid data",
    }
}

/// What one accepted scene carries, as the reader's own surface names it.
struct Features {
    meshes: usize,
    joint_models: bool,
    camera_or_light: bool,
    uv_sets: bool,
    color_sets: bool,
    tangent_sets: bool,
    binormal_sets: bool,
    smoothing_or_crease: bool,
    material_assignment: bool,
    edges: bool,
    skins: bool,
    blend_shapes: bool,
    materials: bool,
    textures: bool,
    embedded_textures: bool,
    animation: bool,
    read_warnings: bool,
}

fn collect_features(scene: &FbxScene) -> Features {
    let mut state = Features {
        meshes: 0,
        joint_models: false,
        camera_or_light: false,
        uv_sets: false,
        color_sets: false,
        tangent_sets: false,
        binormal_sets: false,
        smoothing_or_crease: false,
        material_assignment: false,
        edges: false,
        skins: false,
        blend_shapes: false,
        materials: !scene.materials.is_empty(),
        textures: !scene.textures.is_empty(),
        embedded_textures: scene.textures.iter().any(|t| t.content.is_some()),
        animation: !scene.animations.is_empty(),
        read_warnings: !scene.warnings.is_empty(),
    };
    fn visit(node: &draco_io::FbxSceneNode, state: &mut Features) {
        if node.kind == Some(FbxNodeKind::Joint) {
            state.joint_models = true;
        }
        if node.attribute.is_some() {
            state.camera_or_light = true;
        }
        for mesh in &node.mesh_instances {
            state.meshes += 1;
            state.uv_sets |= !mesh.layers.uv_sets.is_empty();
            state.color_sets |= !mesh.layers.color_sets.is_empty();
            state.tangent_sets |= !mesh.layers.tangent_sets.is_empty();
            state.binormal_sets |= !mesh.layers.binormal_sets.is_empty();
            state.smoothing_or_crease |=
                !mesh.layers.smoothing_layers.is_empty() || !mesh.layers.crease_layers.is_empty();
            state.material_assignment |= !mesh.material_indices.is_empty();
            state.edges |= !mesh.edges.is_empty();
            state.skins |= mesh.skin.is_some();
            state.blend_shapes |= !mesh.morph_targets.is_empty();
        }
        for child in &node.children {
            visit(child, state);
        }
    }
    for root in &scene.root_nodes {
        visit(root, &mut state);
    }
    state
}

/// Declared format version: the binary header carries it as a little-endian
/// u32 after the vendor signature; an ASCII document spells it in its first
/// comment, either as `FBX Version 7.4` or as `FBX 7.4.0 project file`.
fn declared_version(bytes: &[u8]) -> String {
    if bytes.starts_with(b"Kaydara") && bytes.len() > 27 {
        let raw = u32::from_le_bytes(bytes[23..27].try_into().unwrap());
        // A handful of corpus files carry something else in the field; the
        // census buckets them rather than pretending they spell a version.
        return if raw <= 10_000 {
            format!("{raw}")
        } else {
            "nonstandard".to_string()
        };
    }
    let digits_after = |marker: &[u8]| -> Option<String> {
        let at = bytes.windows(marker.len()).position(|w| w == marker)?;
        let rest = &bytes[at + marker.len()..(at + marker.len() + 10).min(bytes.len())];
        Some(
            String::from_utf8_lossy(rest)
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect(),
        )
    };
    let ascii = digits_after(b"FBX Version")
        .filter(|v| !v.is_empty())
        .or_else(|| digits_after(b"FBX ").filter(|v| !v.is_empty()))
        .map(|v| {
            // `6.1.0` and `6100` are the same version spelled by two
            // exporters; normalize to the four-digit form.
            let digits: String = v.chars().filter(|c| *c != '.').collect();
            if digits.len() == 3 {
                format!("{digits}0")
            } else {
                digits
            }
        })
        .unwrap_or_else(|| "unknown".to_string());
    ascii
}

fn tally(map: &mut BTreeMap<String, usize>, key: String) {
    *map.entry(key).or_default() += 1;
}

/// The share of the accepted set, as a whole percent.
fn share(count: usize, accepted: usize) -> usize {
    (count * 100).div_ceil(accepted)
}

fn bump(map: &mut BTreeMap<&'static str, usize>, key: &'static str) {
    *map.entry(key).or_default() += 1;
}

#[test]
fn census_the_corpus() {
    let Some(dir) = corpus_dir() else {
        eprintln!("skipping: set DRACO_FBX_CORPUS to a directory of .fbx files");
        return;
    };

    let mut files = Vec::new();
    collect_fbx(&dir, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "the corpus directory holds no .fbx files"
    );

    let mut accepted = 0usize;
    let mut rejected: BTreeMap<String, usize> = BTreeMap::new();
    let mut containers: BTreeMap<String, usize> = BTreeMap::new();
    let mut versions: BTreeMap<String, usize> = BTreeMap::new();
    let mut features: BTreeMap<&'static str, usize> = BTreeMap::new();

    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        match FbxScene::from_bytes(&bytes) {
            Ok(scene) => {
                accepted += 1;
                let ascii = bytes.starts_with(b"; FBX") || bytes.starts_with(b";\t");
                tally(
                    &mut containers,
                    (if ascii { "ascii" } else { "binary" }).to_string(),
                );
                tally(&mut versions, declared_version(&bytes));
                let f = collect_features(&scene);
                if f.meshes > 0 {
                    bump(&mut features, "mesh");
                }
                for (present, name) in [
                    (f.joint_models, "joint models"),
                    (f.camera_or_light, "camera or light"),
                    (f.uv_sets, "uv sets"),
                    (f.color_sets, "color sets"),
                    (f.tangent_sets, "tangent sets"),
                    (f.binormal_sets, "binormal sets"),
                    (f.smoothing_or_crease, "smoothing or crease"),
                    (f.material_assignment, "material assignment"),
                    (f.edges, "edge sets"),
                    (f.skins, "skins"),
                    (f.blend_shapes, "blend shapes"),
                    (f.materials, "materials"),
                    (f.textures, "textures"),
                    (f.embedded_textures, "embedded textures"),
                    (f.animation, "animation"),
                    (f.read_warnings, "read warnings"),
                ] {
                    if present {
                        bump(&mut features, name);
                    }
                }
            }
            Err(error) => {
                tally(&mut rejected, rejection_kind(&error).to_string());
            }
        }
    }

    println!(
        "\nFBX feature census over {} files in {}\n",
        files.len(),
        dir.display()
    );
    println!("  accepted          {}", accepted);
    for (kind, count) in &rejected {
        println!("  rejected[{}] {}", kind, count);
    }
    if accepted > 0 {
        println!("\n  by container:");
        for (container, count) in &containers {
            println!(
                "    {:<8} {:>5} ({}%)",
                container,
                count,
                share(*count, accepted)
            );
        }
        println!("\n  by declared version:");
        for (version, count) in &versions {
            println!(
                "    {:<8} {:>5} ({}%)",
                version,
                count,
                share(*count, accepted)
            );
        }
        println!("\n  feature frequency over accepted files:");
        for (name, count) in &features {
            println!(
                "    {:<22} {:>5} ({}%)",
                name,
                count,
                share(*count, accepted)
            );
        }
    }

    assert!(
        accepted > 0,
        "no file in the corpus parsed; the reader is likely broken"
    );
}
