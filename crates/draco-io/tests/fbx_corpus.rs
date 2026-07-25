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

/// Writing a decoded scene back out and reading it again must preserve the
/// semantic content the reader exposes.
///
/// This is what turns the corpus into a bug finder rather than a crash
/// detector: it caught per-polygon material assignments being destroyed on
/// n-gon meshes, because `LayerElementMaterial` is ByPolygon while the
/// in-memory indices are per triangle.
#[test]
fn scenes_survive_a_write_and_read_cycle() {
    let Some(dir) = corpus_dir() else {
        eprintln!("skipping: set DRACO_FBX_CORPUS to a directory of .fbx files");
        return;
    };

    let mut files = Vec::new();
    collect_fbx(&dir, &mut files);
    files.sort();

    let mut compared = 0;
    let mut mismatches: Vec<String> = Vec::new();
    for path in &files {
        // Skip the fuzz corpus: those are deliberately corrupt.
        if path.components().any(|c| c.as_os_str() == "fuzz") {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let Ok(original) = FbxScene::from_bytes(&bytes) else {
            continue;
        };
        // A write that fails used to be skipped in silence, which meant a
        // writer that refused a whole class of file looked like a clean pass.
        let written = match original.to_bytes() {
            Ok(written) => written,
            Err(error) => {
                mismatches.push(format!("{}: write failed: {error}", path.display()));
                continue;
            }
        };
        let Ok(roundtrip) = FbxScene::from_bytes(&written) else {
            mismatches.push(format!(
                "{}: rewritten file does not read back",
                path.display()
            ));
            continue;
        };
        compared += 1;

        let before = summarize(&original);
        let after = summarize(&roundtrip);
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        for field in before.differing_fields(&after) {
            mismatches.push(format!("{name}: {field}"));
        }
    }

    println!(
        "round-tripped {compared} files, {} mismatched",
        mismatches.len()
    );
    for line in mismatches.iter().take(20) {
        println!("  {line}");
    }
    assert!(
        mismatches.is_empty(),
        "{} files changed across a write/read cycle",
        mismatches.len()
    );
}

/// Everything a round-trip must preserve, rendered as comparable strings.
///
/// Kept as text so a mismatch report names the field that moved instead of
/// dumping two large structs.
/// The per-mesh counts a round-trip must preserve.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MeshSummary {
    control_points: usize,
    polygon_corners: usize,
    uv_sets: usize,
    color_sets: usize,
    edges: usize,
    material_indices: Vec<i32>,
    /// `(component values, handedness present)` per tangent layer. The flag is
    /// checked because the writer must not invent a `TangentsW` array for a
    /// pre-7500 document that never had one, nor drop one that did.
    tangent_sets: Vec<(usize, bool)>,
    binormal_sets: Vec<(usize, bool)>,
    /// `(mapping, value count)` per smoothing layer, and per crease layer with
    /// its kind. Values are compared by count rather than element-wise because
    /// the round-trip check is about preservation, not about arithmetic.
    smoothing_layers: Vec<(String, usize)>,
    crease_layers: Vec<(String, String, usize)>,
}

#[derive(Debug, PartialEq, Eq)]
struct SceneSummary {
    materials: usize,
    textures: usize,
    animations: usize,
    meshes: Vec<MeshSummary>,
    /// `(name, cluster count, total weights, first bind matrix)` per skin.
    skins: Vec<String>,
    /// `(name, affected points, delta count, weights)` per blend-shape target.
    morphs: Vec<String>,
    /// `(node, path, key count, first and last key)` per animation channel.
    channels: Vec<String>,
    /// Node transforms, flattened.
    transforms: Vec<String>,
    /// Material scalar and colour properties.
    material_values: Vec<String>,
    /// Texture filenames and embedded payload sizes.
    texture_values: Vec<String>,
    /// Source axis, unit and time settings.
    global_settings: String,
    /// The authored Model transform stack: pivots, pre/post rotation, inherit.
    transform_stacks: Vec<String>,
    /// Cubic tangent payloads, which no external importer can verify for us:
    /// Blender downgrades cubic keys to linear on import.
    tangents: Vec<String>,
    /// Camera and light attributes, per node.
    attributes: Vec<String>,
}

