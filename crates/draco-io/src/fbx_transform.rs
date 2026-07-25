//! The FBX transform stack: `Properties70` in, one local matrix out.
//!
//! FBX does not store a node's local transform directly. It stores up to nine
//! separate components -- translation, rotation with a pre- and post-rotation,
//! scaling, and rotation and scaling pivots with their offsets -- that an
//! importer multiplies in a fixed order. Reproducing that order is the whole
//! of this module.
//!
//! The composed matrix is what the rest of the crate uses, but the components
//! are also kept verbatim in [`FbxTransformStack`] so an FBX-to-FBX rewrite
//! can put them back rather than emit a matrix the original never had.
//!
//! Kept apart from the scene reader because none of it reads the node tree
//! beyond one `Model`'s properties: it is pure arithmetic over values already
//! decoded, and the reader is easier to follow without 350 lines of matrix
//! math in the middle of it.

use std::collections::HashMap;

use crate::fbx_container::{FbxNode, FbxProperty};
use crate::fbx_scene::{push_warning, FbxTransform, FbxWarning, FbxWarningCode};
use crate::fbx_templates::{ObjectProperties, PropertyTemplates};

// Helper to parse transform from Model node's Properties70
pub(crate) fn parse_transform(
    properties: ObjectProperties<'_>,
) -> Option<(FbxTransform, crate::fbx_scene::FbxTransformStack, bool)> {
    let mut translation = None;
    let mut rotation = None;
    let mut scaling = None;
    let mut pre_rotation = None;
    let mut post_rotation = None;
    let mut rotation_offset = None;
    let mut rotation_pivot = None;
    let mut scaling_offset = None;
    let mut scaling_pivot = None;
    let mut rotation_order = None;
    let mut rotation_active = None;
    let mut inherit_type = None;

    fn property_vec3(property: &FbxNode) -> Option<[f32; 3]> {
        for value in &property.properties {
            if let crate::fbx_reader::FbxProperty::F64Array(values) = value {
                if values.len() >= 3 {
                    return Some([values[0] as f32, values[1] as f32, values[2] as f32]);
                }
            }
        }

        let values: Vec<f32> = property
            .properties
            .iter()
            .filter_map(|value| match value {
                crate::fbx_reader::FbxProperty::F64(value) => Some(*value as f32),
                crate::fbx_reader::FbxProperty::F32(value) => Some(*value),
                _ => None,
            })
            .take(3)
            .collect();
        (values.len() == 3).then(|| [values[0], values[1], values[2]])
    }

    fn property_i32(property: &FbxNode) -> Option<i32> {
        property.properties.iter().find_map(|value| match value {
            crate::fbx_reader::FbxProperty::I32(value) => Some(*value),
            crate::fbx_reader::FbxProperty::I16(value) => Some(*value as i32),
            crate::fbx_reader::FbxProperty::I64(value) => i32::try_from(*value).ok(),
            _ => None,
        })
    }

    fn property_bool(property: &FbxNode) -> Option<bool> {
        property.properties.iter().find_map(|value| match value {
            crate::fbx_reader::FbxProperty::Bool(value) => Some(*value),
            crate::fbx_reader::FbxProperty::I32(value) => Some(*value != 0),
            crate::fbx_reader::FbxProperty::I16(value) => Some(*value != 0),
            crate::fbx_reader::FbxProperty::I64(value) => Some(*value != 0),
            _ => None,
        })
    }

    // The class defaults first, then the object's own values over the top.
    // Last write wins, so the object wins -- which is the whole point: 928
    // models in this crate's corpus override the template's Lcl Translation,
    // and taking the template instead would move every one to the origin.
    let blocks = properties.template().into_iter().chain(
        properties
            .node()
            .children
            .iter()
            .filter(|child| child.name == "Properties70"),
    );
    for block in blocks {
        for prop in &block.children {
            let Some(FbxProperty::String(name)) = prop.properties.first() else {
                continue;
            };
            match name.as_str() {
                "Lcl Translation" => translation = property_vec3(prop),
                "Lcl Rotation" => rotation = property_vec3(prop),
                "Lcl Scaling" => scaling = property_vec3(prop),
                "PreRotation" => pre_rotation = property_vec3(prop),
                "PostRotation" => post_rotation = property_vec3(prop),
                "RotationOffset" => rotation_offset = property_vec3(prop),
                "RotationPivot" => rotation_pivot = property_vec3(prop),
                "ScalingOffset" => scaling_offset = property_vec3(prop),
                "ScalingPivot" => scaling_pivot = property_vec3(prop),
                "RotationOrder" => rotation_order = property_i32(prop),
                "RotationActive" => rotation_active = property_bool(prop),
                "InheritType" => inherit_type = property_i32(prop),
                _ => {}
            }
        }
    }

    if translation.is_none() && rotation.is_none() && scaling.is_none() {
        return None;
    }

    // FBX local transform stack (without the parent-dependent
    // InheritType rule):
    // T * Roff * Rp * PreR * R * PostR^-1 * Rp^-1 * Soff * Sp * S * Sp^-1.
    // The packed scene layout is also the WebGL column-major layout.
    let t = translation.unwrap_or([0.0, 0.0, 0.0]);
    let r_deg = rotation.unwrap_or([0.0, 0.0, 0.0]);
    let s = scaling.unwrap_or([1.0, 1.0, 1.0]);

    fn identity() -> [[f32; 4]; 4] {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }
    // FbxTransform is packed column-major: its outer index is the
    // column and its inner index is the row. Evaluate the local stack
    // in that layout so the result can go straight to WebGL.
    fn multiply(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
        let mut result = [[0.0; 4]; 4];
        for column in 0..4 {
            for row in 0..4 {
                result[column][row] = (0..4).map(|index| a[index][row] * b[column][index]).sum();
            }
        }
        result
    }
    fn translation_matrix(values: [f32; 3]) -> [[f32; 4]; 4] {
        let mut matrix = identity();
        matrix[3][0] = values[0];
        matrix[3][1] = values[1];
        matrix[3][2] = values[2];
        matrix
    }
    fn scale(values: [f32; 3]) -> [[f32; 4]; 4] {
        [
            [values[0], 0.0, 0.0, 0.0],
            [0.0, values[1], 0.0, 0.0],
            [0.0, 0.0, values[2], 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }
    fn rotation_matrix(values: [f32; 3]) -> [[f32; 4]; 4] {
        let (sin_x, cos_x) = values[0].to_radians().sin_cos();
        let (sin_y, cos_y) = values[1].to_radians().sin_cos();
        let (sin_z, cos_z) = values[2].to_radians().sin_cos();
        // Rz * Ry * Rx, packed by column for FBX/WebGL.
        [
            [cos_z * cos_y, sin_z * cos_y, -sin_y, 0.0],
            [
                cos_z * sin_y * sin_x - sin_z * cos_x,
                sin_z * sin_y * sin_x + cos_z * cos_x,
                cos_y * sin_x,
                0.0,
            ],
            [
                cos_z * sin_y * cos_x + sin_z * sin_x,
                sin_z * sin_y * cos_x - cos_z * sin_x,
                cos_y * cos_x,
                0.0,
            ],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }
    let inverse_translation =
        |values: [f32; 3]| translation_matrix([-values[0], -values[1], -values[2]]);
    let inverse_rotation = |values: [f32; 3]| {
        let rotation = rotation_matrix(values);
        let mut inverse = [[0.0; 4]; 4];
        for column in 0..4 {
            for row in 0..4 {
                inverse[column][row] = rotation[row][column];
            }
        }
        inverse
    };
    // Blender's FBX bind matrices encode the pre-rotation with the
    // opposite handedness from the local Euler property.  Applying
    // its inverse here keeps Mixamo-style armatures aligned while
    // leaving ordinary TRS-only files unchanged.
    let inverse_pre_rotation = pre_rotation
        .map(|values| [-values[0], -values[1], -values[2]])
        .unwrap_or([0.0; 3]);
    // The scene matrix uses the same packed layout as the writer:
    // translation occupies the final packed column.  Do not multiply
    // the local translation through the rotation stack here; doing so
    // rotates a node's origin (and breaks ordinary TRS-only FBX
    // written by us).  The pivot terms below only shape the linear
    // part and their translation compensation.
    let mut mat = identity();
    for term in [
        translation_matrix(rotation_offset.unwrap_or([0.0; 3])),
        translation_matrix(rotation_pivot.unwrap_or([0.0; 3])),
        rotation_matrix(inverse_pre_rotation),
        rotation_matrix(r_deg),
        inverse_rotation(post_rotation.unwrap_or([0.0; 3])),
        inverse_translation(rotation_pivot.unwrap_or([0.0; 3])),
        translation_matrix(scaling_offset.unwrap_or([0.0; 3])),
        translation_matrix(scaling_pivot.unwrap_or([0.0; 3])),
        scale(s),
        inverse_translation(scaling_pivot.unwrap_or([0.0; 3])),
    ] {
        mat = multiply(mat, term);
    }
    mat[3][0] += t[0];
    mat[3][1] += t[1];
    mat[3][2] += t[2];

    // A bind pose contains the exporter-evaluated local orientation
    // for nodes that use FBX's pre/post rotation or pivot terms. The
    // semantic preview keeps that baked basis for animation, while
    // ordinary Model TRS nodes keep their authored local values.
    let non_zero = |values: Option<[f32; 3]>| {
        values.is_some_and(|values| values.iter().any(|value| value.abs() > f32::EPSILON))
    };
    let transform_stack = crate::fbx_scene::FbxTransformStack {
        translation,
        rotation,
        scaling,
        rotation_order,
        rotation_active,
        pre_rotation,
        post_rotation,
        rotation_offset,
        rotation_pivot,
        scaling_offset,
        scaling_pivot,
        inherit_type,
    };
    // RotationOrder and InheritType are source-provenance metadata;
    // their ordinary/default values do not mean that the static
    // Model TRS has been baked into the skin BindPose. Keep the
    // runtime flag limited to actual pivot/offset/pre/post terms so
    // plain TRS clips (including Samba Dancing) retain authored
    // animation composition while the metadata is still re-emitted.
    // Non-default rotation-order/inheritance evaluation remains an
    // explicit compatibility caveat at the animation boundary.
    let has_complex_transform_stack = non_zero(pre_rotation)
        || non_zero(post_rotation)
        || non_zero(rotation_offset)
        || non_zero(rotation_pivot)
        || non_zero(scaling_offset)
        || non_zero(scaling_pivot);

    Some((
        FbxTransform { matrix: mat },
        transform_stack,
        has_complex_transform_stack,
    ))
}

pub(crate) fn identity_transform() -> FbxTransform {
    FbxTransform {
        matrix: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    }
}

pub(crate) fn transform_array(node: &FbxNode, child_name: &str) -> Option<FbxTransform> {
    let values = node
        .children
        .iter()
        .find(|child| child.name == child_name)?
        .properties
        .first()?;
    let values: Vec<f32> = match values {
        FbxProperty::F64Array(values) => values.iter().copied().map(|value| value as f32).collect(),
        FbxProperty::F32Array(values) => values.clone(),
        _ => return None,
    };
    if values.len() != 16 {
        return None;
    }
    // Preserve the 16-value FBX matrix layout. In particular, cluster
    // Transform/TransformLink translations are already at elements 12..14;
    // transposing here moves them into the projective row and explodes a
    // skinned mesh.
    let mut matrix = [[0.0; 4]; 4];
    for (index, value) in values.into_iter().enumerate() {
        matrix[index / 4][index % 4] = value;
    }
    Some(FbxTransform { matrix })
}

/// Reports Models whose FBX inheritance rule the imported transform cannot
/// express.
///
/// Only the first such Model is reported: the notice describes a property of
/// the file, and repeating it per Model would bury everything else. Models are
/// visited in authored order so the scan does not depend on hash iteration.
pub(crate) fn collect_transform_warnings<'a>(
    model_map: &HashMap<i64, &'a FbxNode>,
    model_order: &[i64],
    templates: &PropertyTemplates<'a>,
    warnings: &mut Vec<FbxWarning>,
) {
    for model in model_order.iter().filter_map(|id| model_map.get(id)) {
        let properties = ObjectProperties::new(model, templates);
        // Resolved the same way the transform itself is, or the notice would
        // describe a document different from the one that was imported.
        let Some(entry) = properties.get("InheritType") else {
            continue;
        };
        let inherit_type = entry.properties.iter().find_map(|value| match value {
            FbxProperty::I32(value) => Some(*value),
            FbxProperty::I64(value) => i32::try_from(*value).ok(),
            _ => None,
        });
        let local_scale = properties.get("Lcl Scaling").and_then(|property| {
            let values: Vec<f32> = property
                .properties
                .iter()
                .filter_map(|value| match value {
                    FbxProperty::F64(value) => Some(*value as f32),
                    FbxProperty::F32(value) => Some(*value),
                    _ => None,
                })
                .take(3)
                .collect();
            // `then_some` evaluates its argument eagerly, so indexing here
            // panicked whenever the property carried fewer than three numbers.
            (values.len() == 3).then(|| [values[0], values[1], values[2]])
        });
        // A non-uniform scale is what makes the unsupported inherit modes
        // observable; uniform scale behaves the same either way.
        let uniform_scale = local_scale
            .map(|scale| (scale[0] - scale[1]).abs() <= 1e-5 && (scale[1] - scale[2]).abs() <= 1e-5)
            .unwrap_or(true);
        if matches!(inherit_type, Some(0..=2)) && uniform_scale {
            continue;
        }
        push_warning(
            warnings,
            FbxWarningCode::UnsupportedTransformInherit,
            "FBX model uses unsupported InheritType; local TRS was imported \
             without that FBX transform rule"
                .to_string(),
            None,
        );
    }
}
