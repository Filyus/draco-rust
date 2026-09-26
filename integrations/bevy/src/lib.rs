//! `KHR_draco_mesh_compression` for Bevy's glTF loader, decoded by
//! [`draco_gltf`].
//!
//! Add [`DracoGltfPlugin`] next to Bevy's `GltfPlugin` (in either order) and
//! load Draco-compressed files with [`draco_loader_settings`]:
//!
//! ```no_run
//! use bevy::prelude::*;
//! use draco_gltf_bevy::{draco_loader_settings, DracoGltfPlugin};
//!
//! fn main() {
//!     App::new()
//!         .add_plugins((DefaultPlugins, DracoGltfPlugin::default()))
//!         .add_systems(Startup, setup)
//!         .run();
//! }
//!
//! fn setup(mut commands: Commands, assets: Res<AssetServer>) {
//!     commands.spawn(WorldAssetRoot(assets.load_with_settings(
//!         GltfAssetLabel::Scene(0).from_asset("truck.glb"),
//!         draco_loader_settings,
//!     )));
//! }
//! ```
//!
//! The decoder is pure Rust with no `unsafe`, and it runs the same way natively
//! and in the browser: there is no JavaScript decoder to load and nothing to
//! wait for.

mod decode;
mod handler;
mod mesh;

use bevy::app::{App, Plugin};
use bevy::gltf::extensions::GltfExtensionHandlers;
use bevy::gltf::{GltfLoaderSettings, GltfPlugin};
use bevy::log::warn;

pub use draco_gltf::DecodeLimits;

use crate::handler::DracoHandler;

/// Adds `KHR_draco_mesh_compression` decoding to Bevy's glTF loader.
#[derive(Clone, Debug, Default)]
pub struct DracoGltfPlugin {
    /// Ceilings on what one primitive's decode may allocate and reconstruct.
    ///
    /// A Draco stream says how many points and faces it will produce before it
    /// produces them; these bound what a hostile file can make the loader
    /// allocate.
    pub limits: DecodeLimits,
}

impl Plugin for DracoGltfPlugin {
    fn build(&self, app: &mut App) {
        // Created here too so that the order of this plugin and `GltfPlugin`
        // does not matter: both only initialize it, and the loader takes the
        // shared list in `GltfPlugin::finish`.
        app.init_resource::<GltfExtensionHandlers>();
    }

    fn finish(&self, app: &mut App) {
        // The handler is told a load's coordinate settings, but not the
        // plugin-wide default a load falls back to, so it is read here, once
        // every plugin has been added.
        let default_rotate_meshes = app
            .get_added_plugins::<GltfPlugin>()
            .first()
            .is_some_and(|gltf| gltf.convert_coordinates.rotate_meshes);
        let handler = DracoHandler::new(self.limits, default_rotate_meshes);

        let handlers = app.world().resource::<GltfExtensionHandlers>();
        // Nothing else holds the lock while the app is being built.
        match handlers.0.try_write() {
            Some(mut handlers) => handlers.push(Box::new(handler)),
            None => warn!("DracoGltfPlugin: glTF extension handlers are locked; Draco decoding is not registered"),
        }
    }
}

/// Loader settings for a glTF file that requires `KHR_draco_mesh_compression`.
///
/// Such a file lists the extension in `extensionsRequired`, and the gltf crate
/// Bevy parses with rejects every required extension it does not implement
/// itself, before any extension handler runs. This turns that validation off;
/// the Draco payload is still bounds-checked and validated while it is
/// decoded.
///
/// Pass it to `AssetServer::load_with_settings`.
pub fn draco_loader_settings(settings: &mut GltfLoaderSettings) {
    settings.validate = false;
}