fn summarize(scene: &FbxScene) -> SceneSummary {
    fn visit(node: &draco_io::FbxSceneNode, out: &mut Vec<MeshSummary>) {
        for mesh in &node.mesh_instances {
            // A Geometry with neither vertices nor polygons carries nothing
            // but a name. The writer emits no geometry nodes for it, so it
            // does not come back; that is accepted rather than padded out
            // with empty arrays other importers would have to interpret.
            if mesh.control_points.is_empty() && mesh.polygon_vertex_indices.is_empty() {
                continue;
            }
            out.push(MeshSummary {
                control_points: mesh.control_points.len(),
                polygon_corners: mesh.polygon_vertex_indices.len(),
                uv_sets: mesh.layers.uv_sets.len(),
                color_sets: mesh.layers.color_sets.len(),
                edges: mesh.edges.len(),
                material_indices: mesh.material_indices.clone(),
                tangent_sets: mesh
                    .layers
                    .tangent_sets
                    .iter()
                    .map(|set| (set.layer.values.len(), set.has_handedness))
                    .collect(),
                binormal_sets: mesh
                    .layers
                    .binormal_sets
                    .iter()
                    .map(|set| (set.layer.values.len(), set.has_handedness))
                    .collect(),
                smoothing_layers: mesh
                    .layers
                    .smoothing_layers
                    .iter()
                    .map(|layer| {
                        (
                            layer.mapping.clone().unwrap_or_default(),
                            layer.values.len(),
                        )
                    })
                    .collect(),
                crease_layers: mesh
                    .layers
                    .crease_layers
                    .iter()
                    .map(|layer| {
                        (
                            format!("{:?}", layer.kind),
                            layer.mapping.clone().unwrap_or_default(),
                            layer.values.len(),
                        )
                    })
                    .collect(),
            });
        }
        for child in &node.children {
            visit(child, out);
        }
    }
    let mut meshes = Vec::new();
    for root in &scene.root_nodes {
        visit(root, &mut meshes);
    }
    meshes.sort();

    let mut skins = Vec::new();
    let mut morphs = Vec::new();
    let mut transforms = Vec::new();
    let mut transform_stacks = Vec::new();
    let mut attributes = Vec::new();
    collect_deformers(
        scene,
        &mut skins,
        &mut morphs,
        &mut transforms,
        &mut transform_stacks,
        &mut attributes,
    );
    skins.sort();
    morphs.sort();
    transforms.sort();
    transform_stacks.sort();
    attributes.sort();

    let mut channels: Vec<String> = scene
        .animations
        .iter()
        .flat_map(|clip| {
            clip.channels.iter().map(move |channel| {
                let output = &channel.sampler.output;
                format!(
                    "{}|{:?}|{:?}|keys={}|first={:?}|last={:?}|interp={:?}",
                    channel.node_name,
                    channel.path,
                    channel.morph_target_index,
                    channel.sampler.input.len(),
                    output.first().copied().map(milli),
                    output.last().copied().map(milli),
                    channel.sampler.interpolation,
                )
            })
        })
        .collect();
    channels.sort();

    let material_values = scene
        .materials
        .iter()
        .map(|material| {
            format!(
                "{:?}|{:?}|diffuse={:?}|specular={:?}|emissive={:?}|shininess={:?}|opacity={:?}|textures={:?}",
                material.name,
                material.shading_model,
                material.diffuse.map(milli3),
                material.specular.map(milli3),
                material.emissive.map(milli3),
                material.shininess.map(milli),
                material.opacity.map(milli),
                material.textures,
            )
        })
        .collect();

    let texture_values = scene
        .textures
        .iter()
        .map(|texture| {
            format!(
                "{:?}|{:?}|content={}",
                texture.name,
                texture.filename,
                texture.content.as_ref().map_or(0, Vec::len)
            )
        })
        .collect();

    let mut tangents: Vec<String> = scene
        .animations
        .iter()
        .flat_map(|clip| {
            clip.channels.iter().filter_map(move |channel| {
                let sampler = &channel.sampler;
                let (Some(incoming), Some(outgoing)) =
                    (&sampler.in_tangents, &sampler.out_tangents)
                else {
                    return None;
                };
                Some(format!(
                    "{}|{:?}|in={}:{:?}|out={}:{:?}",
                    channel.node_name,
                    channel.path,
                    incoming.len(),
                    incoming.first().copied().map(milli),
                    outgoing.len(),
                    outgoing.first().copied().map(milli),
                ))
            })
        })
        .collect();
    tangents.sort();

    SceneSummary {
        materials: scene.materials.len(),
        textures: scene.textures.len(),
        animations: scene.animations.len(),
        meshes,
        skins,
        morphs,
        channels,
        transforms,
        material_values,
        texture_values,
        global_settings: format!("{:?}", scene.global_settings),
        transform_stacks,
        tangents,
        attributes,
    }
}

