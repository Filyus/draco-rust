//! Polygon-corner-domain expansion of an [`FbxMeshInstance`].
//!
//! FBX stores geometry as control points plus layer elements that may be
//! addressed per control point, per polygon, or per polygon *corner*. Only the
//! corner domain can express a seam: two triangles meeting at one control
//! point but carrying different UVs or normals across a hard edge.
//!
//! Resolving layers onto control points therefore loses data, silently. This
//! module resolves them onto corners instead, which is what Blender's importer
//! and every renderer do.

use std::collections::HashMap;

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;

use crate::fbx_scene::{
    FbxBinormalSet, FbxColorSet, FbxLayerSet, FbxMeshInstance, FbxNormalSet, FbxTangentSet,
    FbxUvSet,
};

/// One layer element resolved onto the polygon-corner domain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FbxRenderLayer<T> {
    /// Layer name from the source file, when it had one.
    pub name: Option<String>,
    /// One value per entry in [`FbxRenderMesh::positions`].
    pub values: Vec<T>,
}

/// An [`FbxMeshInstance`] expanded onto the polygon-corner domain.
///
/// Every list indexed "per corner" has exactly [`Self::corner_count`] entries,
/// so a renderer can upload them as parallel vertex buffers directly.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FbxRenderMesh {
    /// Corner positions, duplicated wherever a control point is shared.
    pub positions: Vec<[f32; 3]>,
    /// Every `LayerElementNormal`, in source order.
    pub normals: Vec<FbxRenderLayer<[f32; 3]>>,
    /// Every `LayerElementUV`, in source order.
    pub uvs: Vec<FbxRenderLayer<[f32; 2]>>,
    /// Every `LayerElementColor`, in source order. Linear RGBA.
    pub colors: Vec<FbxRenderLayer<[f32; 4]>>,
    /// Every `LayerElementTangent`, in source order, with the handedness sign
    /// in `w` -- the layout glTF's `TANGENT` expects.
    pub tangents: Vec<FbxRenderLayer<[f32; 4]>>,
    /// Every `LayerElementBinormal`, in source order.
    pub binormals: Vec<FbxRenderLayer<[f32; 4]>>,
    /// Triangle-fan indices into the corner arrays.
    pub indices: Vec<u32>,
    /// Corner count of each source polygon, in order.
    ///
    /// Lets a consumer that preserves n-gons -- Blender's importer does --
    /// rebuild the original faces without re-reading the file.
    pub polygon_sizes: Vec<u32>,
    /// Source control point of each corner.
    ///
    /// Skin weights and blend-shape deltas are indexed by control point, so
    /// this is how they are re-indexed onto the expanded mesh.
    pub corner_to_control_point: Vec<u32>,
    /// Source polygon of each corner.
    pub corner_to_polygon: Vec<u32>,
}

impl FbxRenderMesh {
    /// Number of polygon corners, and therefore of per-corner values.
    pub fn corner_count(&self) -> usize {
        self.positions.len()
    }
}

/// Borrowed FBX geometry, as [`expand_to_render_mesh`] consumes it.
///
/// A struct rather than a positional argument list because the list grows with
/// every layer family the crate learns to read, and seven adjacent slices of
/// nearly interchangeable types are easy to transpose silently.
#[derive(Debug, Clone, Copy, Default)]
pub struct FbxGeometryLayers<'a> {
    /// FBX control-point positions.
    pub control_points: &'a [[f32; 3]],
    /// Polygon-corner indices; a negative value terminates a polygon.
    pub polygon_vertex_indices: &'a [i32],
    /// UV layer elements.
    pub uv_sets: &'a [FbxUvSet],
    /// Normal layer elements.
    pub normal_sets: &'a [FbxNormalSet],
    /// Colour layer elements.
    pub color_sets: &'a [FbxColorSet],
    /// Tangent layer elements.
    pub tangent_sets: &'a [FbxTangentSet],
    /// Binormal layer elements.
    pub binormal_sets: &'a [FbxBinormalSet],
}

impl<'a> FbxGeometryLayers<'a> {
    /// Borrows every layer family from a decoded mesh instance.
    pub fn from_instance(instance: &'a FbxMeshInstance) -> Self {
        Self {
            control_points: &instance.control_points,
            polygon_vertex_indices: &instance.polygon_vertex_indices,
            uv_sets: &instance.uv_sets,
            normal_sets: &instance.normal_sets,
            color_sets: &instance.color_sets,
            tangent_sets: &instance.tangent_sets,
            binormal_sets: &instance.binormal_sets,
        }
    }
}

