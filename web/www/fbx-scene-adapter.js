/**
 * FBX scene export boundary for the web application.
 *
 * This module deliberately has no dependency on glTF loader code. It owns the
 * FBX representation and format-specific material/texture mapping.
 */

import { identityMat4, invertMat4 } from './mat4.js';

// glTF is right-handed Y-up; the FBX emitted for Blender is right-handed
// Z-up. With row vectors this rotates -90 degrees around X: (x, y, z) ->
// (x, z, -y). Keep this at the single glTF -> FBX boundary.
const GLTF_TO_FBX_BASIS = [
    1, 0, 0, 0,
    0, 0, -1, 0,
    0, 1, 0, 0,
    0, 0, 0, 1,
];
const FBX_TO_GLTF_BASIS = [
    1, 0, 0, 0,
    0, 0, 1, 0,
    0, -1, 0, 0,
    0, 0, 0, 1,
];

function multiplyMat4(a, b) {
    return Array.from({ length: 16 }, (_, index) => {
        const row = Math.floor(index / 4); const column = index % 4;
        return a[row * 4] * b[column] + a[row * 4 + 1] * b[column + 4]
            + a[row * 4 + 2] * b[column + 8] + a[row * 4 + 3] * b[column + 12];
    });
}

export function convertGltfVectorArrayToFbx(values) {
    const converted = Array.from(values);
    for (let offset = 0; offset + 2 < converted.length; offset += 3) {
        const y = converted[offset + 1];
        converted[offset + 1] = converted[offset + 2];
        converted[offset + 2] = -y;
    }
    return converted;
}

export function convertGltfMatrixToFbx(matrix) {
    return multiplyMat4(multiplyMat4(FBX_TO_GLTF_BASIS, matrix), GLTF_TO_FBX_BASIS);
}

/** Convert the portable glTF PBR subset to FBX Phong material properties. */
export function buildFbxMaterials(definitions) {
    return definitions.map((definition, index) => {
        const pbr = definition.pbrMetallicRoughness || {};
        const base = pbr.baseColorFactor || [1, 1, 1, 1];
        const textures = [];
        const add = (info, slot) => {
            if (typeof info?.index === 'number') textures.push({ slot, textureIndex: info.index });
        };
        add(pbr.baseColorTexture, 'diffuse');
        add(definition.normalTexture, 'normal');
        add(definition.emissiveTexture, 'emissive');
        add(pbr.metallicRoughnessTexture, 'roughness');
        return {
            name: definition.name || `material_${index}`,
            shadingModel: 'Phong',
            diffuse: base.slice(0, 3),
            diffuseFactor: 1,
            emissive: definition.emissiveFactor || [0, 0, 0],
            emissiveFactor: 1,
            reflectionFactor: pbr.metallicFactor ?? 0,
            shininess: Math.max(0, Math.min(1, pbr.roughnessFactor ?? 1)) * -100 + 100,
            opacity: base[3] ?? 1,
            textures,
        };
    });
}

/** Preserve embedded or external glTF image data for an FBX Texture/Video. */
export function buildFbxTextures(asset, document, resources, resolveUriBytes) {
    const images = document.images || [];
    return (document.textures || []).map((texture, index) => {
        const image = images[texture.source] || {};
        const content = typeof image.bufferView === 'number'
            ? Array.from(new Uint8Array(asset.bufferViewBytes(image.bufferView)))
            : (image.uri ? Array.from(resolveUriBytes(image.uri, resources) || []) : null);
        return { name: texture.name || image.name || `texture_${index}`, content, filename: image.uri || null };
    });
}