impl SceneSummary {
    /// Names each field that differs, with a short sample, so a report points
    /// at the defect instead of dumping two whole scenes.
    fn differing_fields(&self, other: &Self) -> Vec<String> {
        let mut out = Vec::new();
        let mut scalar = |label: &str, a: usize, b: usize| {
            if a != b {
                out.push(format!("{label}: {a} -> {b}"));
            }
        };
        scalar("materials", self.materials, other.materials);
        scalar("textures", self.textures, other.textures);
        scalar("animations", self.animations, other.animations);

        // A scene with no `GlobalSettings` gets the writer's defaults, which
        // then read back as present. That is the writer completing a section
        // every consumer expects rather than inventing content: exporting the
        // Fox glTF and importing the result into Blender reproduces the source
        // bounding box exactly, so the declared axes and the geometry agree.
        // Only a *change* to settings the source declared is a defect.
        if self.global_settings != other.global_settings && self.global_settings != "None" {
            out.push(format!(
                "global_settings:
       before {}
       after  {}",
                self.global_settings, other.global_settings
            ));
        }
        if self.meshes != other.meshes {
            out.push(format!(
                "meshes: {} -> {} entries",
                self.meshes.len(),
                other.meshes.len()
            ));
        }
        for (label, a, b) in [
            ("skins", &self.skins, &other.skins),
            ("morphs", &self.morphs, &other.morphs),
            ("channels", &self.channels, &other.channels),
            ("transforms", &self.transforms, &other.transforms),
            (
                "material_values",
                &self.material_values,
                &other.material_values,
            ),
            (
                "texture_values",
                &self.texture_values,
                &other.texture_values,
            ),
            (
                "transform_stacks",
                &self.transform_stacks,
                &other.transform_stacks,
            ),
            ("tangents", &self.tangents, &other.tangents),
            ("attributes", &self.attributes, &other.attributes),
        ] {
            if a == b {
                continue;
            }
            let sample = a
                .iter()
                .zip(b.iter())
                .find(|(x, y)| x != y)
                .map(|(x, y)| {
                    format!(
                        "
       before {x}
       after  {y}"
                    )
                })
                .unwrap_or_else(|| format!(" ({} -> {} entries)", a.len(), b.len()));
            out.push(format!("{label}:{sample}"));
        }
        out
    }
}

/// Quantizes to thousandths, a tolerance that survives an f32 write/read
/// cycle. Named for what it does: the earlier name said six decimals and gave
/// three, which is exactly the wrong thing to misread while chasing a
/// precision difference.
fn milli(value: f32) -> i64 {
    (value as f64 * 1e3).round() as i64
}

fn milli3(values: [f32; 3]) -> [i64; 3] {
    values.map(|v| (v as f64 * 1e3).round() as i64)
}

/// Quantizes a matrix to ten-thousandths -- finer than [`milli`], and fine
/// enough that a ~1e-7 relative difference in a translation can still flip a
/// digit. `maya_human_ik_7400` is excluded for exactly that.
fn matrix_digest(matrix: &[[f32; 4]; 4]) -> Vec<i64> {
    matrix
        .iter()
        .flatten()
        .map(|v| (*v as f64 * 1e4).round() as i64)
        .collect()
}