/// One emitted polygon corner, carrying every domain a layer element can be
/// addressed in.
#[derive(Debug, Clone, Copy)]
struct EmittedCorner {
    /// Control point this corner references.
    control_point: u32,
    /// Position in the source `PolygonVertexIndex` stream.
    source_corner: usize,
    /// Polygon this corner belongs to.
    polygon: u32,
}

/// Resolves one layer-element value for a corner.
///
/// `mapping` selects the domain the layer is addressed in and `reference`
/// whether values are direct or indirected through `indices`.
fn resolve_layer_value<const N: usize>(
    mapping: Option<&str>,
    reference: Option<&str>,
    indices: &[i32],
    values: &[[f32; N]],
    corner: EmittedCorner,
) -> [f32; N] {
    let logical = match mapping {
        Some("ByPolygonVertex") => corner.source_corner,
        // One value per polygon -- flat shading, typically. Reading this on
        // the control-point domain returned an unrelated value.
        Some("ByPolygon") => corner.polygon as usize,
        Some("AllSame") | Some("AllSameOrPolygon") => 0,
        // `ByVertice`, `ByVertex`, `ByControlPoint` and anything unrecognized
        // address the control point.
        _ => corner.control_point as usize,
    };
    let value_index = if reference == Some("IndexToDirect") {
        match indices.get(logical).copied() {
            // A negative index is corrupt data, not a back-reference.
            Some(index) if index < 0 => return [0.0; N],
            Some(index) => index as usize,
            None => logical,
        }
    } else {
        logical
    };
    values.get(value_index).copied().unwrap_or([0.0; N])
}

/// Resolves one preserved layer element onto every polygon corner.
///
/// Component count is the only thing that differs between UV, normal, colour
/// and tangent layers, and it is a const parameter, so one function covers all
/// of them.
fn resolve_layer<const N: usize>(
    set: &FbxLayerSet<N>,
    corners: &[EmittedCorner],
) -> FbxRenderLayer<[f32; N]> {
    FbxRenderLayer {
        name: set.name.clone(),
        values: corners
            .iter()
            .map(|&corner| {
                resolve_layer_value(
                    set.mapping.as_deref(),
                    set.reference.as_deref(),
                    &set.indices,
                    &set.values,
                    corner,
                )
            })
            .collect(),
    }
}

/// Expands raw FBX geometry onto the polygon-corner domain.
///
/// Takes the parts rather than an [`FbxMeshInstance`] so the reader can call
/// it while decoding, before an instance exists.
///
/// Returns an empty mesh when there is no raw geometry, which is the case for
/// scenes synthesized in memory rather than read from a file.
pub fn expand_to_render_mesh(source: FbxGeometryLayers<'_>) -> FbxRenderMesh {
    let FbxGeometryLayers {
        control_points,
        polygon_vertex_indices,
        uv_sets,
        normal_sets,
        color_sets,
        tangent_sets,
        binormal_sets,
    } = source;
    let mut render = FbxRenderMesh::default();
    if control_points.is_empty() || polygon_vertex_indices.is_empty() {
        return render;
    }

    // Walk the polygon-corner stream once, fan-triangulating as we go and
    // recording which source corner each emitted vertex came from. Layer
    // resolution then happens per emitted corner.
    let mut emitted: Vec<EmittedCorner> = Vec::new();
    let mut polygon: Vec<EmittedCorner> = Vec::new();
    let mut polygon_index = 0u32;

    for (corner, encoded) in polygon_vertex_indices.iter().enumerate() {
        let control_point = if *encoded < 0 {
            !*encoded as u32
        } else {
            *encoded as u32
        };
        polygon.push(EmittedCorner {
            control_point,
            source_corner: corner,
            polygon: polygon_index,
        });

        // A negative index terminates the polygon.
        if *encoded < 0 {
            render.polygon_sizes.push(polygon.len() as u32);
            for offset in 1..polygon.len().saturating_sub(1) {
                for &vertex in &[polygon[0], polygon[offset], polygon[offset + 1]] {
                    emitted.push(vertex);
                    render.corner_to_polygon.push(polygon_index);
                }
            }
            polygon.clear();
            polygon_index += 1;
        }
    }

    render.positions = emitted
        .iter()
        .map(|corner| {
            control_points
                .get(corner.control_point as usize)
                .copied()
                .unwrap_or([0.0; 3])
        })
        .collect();
    render.corner_to_control_point = emitted.iter().map(|c| c.control_point).collect();
    render.indices = (0..emitted.len() as u32).collect();
    render.uvs = uv_sets.iter().map(|s| resolve_layer(s, &emitted)).collect();
    render.normals = normal_sets
        .iter()
        .map(|s| resolve_layer(s, &emitted))
        .collect();
    render.colors = color_sets
        .iter()
        .map(|s| resolve_layer(s, &emitted))
        .collect();
    render.tangents = tangent_sets
        .iter()
        .map(|s| resolve_layer(&s.layer, &emitted))
        .collect();
    render.binormals = binormal_sets
        .iter()
        .map(|s| resolve_layer(&s.layer, &emitted))
        .collect();
    render
}

