//! Loads Draco-compressed files through Bevy's real asset pipeline.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use bevy::asset::LoadState;
use bevy::gltf::{GltfAssetLabel, GltfLoaderSettings};
use bevy::mesh::{Mesh, VertexAttributeValues};
use bevy::prelude::*;
use bevy_draco::{draco_loader_settings, DracoGltfPlugin};

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata")
        .join(path)
}

/// An asset directory holding the fixtures plus a copy of `Box_Draco.glb`
/// whose Draco stream is overwritten after its magic.
///
/// Built once: the tests run in parallel and would otherwise rewrite files
/// another test is reading.
fn asset_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(build_asset_dir)
}

fn build_asset_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bevy_draco-assets");
    std::fs::create_dir_all(&dir).unwrap();
    let draco = std::fs::read(fixture("Box/glTF_Binary/Box_Draco.glb")).unwrap();
    std::fs::write(dir.join("Box_Draco.glb"), &draco).unwrap();
    std::fs::copy(fixture("Box/glTF_Binary/Box.glb"), dir.join("Box.glb")).unwrap();

    let mut corrupt = draco;
    let stream = corrupt
        .windows(5)
        .position(|window| window == b"DRACO")
        .expect("fixture carries a Draco stream");
    // Keep the header, destroy what follows: the stream then promises
    // geometry it cannot deliver.
    for byte in &mut corrupt[stream + 11..stream + 60] {
        *byte = 0xff;
    }
    std::fs::write(dir.join("Box_Corrupt.glb"), corrupt).unwrap();
    dir
}

fn app(with_draco: bool) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin {
            file_path: asset_dir().to_string_lossy().into_owned(),
            ..Default::default()
        },
        bevy::image::ImagePlugin::default(),
    ));
    // In front of `GltfPlugin` on purpose: registration must not depend on
    // plugin order.
    if with_draco {
        app.add_plugins(DracoGltfPlugin::default());
    }
    app.add_plugins(bevy::gltf::GltfPlugin::default());
    app.init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<bevy::animation::AnimationClip>()
        .init_asset::<bevy::world_serialization::WorldAsset>();
    app.finish();
    app.cleanup();
    app
}

/// Pumps `app` until primitive 0 of mesh 0 of `file` has loaded or failed.
fn load_primitive(
    app: &mut App,
    file: &str,
    settings: impl Fn(&mut GltfLoaderSettings) + Send + Sync + 'static,
) -> Option<Mesh> {
    let handle: Handle<Mesh> = app
        .world()
        .resource::<AssetServer>()
        .load_builder()
        .with_settings(settings)
        .load(
            GltfAssetLabel::Primitive {
                mesh: 0,
                primitive: 0,
            }
            .from_asset(file.to_owned()),
        );
    for _ in 0..2000 {
        app.update();
        if let Some(mesh) = app.world().resource::<Assets<Mesh>>().get(&handle) {
            return Some(mesh.clone());
        }
        if let Some(LoadState::Failed(_)) = app
            .world()
            .resource::<AssetServer>()
            .get_load_state(&handle)
        {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("{file} did not finish loading");
}

fn positions(mesh: &Mesh) -> Vec<[f32; 3]> {
    match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(values)) => values.clone(),
        other => panic!("positions are {other:?}"),
    }
}

/// Resolves indices to triangles of positions, rounded to absorb Draco's
/// quantization and sorted to absorb its vertex reordering.
fn triangles(mesh: &Mesh) -> Vec<[[i32; 3]; 3]> {
    let positions = positions(mesh);
    let indices: Vec<usize> = mesh.indices().expect("indexed").iter().collect();
    let mut triangles: Vec<_> = indices
        .chunks_exact(3)
        .map(|triangle| {
            let mut corners =
                [0, 1, 2].map(|i| positions[triangle[i]].map(|v| (v * 1000.0).round() as i32));
            // Rotating a triangle's corners does not change it.
            let first = (0..3).min_by_key(|&i| corners[i]).unwrap();
            corners.rotate_left(first);
            corners
        })
        .collect();
    triangles.sort();
    triangles
}

#[test]
fn draco_box_matches_uncompressed_box() {
    let mut app = app(true);
    let draco =
        load_primitive(&mut app, "Box_Draco.glb", draco_loader_settings).expect("Draco box loads");
    let plain = load_primitive(&mut app, "Box.glb", |_| {}).expect("plain box loads");

    assert_eq!(draco.count_vertices(), plain.count_vertices());
    assert!(draco.attribute(Mesh::ATTRIBUTE_NORMAL).is_some());
    // 24 vertices fit in u16, as they do in the uncompressed file.
    assert!(matches!(draco.indices(), Some(bevy::mesh::Indices::U16(_))));
    assert_eq!(triangles(&draco), triangles(&plain));
}

#[test]
fn corrupt_stream_yields_empty_primitive_instead_of_panicking() {
    let mut app = app(true);
    let mesh = load_primitive(&mut app, "Box_Corrupt.glb", draco_loader_settings)
        .expect("the file itself still loads");
    assert_eq!(mesh.count_vertices(), 0);
}

#[test]
fn required_extension_needs_loader_settings() {
    // Without the settings the gltf crate refuses the file before any
    // extension handler is asked -- the reason `draco_loader_settings` exists.
    let mut app = app(true);
    assert!(load_primitive(&mut app, "Box_Draco.glb", |_| {}).is_none());
}
