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

use crate::fbx_scene::{FbxColorSet, FbxMeshInstance, FbxNormalSet, FbxUvSet};

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

/// Resolves one layer-element value for a corner.
///
/// `mapping` selects the domain the layer is addressed in and `reference`
/// whether values are direct or indirected through `indices`.
fn resolve_layer_value<const N: usize>(
    mapping: Option<&str>,
    reference: Option<&str>,
    indices: &[i32],
    values: &[[f32; N]],
    control_point: usize,
    corner: usize,
) -> [f32; N] {
    let logical = match mapping {
        Some("ByPolygonVertex") => corner,
        Some("AllSame") | Some("AllSameOrPolygon") => 0,
        // `ByVertice`, `ByVertex`, `ByControlPoint` and anything unrecognized
        // address the control point.
        _ => control_point,
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

fn uv_layer(set: &FbxUvSet, corners: &[(u32, usize)]) -> FbxRenderLayer<[f32; 2]> {
    FbxRenderLayer {
        name: set.name.clone(),
        values: corners
            .iter()
            .map(|&(control_point, corner)| {
                resolve_layer_value(
                    set.mapping.as_deref(),
                    set.reference.as_deref(),
                    &set.indices,
                    &set.values,
                    control_point as usize,
                    corner,
                )
            })
            .collect(),
    }
}

fn color_layer(set: &FbxColorSet, corners: &[(u32, usize)]) -> FbxRenderLayer<[f32; 4]> {
    FbxRenderLayer {
        name: set.name.clone(),
        values: corners
            .iter()
            .map(|&(control_point, corner)| {
                resolve_layer_value(
                    set.mapping.as_deref(),
                    set.reference.as_deref(),
                    &set.indices,
                    &set.values,
                    control_point as usize,
                    corner,
                )
            })
            .collect(),
    }
}

fn normal_layer(set: &FbxNormalSet, corners: &[(u32, usize)]) -> FbxRenderLayer<[f32; 3]> {
    FbxRenderLayer {
        name: set.name.clone(),
        values: corners
            .iter()
            .map(|&(control_point, corner)| {
                resolve_layer_value(
                    set.mapping.as_deref(),
                    set.reference.as_deref(),
                    &set.indices,
                    &set.values,
                    control_point as usize,
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
pub fn expand_to_render_mesh(
    control_points: &[[f32; 3]],
    polygon_vertex_indices: &[i32],
    uv_sets: &[FbxUvSet],
    normal_sets: &[FbxNormalSet],
    color_sets: &[FbxColorSet],
) -> FbxRenderMesh {
    let mut render = FbxRenderMesh::default();
    if control_points.is_empty() || polygon_vertex_indices.is_empty() {
        return render;
    }

    // Walk the polygon-corner stream once, fan-triangulating as we go and
    // recording which source corner each emitted vertex came from. Layer
    // resolution then happens per emitted corner.
    let mut emitted: Vec<(u32, usize)> = Vec::new();
    let mut polygon: Vec<(u32, usize)> = Vec::new();
    let mut polygon_index = 0u32;

    for (corner, encoded) in polygon_vertex_indices.iter().enumerate() {
        let control_point = if *encoded < 0 {
            !*encoded as u32
        } else {
            *encoded as u32
        };
        polygon.push((control_point, corner));

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
        .map(|&(control_point, _)| {
            control_points
                .get(control_point as usize)
                .copied()
                .unwrap_or([0.0; 3])
        })
        .collect();
    render.corner_to_control_point = emitted.iter().map(|&(point, _)| point).collect();
    render.indices = (0..emitted.len() as u32).collect();
    render.uvs = uv_sets.iter().map(|set| uv_layer(set, &emitted)).collect();
    render.normals = normal_sets
        .iter()
        .map(|set| normal_layer(set, &emitted))
        .collect();
    render.colors = color_sets
        .iter()
        .map(|set| color_layer(set, &emitted))
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
        expand_to_render_mesh(
            &self.control_points,
            &self.polygon_vertex_indices,
            &self.uv_sets,
            &self.normal_sets,
            &self.color_sets,
        )
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

    #[test]
    fn a_negative_index_to_direct_entry_yields_a_default_rather_than_panicking() {
        let value: [f32; 2] = resolve_layer_value(
            Some("ByPolygonVertex"),
            Some("IndexToDirect"),
            &[-31],
            &[[1.0, 2.0]],
            0,
            0,
        );
        assert_eq!(value, [0.0, 0.0]);
    }

    #[test]
    fn an_instance_without_raw_geometry_expands_to_nothing() {
        let mut instance = seamed_quad();
        instance.control_points.clear();
        instance.polygon_vertex_indices.clear();
        assert_eq!(instance.to_render_mesh(), FbxRenderMesh::default());
    }
}