/// Maps every node id to its name, so skin joints can be compared by identity
/// rather than by the document-local id the writer reassigns.
fn node_names(scene: &FbxScene) -> std::collections::HashMap<u32, String> {
    fn visit(node: &draco_io::FbxSceneNode, out: &mut std::collections::HashMap<u32, String>) {
        out.insert(node.id.0, node.name.clone().unwrap_or_default());
        for child in &node.children {
            visit(child, out);
        }
    }
    let mut out = std::collections::HashMap::new();
    for root in &scene.root_nodes {
        visit(root, &mut out);
    }
    out
}

fn collect_deformers(
    scene: &FbxScene,
    skins: &mut Vec<String>,
    morphs: &mut Vec<String>,
    transforms: &mut Vec<String>,
    transform_stacks: &mut Vec<String>,
    attributes: &mut Vec<String>,
) {
    let names = node_names(scene);
    fn visit(
        node: &draco_io::FbxSceneNode,
        names: &std::collections::HashMap<u32, String>,
        skins: &mut Vec<String>,
        morphs: &mut Vec<String>,
        transforms: &mut Vec<String>,
        transform_stacks: &mut Vec<String>,
        attributes: &mut Vec<String>,
    ) {
        if let Some(attribute) = &node.attribute {
            attributes.push(describe_attribute(node.name.as_deref(), attribute));
        }
        transform_stacks.push(format!("{:?}|{:?}", node.name, node.transform_stack));
        transforms.push(format!(
            "{:?}|{:?}|complex={}",
            node.name,
            node.transform.as_ref().map(|t| matrix_digest(&t.matrix)),
            node.has_complex_transform_stack,
        ));
        for mesh in &node.mesh_instances {
            if let Some(skin) = &mesh.skin {
                let weights: usize = skin.clusters.iter().map(|c| c.weights.len()).sum();
                let joints: Vec<&str> = skin
                    .clusters
                    .iter()
                    .map(|c| {
                        names
                            .get(&c.joint_node_id.0)
                            .map_or("<unknown>", String::as_str)
                    })
                    .collect();
                skins.push(format!(
                    "{:?}|clusters={}|weights={weights}|joints={joints:?}|bind={}|first_link={:?}",
                    mesh.name,
                    skin.clusters.len(),
                    skin.bind_pose.len(),
                    skin.clusters
                        .first()
                        .map(|c| matrix_digest(&c.joint_bind_transform.matrix)),
                ));
            }
            for target in &mesh.morph_targets {
                morphs.push(format!(
                    "{:?}|{:?}|points={}|deltas={}|normals={:?}|default={}|full={}",
                    mesh.name,
                    target.name,
                    target.control_point_indices.len(),
                    target.position_deltas.len(),
                    target.normal_deltas.as_ref().map(Vec::len),
                    milli(target.default_weight),
                    milli(target.full_weight),
                ));
            }
        }
        for child in &node.children {
            visit(
                child,
                names,
                skins,
                morphs,
                transforms,
                transform_stacks,
                attributes,
            );
        }
    }
    for root in &scene.root_nodes {
        visit(
            root,
            &names,
            skins,
            morphs,
            transforms,
            transform_stacks,
            attributes,
        );
    }
}