/** Convert glTF quaternion key values to FBX XYZ Euler key values. */
export function quaternionKeysToFbxEuler(values) {
    const result = [];
    for (let index = 0; index + 3 < values.length; index += 4) {
        // q' = C⁻¹ q C, where C is the glTF Y-up -> FBX Z-up basis.
        // The simplified component form is stable and avoids converting each
        // frame through a matrix before Euler decomposition.
        const [x, y, z, w] = values.slice(index, index + 4);
        const qx = x;
        const qy = z;
        const qz = -y;
        const euler = [
            Math.atan2(2 * (w * qx + qy * qz), 1 - 2 * (qx * qx + qy * qy)),
            Math.asin(Math.max(-1, Math.min(1, 2 * (w * qy - qz * qx)))),
            Math.atan2(2 * (w * qz + qx * qy), 1 - 2 * (qy * qy + qz * qz)),
        ];
        // Legacy FBX's Euler evaluator does not pick an equivalent branch.
        // Unwrap each component so consecutive keys remain continuous.
        for (let component = 0; component < 3; component += 1) {
            const previous = result.length >= 3 ? result[result.length - 3 + component] : euler[component];
            while (euler[component] - previous > Math.PI) euler[component] -= Math.PI * 2;
            while (euler[component] - previous < -Math.PI) euler[component] += Math.PI * 2;
        }
        result.push(...euler);
    }
    return result;
}

/** Split glTF CUBICSPLINE [in, value, out] key payloads. */
export function extractGltfCubicSegment(values, components, segment) {
    const result = [];
    const stride = components * 3;
    for (let offset = 0; offset + stride <= values.length; offset += stride) {
        result.push(...values.slice(offset + components * segment, offset + components * (segment + 1)));
    }
    return result;
}

export function fbxRowMajorMatrix(node, composeTrs) {
    const gltfMatrix = Array.from(Array.isArray(node.matrix) && node.matrix.length === 16
        ? node.matrix
        : composeTrs(node.translation || [0, 0, 0], node.rotation || [0, 0, 0, 1], node.scale || [1, 1, 1]));
    return convertGltfMatrixToFbx(gltfMatrix);
}

export function buildFbxWorldMatrices(nodes, roots, composeTrs) {
    const worlds = Array.from({ length: nodes.length }, () => null);
    // Scale is carried entirely by `UnitScaleFactor = 100.0` in the writer's
    // GlobalSettings (Blender reads it as the centimeters->meters factor).
    // Scaling coordinates here as well makes the imported scene 100× too
    // large, which was the original "legacy FBX" workaround that is no
    // longer needed.
    const visit = (index, parent) => {
        if (worlds[index]) return;
        const local = fbxRowMajorMatrix(nodes[index] || {}, composeTrs);
        // FBX uses row vectors. The local transform therefore precedes its
        // parent's transform in the composed world matrix.
        worlds[index] = parent ? multiplyMat4(local, parent) : local;
        for (const child of nodes[index]?.children || []) visit(child, worlds[index]);
    };
    for (const root of roots) visit(root, null);
    nodes.forEach((_, index) => { if (!worlds[index]) visit(index, null); });
    return worlds;
}

/** Turn glTF skin accessors into FBX clusters without truncating influences. */
export function buildFbxSkins(asset, definitions, worlds, warnings, readAccessorAsTyped, composeTrs) {
    return definitions.map((definition, index) => {
        const joints = (definition.joints || []).map((nodeIndex) => ({
            nodeId: nodeIndex + 1,
            bind: worlds[nodeIndex] || fbxRowMajorMatrix({}, composeTrs),
        }));
        if (typeof definition.inverseBindMatrices === 'number') {
            const accessor = readAccessorAsTyped(asset, definition.inverseBindMatrices);
            if (accessor.componentType !== 5126 || accessor.components !== 16) {
                warnings.push(`Skin ${index} has unsupported inverse bind matrices`);
            } else {
                for (let jointIndex = 0; jointIndex < joints.length && jointIndex < accessor.count; jointIndex += 1) {
                    const inverse = Array.from(accessor.data.subarray(jointIndex * 16, jointIndex * 16 + 16));
                    const bind = invertMat4(convertGltfMatrixToFbx(inverse)) || joints[jointIndex].bind;
                    joints[jointIndex].bind = bind;
                }
            }
        }
        return {
            joints,
            // glTF has joints but no Armature object. Blender's legacy FBX
            // importer interprets TransformAssociateModel as the Armature
            // object's world matrix, not the root joint's matrix. The latter
            // makes the legacy importer apply the root transform twice.
            // Identity is correct in both legacy and standard paths now that
            // scale is carried by UnitScaleFactor instead of by the matrix.
            armatureBindTransform: null,
        };
    });
}

