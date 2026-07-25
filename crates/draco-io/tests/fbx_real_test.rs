//! End-to-end FBX reader/writer validation against real binary fixtures.
//!
//! These tests are gated behind the repository-only `test` feature and
//! self-disable when the fixture directory is absent, so they never break CI.

#![cfg(feature = "test")]

use std::path::PathBuf;

use draco_io::{FbxMemoryReader, FbxScene};

/// Local fixtures borrowed from the Three.js sample model collection. They are
/// intentionally not vendored into the crate.
///
/// Set `FBX_FIXTURES` to your checkout's `examples/models/fbx` directory, the
/// same variable the web probes use. The previous hard-coded relative path
/// resolved to `<repo>/Three.ts/...`, one level short of a sibling checkout, so
/// these tests silently skipped everywhere -- including for anyone who did have
/// the fixtures.
fn fixtures_dir() -> PathBuf {
    if let Ok(from_env) = std::env::var("FBX_FIXTURES") {
        return PathBuf::from(from_env);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../Three.ts/examples/models/fbx")
}

#[test]
fn morph_test_exposes_its_material_and_blend_shape() {
    let path = fixtures_dir().join("morph_test.fbx");
    if !std::path::Path::new(&path).exists() {
        eprintln!("skipping: {}", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read morph_test.fbx");
    let mut reader = FbxMemoryReader::from_bytes(bytes).expect("open morph_test.fbx");
    let scene = reader.read_scene().expect("parse morph_test.fbx");

    // This test previously claimed the file ships four Phong materials. It
    // ships exactly one, with no shading model -- the assertions were never
    // checked because the fixture path did not resolve.
    assert_eq!(
        scene.materials.len(),
        1,
        "morph_test.fbx has a single material"
    );
    let has_indices = scene.root_nodes.iter().any(has_mesh_with_material_indices);
    assert!(
        has_indices,
        "expected at least one mesh instance with material indices"
    );
    // This used to assert a "blend shapes were skipped" warning. Blend shapes
    // are imported now, so the warning is gone and the assertion was stale --
    // invisible because the fixture path never resolved. Assert the import.
    let morph_targets: usize = count_morph_targets(&scene);
    assert!(
        morph_targets > 0,
        "morph_test.fbx should expose blend-shape targets"
    );
}

#[test]
fn mixamo_exposes_node_trs_animation() {
    let path = fixtures_dir().join("mixamo.fbx");
    if !path.exists() {
        eprintln!("skipping: {}", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read mixamo.fbx");
    let mut reader = FbxMemoryReader::from_bytes(bytes).expect("open mixamo.fbx");
    let scene = reader.read_scene().expect("parse mixamo.fbx");

    assert!(
        !scene.animations.is_empty(),
        "expected at least one animation"
    );
    let clip = &scene.animations[0];
    assert!(
        clip.duration > 0.0,
        "expected positive clip duration, got {}",
        clip.duration
    );
    assert!(
        clip.channels.len() >= 50,
        "expected at least 50 channels, got {}",
        clip.channels.len()
    );
    for channel in &clip.channels {
        assert!(
            !channel.sampler.input.is_empty(),
            "channel {:?} should have keyframes",
            channel.node_name
        );
        assert_eq!(
            channel.sampler.output.len(),
            channel.sampler.input.len() * 3,
            "channel output should be {}*3 values",
            channel.sampler.input.len()
        );
    }
    // Likewise: skin deformers are imported rather than skipped.
    assert!(
        has_skinned_mesh(&scene),
        "mixamo.fbx should expose a skinned mesh"
    );
}

fn count_morph_targets(scene: &FbxScene) -> usize {
    fn visit(node: &draco_io::FbxSceneNode) -> usize {
        node.mesh_instances
            .iter()
            .map(|mesh| mesh.morph_targets.len())
            .sum::<usize>()
            + node.children.iter().map(visit).sum::<usize>()
    }
    scene.root_nodes.iter().map(visit).sum()
}

fn has_skinned_mesh(scene: &FbxScene) -> bool {
    fn visit(node: &draco_io::FbxSceneNode) -> bool {
        node.mesh_instances
            .iter()
            .any(|mesh| mesh.skin.as_ref().is_some_and(|s| !s.clusters.is_empty()))
            || node.children.iter().any(visit)
    }
    scene.root_nodes.iter().any(visit)
}

#[test]
fn mixamo_round_trips_animation_channels() {
    let path = fixtures_dir().join("mixamo.fbx");
    if !path.exists() {
        eprintln!("skipping: {}", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read mixamo.fbx");
    let original = FbxScene::from_bytes(&bytes).expect("parse mixamo.fbx");
    let rewritten = original.to_bytes().expect("write mixamo round-trip");
    let roundtrip = FbxScene::from_bytes(&rewritten).expect("re-parse mixamo round-trip");

    assert_eq!(
        original.materials.len(),
        roundtrip.materials.len(),
        "material count should round-trip"
    );
    assert_eq!(
        original.animations.len(),
        roundtrip.animations.len(),
        "animation count should round-trip"
    );
    let original_channels: usize = original.animations.iter().map(|a| a.channels.len()).sum();
    let roundtrip_channels: usize = roundtrip.animations.iter().map(|a| a.channels.len()).sum();
    assert_eq!(
        original_channels, roundtrip_channels,
        "channel count should round-trip"
    );
    assert_eq!(
        mesh_source_stats(&original),
        mesh_source_stats(&roundtrip),
        "control points, polygon corners, and UV layers should round-trip"
    );
    // Match channels by what they drive, not by position. The writer assigns
    // its own object ids, and channels are ordered by id, so a rewrite
    // legitimately reorders them; only the set and the values must survive.
    for (orig_clip, rt_clip) in original.animations.iter().zip(roundtrip.animations.iter()) {
        for orig in &orig_clip.channels {
            let rt = rt_clip
                .channels
                .iter()
                .find(|candidate| {
                    candidate.node_name == orig.node_name
                        && candidate.path == orig.path
                        && candidate.morph_target_index == orig.morph_target_index
                })
                .unwrap_or_else(|| {
                    panic!(
                        "channel {} {:?} disappeared across the round-trip",
                        orig.node_name, orig.path
                    )
                });
            assert_eq!(
                orig.sampler.output.len(),
                rt.sampler.output.len(),
                "channel {} {:?} changed key count",
                orig.node_name,
                orig.path
            );
            for (before, after) in orig.sampler.output.iter().zip(rt.sampler.output.iter()) {
                assert!(
                    (before - after).abs() < 1e-4,
                    "channel {} {:?} value mismatch: {before} vs {after}",
                    orig.node_name,
                    orig.path
                );
            }
        }
    }
}

/// Two reads of the same bytes must produce identical scenes.
///
/// Animation channels used to be ordered by `HashMap` iteration, so this
/// varied per process and made any positional comparison unreliable.
#[test]
fn reading_the_same_file_twice_is_deterministic() {
    let path = fixtures_dir().join("mixamo.fbx");
    if !path.exists() {
        eprintln!("skipping: {}", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read mixamo.fbx");
    let first = FbxScene::from_bytes(&bytes).expect("first parse");
    let second = FbxScene::from_bytes(&bytes).expect("second parse");

    // Bind poses had the same problem as animation channels: a file with more
    // than one `Pose` resolved a node's matrix from whichever pose hash order
    // reached it first. mixamo.fbx has two.
    let bind_poses = |scene: &FbxScene| -> Vec<String> {
        fn visit(node: &draco_io::FbxSceneNode, out: &mut Vec<String>) {
            for mesh in &node.mesh_instances {
                for (id, transform) in mesh.skin.iter().flat_map(|skin| &skin.bind_pose) {
                    out.push(format!("{id:?}|{:?}", transform.matrix));
                }
            }
            for child in &node.children {
                visit(child, out);
            }
        }
        let mut out = Vec::new();
        for root in &scene.root_nodes {
            visit(root, &mut out);
        }
        out
    };
    assert_eq!(
        bind_poses(&first),
        bind_poses(&second),
        "bind pose resolution must not depend on hash iteration"
    );

    let channel_order = |scene: &FbxScene| -> Vec<String> {
        scene
            .animations
            .iter()
            .flat_map(|clip| {
                clip.channels
                    .iter()
                    .map(|c| format!("{}|{:?}|{:?}", c.node_name, c.path, c.morph_target_index))
            })
            .collect()
    };
    assert_eq!(
        channel_order(&first),
        channel_order(&second),
        "animation channel order must not depend on hash iteration"
    );
}

fn mesh_source_stats(scene: &FbxScene) -> Vec<(String, usize, usize, usize)> {
    fn visit(node: &draco_io::FbxSceneNode, output: &mut Vec<(String, usize, usize, usize)>) {
        for mesh in &node.mesh_instances {
            output.push((
                mesh.name.clone().unwrap_or_default(),
                mesh.control_points.len(),
                mesh.polygon_vertex_indices.len(),
                mesh.uv_sets.len(),
            ));
        }
        for child in &node.children {
            visit(child, output);
        }
    }
    let mut output = Vec::new();
    for root in &scene.root_nodes {
        visit(root, &mut output);
    }
    output.sort();
    output
}

fn has_mesh_with_material_indices(node: &draco_io::FbxSceneNode) -> bool {
    node.mesh_instances
        .iter()
        .any(|mesh| !mesh.material_indices.is_empty())
        || node.children.iter().any(has_mesh_with_material_indices)
}
