//! The glTF loader hook.

use bevy::asset::{LoadContext, RenderAssetUsages};
use bevy::gltf::extensions::{ErasedGltfExtensionHandler, GltfExtensionHandler};
use bevy::gltf::{GltfAssetLabel, GltfLoaderSettings};
use bevy::log::error;
use bevy::mesh::{Mesh, MeshVertexAttribute};
use bevy::platform::collections::HashMap;
use bevy::tasks::ConditionalSendFuture;
use draco_gltf::{DecodeLimits, KHR_DRACO_MESH_COMPRESSION};

use crate::decode::decode_primitive;
use crate::mesh::{build_mesh, empty_mesh, MeshContext};

/// Decodes `KHR_draco_mesh_compression` primitives for one glTF load.
///
/// Bevy clones the registered handler for every load, so the per-load fields
/// start from the plugin's defaults each time.
#[derive(Clone)]
pub(crate) struct DracoHandler {
    /// Decode ceilings, from the plugin.
    pub limits: DecodeLimits,
    /// `GltfPlugin::convert_coordinates.rotate_meshes`, which a load's
    /// settings override only when they set `convert_coordinates` at all.
    pub default_rotate_meshes: bool,
    load_meshes: RenderAssetUsages,
    rotate_meshes: bool,
}

impl DracoHandler {
    pub(crate) fn new(limits: DecodeLimits, default_rotate_meshes: bool) -> Self {
        Self {
            limits,
            default_rotate_meshes,
            load_meshes: RenderAssetUsages::default(),
            rotate_meshes: default_rotate_meshes,
        }
    }
}

impl GltfExtensionHandler for DracoHandler {
    fn dyn_clone(&self) -> Box<dyn ErasedGltfExtensionHandler> {
        Box::new(self.clone())
    }

    fn on_root(
        &mut self,
        _load_context: &mut LoadContext<'_>,
        _gltf: &gltf::Gltf,
        settings: &GltfLoaderSettings,
    ) {
        self.load_meshes = settings.load_meshes;
        self.rotate_meshes = settings
            .convert_coordinates
            .map_or(self.default_rotate_meshes, |convert| convert.rotate_meshes);
    }

    fn on_gltf_primitive(
        &mut self,
        _load_context: &mut LoadContext<'_>,
        gltf_document: &gltf::Gltf,
        gltf_mesh: &gltf::Mesh<'_>,
        gltf_primitive: &gltf::Primitive<'_>,
        buffer_data: &[Vec<u8>],
        custom_vertex_attributes: &HashMap<Box<str>, MeshVertexAttribute>,
        gltf_mesh_on_skinned_nodes: bool,
        gltf_mesh_on_non_skinned_nodes: bool,
        user_mesh: &mut Option<Mesh>,
    ) -> impl ConditionalSendFuture<Output = ()> {
        // The decode is synchronous on every target -- no JavaScript decoder
        // to wait for -- so it runs here and the future is already complete.
        if let Some(extension) = gltf_primitive
            .extensions()
            .and_then(|extensions| extensions.get(KHR_DRACO_MESH_COMPRESSION))
        {
            let label = GltfAssetLabel::Primitive {
                mesh: gltf_mesh.index(),
                primitive: gltf_primitive.index(),
            }
            .to_string();
            let mesh = match decode_primitive(
                gltf_document,
                gltf_primitive,
                extension,
                buffer_data,
                self.limits,
            ) {
                Ok(decoded) => build_mesh(
                    &decoded,
                    gltf_primitive,
                    gltf_mesh,
                    buffer_data,
                    &MeshContext {
                        load_meshes: self.load_meshes,
                        rotate_meshes: self.rotate_meshes,
                        custom_vertex_attributes,
                        on_skinned_nodes: gltf_mesh_on_skinned_nodes,
                        on_non_skinned_nodes: gltf_mesh_on_non_skinned_nodes,
                        label: &label,
                    },
                ),
                Err(error) => {
                    error!("{label}: cannot decode {KHR_DRACO_MESH_COMPRESSION}: {error}");
                    empty_mesh(self.load_meshes)
                }
            };
            *user_mesh = Some(mesh);
        }
        core::future::ready(())
    }
}