/// Builds a Draco mesh from corner-domain data, welding corners that agree on
/// every attribute.
///
/// Welding keeps seams -- corners that differ in UV or normal stay separate --
/// while collapsing the interior duplicates that plain corner expansion would
/// triple. This is the same trade-off a glTF exporter makes.
///
/// Only the first UV, normal and colour set reach the Draco mesh; Draco has no
/// concept of multiple sets. The rest stay on [`FbxRenderMesh`].
pub fn build_draco_mesh(render: &FbxRenderMesh) -> Mesh {
    let normals = render.normals.first();
    let uvs = render.uvs.first();
    let colors = render.colors.first();

    // Weld on the exact bit patterns so the key is hashable and two corners
    // only merge when they are genuinely identical.
    type WeldKey = (
        [u32; 3],
        Option<[u32; 3]>,
        Option<[u32; 2]>,
        Option<[u32; 4]>,
    );
    let mut unique: HashMap<WeldKey, u32> = HashMap::new();
    let mut order: Vec<usize> = Vec::new();
    let mut remapped: Vec<u32> = Vec::with_capacity(render.corner_count());

    for corner in 0..render.corner_count() {
        let position = render.positions[corner].map(f32::to_bits);
        let normal = normals.map(|layer| layer.values[corner].map(f32::to_bits));
        let uv = uvs.map(|layer| layer.values[corner].map(f32::to_bits));
        let color = colors.map(|layer| layer.values[corner].map(f32::to_bits));
        let key: WeldKey = (position, normal, uv, color);
        let next = unique.len() as u32;
        let index = *unique.entry(key).or_insert_with(|| {
            order.push(corner);
            next
        });
        remapped.push(index);
    }

    let point_count = order.len();
    let mut mesh = Mesh::new();
    mesh.set_num_points(point_count);

    let mut position = PointAttribute::new();
    position.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        point_count,
    );
    for (index, &corner) in order.iter().enumerate() {
        let bytes: Vec<u8> = render.positions[corner]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        position.buffer_mut().write(index * 12, &bytes);
    }
    mesh.add_attribute(position);

    if let Some(layer) = normals {
        let mut normal = PointAttribute::new();
        normal.init(
            GeometryAttributeType::Normal,
            3,
            DataType::Float32,
            false,
            point_count,
        );
        for (index, &corner) in order.iter().enumerate() {
            let bytes: Vec<u8> = layer.values[corner]
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            normal.buffer_mut().write(index * 12, &bytes);
        }
        mesh.add_attribute(normal);
    }

    if let Some(layer) = uvs {
        let mut tex_coord = PointAttribute::new();
        tex_coord.init(
            GeometryAttributeType::TexCoord,
            2,
            DataType::Float32,
            false,
            point_count,
        );
        for (index, &corner) in order.iter().enumerate() {
            let bytes: Vec<u8> = layer.values[corner]
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            tex_coord.buffer_mut().write(index * 8, &bytes);
        }
        mesh.add_attribute(tex_coord);
    }

    if let Some(layer) = colors {
        let mut color = PointAttribute::new();
        color.init(
            GeometryAttributeType::Color,
            4,
            DataType::Float32,
            false,
            point_count,
        );
        for (index, &corner) in order.iter().enumerate() {
            let bytes: Vec<u8> = layer.values[corner]
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            color.buffer_mut().write(index * 16, &bytes);
        }
        mesh.add_attribute(color);
    }

    mesh.set_num_faces(remapped.len() / 3);
    for (face, triangle) in remapped.chunks_exact(3).enumerate() {
        mesh.set_face(
            FaceIndex(face as u32),
            [
                PointIndex(triangle[0]),
                PointIndex(triangle[1]),
                PointIndex(triangle[2]),
            ],
        );
    }
    mesh
}

impl FbxMeshInstance {
    /// Expands this instance onto the polygon-corner domain.
    pub fn to_render_mesh(&self) -> FbxRenderMesh {
        expand_to_render_mesh(FbxGeometryLayers::from_instance(self))
    }