/// Renders one camera or light for comparison.
///
/// Written out field by field with the same quantization the other float
/// comparisons use. A `{:?}` of the struct would compare raw `f32`s and report
/// a difference for values like `field_of_view = 49.134342` that survive a
/// write and read perfectly well.
fn describe_attribute(node: Option<&str>, attribute: &draco_io::FbxNodeAttribute) -> String {
    match attribute {
        draco_io::FbxNodeAttribute::Camera(camera) => format!(
            "{node:?}|camera|position={:?}|interest={:?}|up={:?}|projection={:?}|fov={:?},{:?},{:?}             |focal={:?}|near={:?}|far={:?}|aspect={:?},{:?}|zoom={:?}",
            camera.position.map(milli3),
            camera.interest_position.map(milli3),
            camera.up_vector.map(milli3),
            camera.projection_type,
            camera.field_of_view.map(milli),
            camera.field_of_view_x.map(milli),
            camera.field_of_view_y.map(milli),
            camera.focal_length.map(milli),
            camera.near_plane.map(milli),
            camera.far_plane.map(milli),
            camera.aspect_width.map(milli),
            camera.aspect_height.map(milli),
            camera.ortho_zoom.map(milli),
        ),
        draco_io::FbxNodeAttribute::Light(light) => format!(
            "{node:?}|light|type={:?}|colour={:?}|intensity={:?}|cast={:?},{:?}|decay={:?},{:?}",
            light.light_type,
            light.color.map(milli3),
            light.intensity.map(milli),
            light.cast_light,
            light.cast_shadows,
            light.decay_type,
            light.decay_start.map(milli),
        ),
        other => format!("{node:?}|{other:?}"),
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

/// A document that decodes to nothing must say why.
///
/// Pre-7000 FBX keys its objects and connections by name rather than by id and
/// puts geometry on the `Model`, none of which this reader understands. It
/// still returns a structurally valid scene, so without a warning the result is
/// indistinguishable from a file that genuinely has no meshes. Every such file
/// in the corpus is checked to raise the notice, and every file that raises it
/// is checked to really be empty -- otherwise the warning is the thing that is
/// wrong.
#[test]
fn a_pre_7000_document_says_why_it_decoded_to_nothing() {
    let Some(dir) = corpus_dir() else {
        eprintln!("skipping: set DRACO_FBX_CORPUS to a directory of .fbx files");
        return;
    };
    let mut files = Vec::new();
    collect_fbx(&dir, &mut files);
    files.sort();

    let mut warned = 0usize;
    let mut silent_and_empty = Vec::new();
    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if !bytes.starts_with(b"Kaydara FBX Binary") {
            continue;
        }
        let Ok(scene) = FbxScene::from_bytes(&bytes) else {
            continue;
        };
        let says_why = scene
            .warnings
            .iter()
            .any(|w| w.code == draco_io::FbxWarningCode::NameKeyedObjectModel);
        // Byte 23 holds the version in the little-endian profile.
        let version = u32::from_le_bytes([bytes[23], bytes[24], bytes[25], bytes[26]]);
        let pre_7000 = version < 7000;

        if says_why {
            warned += 1;
            assert_eq!(
                control_point_total(&scene),
                0,
                "{} warns that nothing was imported yet decoded geometry",
                path.display()
            );
        } else if pre_7000 {
            silent_and_empty.push(path.clone());
        }
    }

    assert!(
        silent_and_empty.is_empty(),
        "pre-7000 documents decoded without explanation: {silent_and_empty:?}"
    );
    assert!(warned > 0, "no pre-7000 documents in the corpus to check");
    println!("{warned} pre-7000 documents each explained why they are empty");
}

/// Every camera and light in the corpus survives a rewrite.
///
/// `scenes_survive_a_write_and_read_cycle` compares attribute values through
/// `SceneSummary` already. This asserts the cruder thing that summary cannot:
/// that the count does not drop. A writer that emitted an attribute with every
/// property missing would satisfy a value comparison over an empty set.
#[test]
fn cameras_and_lights_survive_a_rewrite() {
    let Some(dir) = corpus_dir() else {
        eprintln!("skipping: set DRACO_FBX_CORPUS to a directory of .fbx files");
        return;
    };
    let mut files = Vec::new();
    collect_fbx(&dir, &mut files);
    files.sort();

    fn count(node: &draco_io::FbxSceneNode) -> usize {
        usize::from(node.attribute.is_some()) + node.children.iter().map(count).sum::<usize>()
    }
    fn total(scene: &FbxScene) -> usize {
        scene.root_nodes.iter().map(count).sum()
    }

    let mut with_attributes = 0usize;
    let mut attributes = 0usize;
    for path in &files {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if !bytes.starts_with(b"Kaydara FBX Binary") {
            continue;
        }
        let Ok(scene) = FbxScene::from_bytes(&bytes) else {
            continue;
        };
        let read = total(&scene);
        if read == 0 {
            continue;
        }
        with_attributes += 1;
        attributes += read;

        let rewritten = scene
            .to_bytes()
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let reread = FbxScene::from_bytes(&rewritten).expect("rewrite must still parse");
        assert_eq!(
            total(&reread),
            read,
            "{} lost node attributes through a rewrite",
            path.display()
        );
    }

    assert!(
        with_attributes > 0,
        "no camera or light attributes in the corpus to check"
    );
    println!(
        "{attributes} camera/light attributes survived a rewrite across {with_attributes} files"
    );
}

/// The Blender 2.79 default scene has one camera and one light with values a
/// person can check by eye, which the aggregate counts above cannot.
///
/// Camera and light properties are all optional and read by name, so a typo in
/// a property name yields `None` rather than a failure. Asserting concrete
/// values is the only thing that catches that.
#[test]
fn the_blender_default_scene_camera_and_light_read_their_authored_values() {
    let Some(dir) = corpus_dir() else {
        eprintln!("skipping: set DRACO_FBX_CORPUS to a directory of .fbx files");
        return;
    };
    let path = dir.join("blender_279_default_7400_binary.fbx");
    if !path.exists() {
        eprintln!("skipping: {} is not in this corpus", path.display());
        return;
    }
    let scene = FbxScene::from_bytes(&std::fs::read(&path).unwrap()).unwrap();

    let mut camera = None;
    let mut light = None;
    fn visit(
        node: &draco_io::FbxSceneNode,
        camera: &mut Option<draco_io::FbxCamera>,
        light: &mut Option<draco_io::FbxLight>,
    ) {
        match &node.attribute {
            Some(draco_io::FbxNodeAttribute::Camera(value)) => *camera = Some(value.clone()),
            Some(draco_io::FbxNodeAttribute::Light(value)) => *light = Some(value.clone()),
            _ => {}
        }
        for child in &node.children {
            visit(child, camera, light);
        }
    }
    for root in &scene.root_nodes {
        visit(root, &mut camera, &mut light);
    }

    let camera = camera.expect("the default scene has a camera");
    assert_eq!(camera.focal_length, Some(35.0));
    assert_eq!(camera.aspect_width, Some(1920.0));
    assert_eq!(camera.aspect_height, Some(1080.0));
    assert!((camera.field_of_view.unwrap() - 49.134_342).abs() < 1e-3);
    assert!((camera.near_plane.unwrap() - 0.1).abs() < 1e-3);
    assert!((camera.far_plane.unwrap() - 100.0).abs() < 1e-2);
    let position = camera.position.expect("camera position");
    assert!((position[0] - 7.481_131).abs() < 1e-3, "{position:?}");

    let light = light.expect("the default scene has a light");
    // 0 is a point light, 2 quadratic decay.
    assert_eq!(light.light_type, Some(0));
    assert_eq!(light.decay_type, Some(2));
    assert_eq!(light.intensity, Some(100.0));
    assert_eq!(light.color, Some([1.0, 1.0, 1.0]));
    assert_eq!(light.cast_shadows, Some(true));
}

/// An ASCII document must decode to the same scene as its binary twin.
///
/// This is the strongest check available for the ASCII container, and the same
/// shape as the big-endian differential: it needs no ground truth, only two
/// spellings of one document. It is what caught object ids narrow enough to
/// fit in `i32`, animation curves stored at a different float width, and bare
/// enum tokens -- none of which produce an error, only a quietly smaller scene.
///
/// Six documents are excluded by exact file name, none of them a defect. Four
/// of the six are not pairs at all: the two exports were taken at different
/// points on the timeline, so one of them baked a frame's transform into the
/// static `Model` while the other did not. `examples/fbx_twin_diff.rs` prints
/// the same comparison for one pair on demand, which is how each was
/// established and how the next one should be.
#[test]
fn an_ascii_document_decodes_like_its_binary_twin() {
    /// Documents excluded by exact name, each with the difference observed.
    ///
    /// Matched by equality rather than by prefix: a family name would exclude
    /// every version of that scene, and `maya_auto_clamp_7100` decodes
    /// identically even though `maya_auto_clamp_7700` does not.
    const NOT_COMPARABLE: [&str; 6] = [
        // ASCII spells both `"` and a literal `&quot;` the same way, so this
        // file cannot survive the container in either direction.
        "max_quote_7500_ascii.fbx",
        // Not a pair: the binary file has no `Model` objects at all.
        "motionbuilder_actor_7700_ascii.fbx",
        // Not a pair: the ASCII `Model` carries `Lcl Translation` = 0.7466079
        // and the binary one carries no transform properties at all. The value
        // is `KeyValueFloat[1]` of the take, i.e. one frame of the animation
        // baked into the static transform of that export.
        "maya_auto_clamp_7700_ascii.fbx",
        // Not a pair, twice over: the curve holds 30 keys in ASCII against 24
        // in binary, and here it is the binary `Model` that carries the baked
        // `Lcl Translation` (-1.6999689).
        "maya_resampled_7500_ascii.fbx",
        // Not a pair: the ASCII `Model` carries baked `Lcl Translation`,
        // `Lcl Rotation` and `Lcl Scaling`; the binary one carries none.
        "maya_transform_animation_7500_ascii.fbx",
        // Precision, and unrecoverable. This ASCII export prints `f64` with 15
        // significant digits where 17 are needed to round-trip, so three of
        // the 1731 transform components land on the other side of an f32
        // rounding tie: 10.707250595 against 10.707249641, ~1e-7 relative.
        // That is above what `transform_stacks` (raw `{:?}`) and
        // `matrix_digest` (1e-4, and this straddles a .5) tolerate. The digits
        // are gone from the file; no reader can recover them.
        "maya_human_ik_7400_ascii.fbx",
    ];

    let Some(dir) = corpus_dir() else {
        eprintln!("skipping: set DRACO_FBX_CORPUS to a directory of .fbx files");
        return;
    };
    let mut files = Vec::new();
    collect_fbx(&dir, &mut files);
    files.sort();

    let mut compared = 0usize;
    let mut mismatched = Vec::new();
    for path in &files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if !name.ends_with("_ascii.fbx") || NOT_COMPARABLE.contains(&name.as_str()) {
            continue;
        }
        let twin = path.with_file_name(name.replace("_ascii.fbx", "_binary.fbx"));
        let (Ok(ascii_bytes), Ok(binary_bytes)) = (std::fs::read(path), std::fs::read(&twin))
        else {
            continue;
        };
        // Pre-7000 uses the name-keyed object model and is refused in both.
        if binary_bytes.len() < 27
            || u32::from_le_bytes([
                binary_bytes[23],
                binary_bytes[24],
                binary_bytes[25],
                binary_bytes[26],
            ]) < 7000
        {
            continue;
        }

        let binary = FbxScene::from_bytes(&binary_bytes)
            .unwrap_or_else(|error| panic!("{}: {error}", twin.display()));
        let ascii = FbxScene::from_bytes(&ascii_bytes)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        compared += 1;

        // Materials and textures are ordered by FBX object id, and the two
        // exports of one scene assign different ids. That makes both the list
        // order and the `texture_index` inside a material's bindings differ
        // for reasons that have nothing to do with the container, so these two
        // fields are compared as sets of names only. Everything else --
        // geometry, layers, skins, morphs, transforms, animation -- is
        // compared in full and positionally.
        let mut expected = summarize(&binary);
        let mut actual = summarize(&ascii);
        for summary in [&mut expected, &mut actual] {
            summary.material_values = names_only(&summary.material_values);
            summary.texture_values = names_only(&summary.texture_values);
        }
        if expected != actual {
            mismatched.push(format!("{name}: {:?}", expected.differing_fields(&actual)));
        }
    }

    assert!(
        compared > 0,
        "no ascii/binary pairs found under {}",
        dir.display()
    );
    assert!(
        mismatched.is_empty(),
        "{} of {compared} ascii documents decoded differently from their binary twin:\n{}",
        mismatched.len(),
        mismatched.join("\n")
    );
    println!("{compared} ascii documents decoded identically to their binary twin");
}

/// Keeps only the leading name of each summary entry, dropping the fields that
/// reference other objects by position.
fn names_only(entries: &[String]) -> Vec<String> {
    let mut names: Vec<String> = entries
        .iter()
        .map(|entry| entry.split('|').next().unwrap_or_default().to_string())
        .collect();
    names.sort();
    names
}
