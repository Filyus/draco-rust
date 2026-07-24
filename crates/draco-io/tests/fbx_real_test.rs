//! End-to-end FBX reader/writer validation against real binary fixtures.
//!
//! These tests are gated behind the repository-only `test` feature and
//! self-disable when the fixture directory is absent, so they never break CI.

#![cfg(feature = "test")]

use std::path::PathBuf;

use draco_io::{FbxMemoryReader, FbxScene};

/// Local fixtures borrowed from the Three.js sample model collection. They are
/// intentionally not vendored into the crate; point this at your checkout.
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Three.ts/examples/models/fbx")
}

#[test]
fn morph_test_exposes_four_materials_and_indices() {
    let path = fixtures_dir().join("morph_test.fbx");
    if !std::path::Path::new(&path).exists() {
        eprintln!("skipping: {}", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read morph_test.fbx");
    let mut reader = FbxMemoryReader::from_bytes(bytes).expect("open morph_test.fbx");
    let scene = reader.read_scene().expect("parse morph_test.fbx");

    // The Three.js morph_test.fbx ships four Phong materials wired to the
    // geometry through LayerElementMaterial.
    assert!(
        scene.materials.len() >= 4,
        "expected at least 4 materials, got {}",
        scene.materials.len()
    );
    assert!(
        scene.materials.iter().any(|m| m
            .shading_model
            .as_deref()
            .map(|s| s.eq_ignore_ascii_case("phong"))
            .unwrap_or(false)),
        "expected at least one Phong material"
    );
    let has_indices = scene
        .root_nodes
        .iter()
        .any(|node| has_mesh_with_material_indices(node));
    assert!(
        has_indices,
        "expected at least one mesh instance with material indices"
    );
    assert!(
        scene.warnings.iter().any(|w| w.contains("blend shapes")),
        "blend shapes should be reported as skipped: {:?}",
        scene.warnings
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
    assert!(
        scene.warnings.iter().any(|w| w.contains("skin")),
        "skin deformers should be reported as skipped: {:?}",
        scene.warnings
    );
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
    // Spot-check that a TRS channel's first key value is preserved to f32
    // precision after the full round-trip.
    'outer: for (orig_clip, rt_clip) in original.animations.iter().zip(roundtrip.animations.iter())
    {
        for (orig, rt) in orig_clip.channels.iter().zip(rt_clip.channels.iter()) {
            if orig.sampler.output.len() >= 3 && rt.sampler.output.len() >= 3 {
                for c in 0..3 {
                    assert!(
                        (orig.sampler.output[c] - rt.sampler.output[c]).abs() < 1e-4,
                        "channel value mismatch: {} vs {}",
                        orig.sampler.output[c],
                        rt.sampler.output[c]
                    );
                }
                break 'outer;
            }
        }
    }
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