    /// Corner-domain Draco mesh for this instance, welded by attribute tuple.
    ///
    /// Falls back to the stored mesh when the instance carries no raw FBX
    /// geometry to expand.
    pub fn to_draco_mesh(&self) -> Mesh {
        let render = self.to_render_mesh();
        if render.positions.is_empty() {
            return self.mesh.clone();
        }
        build_draco_mesh(&render)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fbx_scene::FbxMeshInstance;

    /// Two triangles sharing an edge, with per-corner UVs that disagree across
    /// it -- the seam case that control-point resolution destroys.
    fn seamed_quad() -> FbxMeshInstance {
        FbxMeshInstance {
            name: None,
            mesh: Mesh::new(),
            control_points: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            // One quad: 0, 1, 2, ~3
            polygon_vertex_indices: vec![0, 1, 2, !3],
            uv_sets: vec![FbxUvSet {
                name: Some("map1".to_string()),
                mapping: Some("ByPolygonVertex".to_string()),
                reference: Some("Direct".to_string()),
                values: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.5, 0.5]],
                indices: Vec::new(),
            }],
            normal_sets: Vec::new(),
            color_sets: Vec::new(),
            tangent_sets: Vec::new(),
            binormal_sets: Vec::new(),
            edges: Vec::new(),
            material_indices: Vec::new(),
            skin: None,
            morph_targets: Vec::new(),
        }
    }

    #[test]
    fn a_quad_fans_into_two_triangles_over_four_corners() {
        let render = seamed_quad().to_render_mesh();
        assert_eq!(render.polygon_sizes, vec![4]);
        assert_eq!(render.corner_count(), 6, "two fan triangles");
        assert_eq!(render.indices, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(render.corner_to_control_point, vec![0, 1, 2, 0, 2, 3]);
        assert!(render.corner_to_polygon.iter().all(|&p| p == 0));
    }

    #[test]
    fn per_corner_uvs_follow_the_corner_not_the_control_point() {
        let render = seamed_quad().to_render_mesh();
        let uv = &render.uvs[0].values;
        // Corner 3 of the fan reuses control point 0 but is source corner 0.
        assert_eq!(uv[0], [0.0, 0.0]);
        assert_eq!(uv[1], [1.0, 0.0]);
        assert_eq!(uv[2], [1.0, 1.0]);
        assert_eq!(render.uvs[0].name.as_deref(), Some("map1"));
    }

    #[test]
    fn welding_keeps_seams_but_drops_exact_duplicates() {
        let mesh = seamed_quad().to_draco_mesh();
        // Corners 0/3 and 2/4 agree on position and UV, so they weld; the
        // quad's four distinct corners survive.
        assert_eq!(mesh.num_points(), 4);
        assert_eq!(mesh.num_faces(), 2);
    }

    /// Corner 5 of polygon 2, sitting on control point 1.
    fn probe_corner() -> EmittedCorner {
        EmittedCorner {
            control_point: 1,
            source_corner: 5,
            polygon: 2,
        }
    }

    #[test]
    fn a_negative_index_to_direct_entry_yields_a_default_rather_than_panicking() {
        let value: [f32; 2] = resolve_layer_value(
            Some("ByPolygonVertex"),
            Some("IndexToDirect"),
            &[-31],
            &[[1.0, 2.0]],
            EmittedCorner {
                control_point: 0,
                source_corner: 0,
                polygon: 0,
            },
        );
        assert_eq!(value, [0.0, 0.0]);
    }

    #[test]
    fn each_mapping_addresses_its_own_domain() {
        // Distinct per index, so a wrong domain cannot coincidentally match.
        let values: Vec<[f32; 1]> = (0..8).map(|i| [i as f32]).collect();
        let resolve = |mapping| {
            resolve_layer_value(Some(mapping), Some("Direct"), &[], &values, probe_corner())
        };
        assert_eq!(resolve("ByPolygonVertex"), [5.0]);
        assert_eq!(resolve("ByVertice"), [1.0]);
        assert_eq!(resolve("AllSame"), [0.0]);
        // Regression: `ByPolygon` used to fall through to the control point,
        // silently returning the value of an unrelated polygon.
        assert_eq!(resolve("ByPolygon"), [2.0]);
    }

    #[test]
    fn an_instance_without_raw_geometry_expands_to_nothing() {
        let mut instance = seamed_quad();
        instance.control_points.clear();
        instance.polygon_vertex_indices.clear();
        assert_eq!(instance.to_render_mesh(), FbxRenderMesh::default());
    }
}