/** Attach all JOINTS_0/1 and WEIGHTS_0/1 influences to FBX skin clusters. */
export function buildFbxMeshSkin(mesh, skin, meshNodeId, meshBindTransform, composeTrs) {
    if (!skin || !mesh.joints0 || !mesh.weights0) return null;
    const meshBind = meshBindTransform || fbxRowMajorMatrix({}, composeTrs);
    const clusters = skin.joints.map((joint) => ({
        jointNodeId: joint.nodeId,
        controlPointIndices: [],
        weights: [],
        // Blender's importer reconstructs the mesh bind matrix as
        // `TransformLink @ Transform`. Solving for `Transform` gives
        // `TransformLink⁻¹ @ MeshWorldBind`; `multiplyMat4(a, b)` is row-major
        // `a · b`, whose column-major equivalent is `b @ a`, so
        // `meshBind · inverse(joint.bind)` yields the required
        // `inverse(TransformLink) @ MeshWorldBind`.
        meshBindTransform: multiplyMat4(meshBind, invertMat4(joint.bind) || identityMat4()),
        jointBindTransform: joint.bind,
        armatureBindTransform: skin.armatureBindTransform || null,
    }));
    const vertexCount = mesh.positions.length / 3;
    for (const [joints, weights] of [[mesh.joints0, mesh.weights0], [mesh.joints1, mesh.weights1]]) {
        if (!joints || !weights) continue;
        for (let vertex = 0; vertex < vertexCount; vertex += 1) {
            for (let component = 0; component < 4; component += 1) {
                const weight = Number(weights[vertex * 4 + component]) || 0;
                const joint = Number(joints[vertex * 4 + component]);
                if (weight > 0 && Number.isInteger(joint) && clusters[joint]) {
                    clusters[joint].controlPointIndices.push(vertex);
                    clusters[joint].weights.push(weight);
                }
            }
        }
    }
    return {
        clusters: clusters.filter((cluster) => cluster.weights.length > 0),
        // Native Blender's importer needs the mesh Model as well as every
        // joint in BindPose to construct an armature modifier.
        bindPose: [
            { nodeId: meshNodeId, matrix: meshBind },
            ...skin.joints.map((joint) => ({ nodeId: joint.nodeId, matrix: joint.bind })),
        ],
    };
}

/** Decode glTF morph deltas into the FBX shape contract. */
export function buildFbxMorphTargets(asset, targetDefinitions, weights, readAccessorAsTyped) {
    return targetDefinitions.flatMap((target, index) => {
        if (typeof target.POSITION !== 'number') return [];
        const accessor = readAccessorAsTyped(asset, target.POSITION);
        if (accessor.componentType !== 5126 || accessor.components !== 3) return [];
        let normalDeltas = null;
        if (typeof target.NORMAL === 'number') {
            const normal = readAccessorAsTyped(asset, target.NORMAL);
            if (normal.componentType === 5126 && normal.components === 3 && normal.count === accessor.count) normalDeltas = Array.from(normal.data);
        }
        return [{
            name: `target_${index}`,
            controlPointIndices: Array.from({ length: accessor.count }, (_, point) => point),
            positionDeltas: convertGltfVectorArrayToFbx(accessor.data),
            normalDeltas: normalDeltas && convertGltfVectorArrayToFbx(normalDeltas),
            defaultWeight: Number(weights[index]) || 0,
            fullWeight: 100,
        }];
    });
}
