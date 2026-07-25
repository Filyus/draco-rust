/**
 * Semantic FBX -> SceneDocument import boundary.
 *
 * The existing FBX bind-pose, root-motion, and cubic compatibility policies
 * stay here (via fbx-animation-adapter.js). The output is source-neutral: it
 * contains matrices/bytes/TRS clips and no FBX parser or browser objects.
 */

import { identityMat4, invertMat4, multiplyMat4 } from './mat4.js';
import { adaptFbxAnimation } from './fbx-animation-adapter.js';
import { adaptFbxMaterial } from './fbx-material-adapter.js';
import { assertValidSceneDocument, createSceneDocument } from './scene-document.js';
import { createFbxSceneProvenance } from './fbx-scene-provenance.js';

// The semantic decoder currently exposes the common FBX centimeter-space
// values. SceneDocument is canonical glTF-meter space; normalize only here so
// the active direct FBX viewer keeps its established source-space behavior.
const FBX_CENTIMETERS_TO_METERS = 0.01;

/** Adapt a semantic `fbx-wasm` parse result into a portable SceneDocument. */
export function buildSceneDocumentFromFbx(parsed, resources = Object.create(null)) {
    const roots = parsed?.scene?.rootNodes;
    if (!Array.isArray(roots) || roots.length === 0) {
        throw new Error('FBX SceneDocument import requires semantic binary FBX scene data');
    }
    const document = createSceneDocument({ warnings: [
        ...(parsed.warnings || []),
        'FBX centimeter-space geometry and transforms were normalized to meters for SceneDocument',
        'FBX source unit/axis settings remain in the optional FBX provenance sidecar; SceneDocument uses canonical glTF meter/Y-up space',
    ] });
    const { materialMap } = collectFbxMaterials(parsed, resources, document);
    const state = buildFbxNodeState(roots, document);
    attachMeshesAndSkins(roots, state, materialMap, document);
    appendFbxAnimations(parsed?.scene?.animations || parsed?.animations || [], state, document);
    assertValidSceneDocument(document);
    return document;
}

/**
 * Build the normal portable document plus an optional FBX-only sidecar.
 *
 * The sidecar is intentionally returned separately: viewer and GLB paths
 * receive only SceneDocument, while a future FBX export boundary can retain
 * raw unit/axis/curve provenance without contaminating shared scene data.
 */
export function buildSceneDocumentWithFbxProvenance(parsed, resources = Object.create(null)) {
    return {
        document: buildSceneDocumentFromFbx(parsed, resources),
        provenance: createFbxSceneProvenance(parsed),
    };
}

function collectFbxMaterials(parsed, resources, document) {
    const sourceTextures = parsed?.scene?.textures || parsed?.textures || [];
    const textureMap = sourceTextures.map((texture, sourceIndex) => {
        const bytes = texture.content?.length ? new Uint8Array(texture.content) : resolveResource(texture.filename, resources);
        if (!bytes) {
            if (texture.filename) document.warnings.push(`FBX texture not selected: ${texture.filename}`);
            return -1;
        }
        const resource = document.resources.length;
        document.resources.push({
            name: texture.name || basename(texture.filename || `texture_${sourceIndex}`),
            mimeType: mimeFromUri(texture.filename) || sniffMime(bytes) || 'application/octet-stream',
            bytes: new Uint8Array(bytes),
        });
        const index = document.textures.length;
        document.textures.push({ name: texture.name || `texture_${sourceIndex}`, resource, sampler: defaultSampler() });
        return index;
    });
    const sourceMaterials = parsed?.scene?.materials || parsed?.materials || [];
    const materialMap = sourceMaterials.map((source, materialIndex) => {
        const converted = adaptFbxMaterial(source, materialIndex);
        const material = {
            name: converted.name,
            baseColorFactor: converted.baseColorFactor,
            metallicFactor: converted.metallic,
            roughnessFactor: converted.roughness,
            emissiveFactor: converted.emissiveFactor,
            doubleSided: converted.doubleSided,
            alphaMode: converted.alphaMode,
            alphaCutoff: converted.alphaCutoff,
            unlit: converted.unlit,
        };
        for (const binding of source.textures || []) {
            const texture = textureMap[binding.textureIndex];
            if (!Number.isInteger(texture) || texture < 0) continue;
            if (binding.slot === 'diffuse') material.baseColorTexture = { texture, texCoord: 0 };
            else if (binding.slot === 'normal') material.normalTexture = { texture, texCoord: 0 };
            else if (binding.slot === 'emissive') material.emissiveTexture = { texture, texCoord: 0 };
            else if (binding.slot === 'roughness' || binding.slot === 'metallic') material.metallicRoughnessTexture = { texture, texCoord: 0 };
            else document.warnings.push(`FBX material ${source.name || materialIndex} ${binding.slot} texture is outside the portable PBR subset`);
        }
        const index = document.materials.length;
        document.materials.push(material);
        return index;
    });
    return { materialMap };
}

function buildFbxNodeState(roots, document) {
    const bindPoseByNodeId = new Map();
    const collectBindPoses = (source) => {
        for (const mesh of source.meshes || []) {
            for (const entry of mesh.skin?.bindPose || []) {
                if (Number.isInteger(entry?.nodeId) && validMatrix(entry.matrix) && !bindPoseByNodeId.has(entry.nodeId)) bindPoseByNodeId.set(entry.nodeId, entry.matrix);
            }
        }
        for (const child of source.children || []) collectBindPoses(child);
    };
    roots.forEach(collectBindPoses);

    const nodes = [];
    const stateById = new Map();
    const stateByName = new Map();
    const rootsOut = [];
    const append = (source, parentBind = null) => {
        const bind = Number.isInteger(source.id) ? bindPoseByNodeId.get(source.id) : null;
        const sourceMatrix = validMatrix(source.matrix) ? source.matrix : null;
        const rawLocalMatrix = bind
            ? Float32Array.from(parentBind ? (multiplyMat4(invertMat4(parentBind), bind) || bind) : bind)
            : sourceMatrix ? Float32Array.from(sourceMatrix) : Float32Array.from(identityMat4());
        const localMatrix = scaleMatrixTranslation(rawLocalMatrix);
        const bindTrs = decomposeMatrix(rawLocalMatrix);
        const animationTrs = sourceMatrix && !source.hasComplexTransformStack
            ? decomposeMatrix(sourceMatrix)
            : cloneTrs(bindTrs);
        const index = nodes.length;
        const sceneNode = {
            name: source.name || `node_${index}`,
            matrix: Array.from(localMatrix),
            children: [],
        };
        nodes.push(sceneNode);
        const state = {
            id: Number.isInteger(source.id) ? source.id : null,
            name: sceneNode.name,
            documentNode: index,
            // Keep the complete animation-adapter node contract here. The
            // adapter is shared with the established direct FBX path, whose
            // rest/animation bases distinguish skin placement from authored
            // local animation values.
            trs: cloneTrs(bindTrs),
            restTrs: cloneTrs(bindTrs),
            bindTrs,
            animationTrs,
            hasComplexTransformStack: Boolean(source.hasComplexTransformStack),
            usesAuthoredModelTrs: Boolean(sourceMatrix && !source.hasComplexTransformStack),
            localMatrix: rawLocalMatrix,
            weights: Float32Array.from(morphWeights(source.meshes?.[0])),
            source,
        };
        if (state.id !== null) stateById.set(state.id, state);
        if (source.name) stateByName.set(source.name, state);
        sceneNode.children.push(...(source.children || []).map((child) => append(child, bind || parentBind)));
        return index;
    };
    for (const root of roots) rootsOut.push(append(root));
    document.nodes.push(...nodes);
    document.rootNodes.push(...rootsOut);
    return { stateById, stateByName, bindPoseByNodeId, nodes };
}

function attachMeshesAndSkins(roots, state, materialMap, document) {
    const append = (source, ownerState) => {
        const ownerNode = document.nodes[ownerState.documentNode];
        const meshBindings = [];
        for (const sourceMesh of source.meshes || []) {
            const meshIndex = appendMesh(sourceMesh, materialMap, document);
            const skinIndex = appendSkin(sourceMesh, ownerState, state, document);
            meshBindings.push({ meshIndex, skinIndex });
        }
        if (meshBindings.length === 1) {
            ownerNode.mesh = meshBindings[0].meshIndex;
            if (meshBindings[0].skinIndex >= 0) ownerNode.skin = meshBindings[0].skinIndex;
            ownerNode.weights = morphWeights(source.meshes?.[0]);
        } else if (meshBindings.length > 1) {
            for (const binding of meshBindings) {
                const childIndex = document.nodes.length;
                document.nodes.push({
                    name: `${ownerNode.name}_mesh_${childIndex}`,
                    translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1],
                    mesh: binding.meshIndex,
                    ...(binding.skinIndex >= 0 ? { skin: binding.skinIndex } : {}),
                });
                ownerNode.children.push(childIndex);
            }
        }
        for (const child of source.children || []) {
            const childState = state.stateById.get(child.id);
            if (childState) append(child, childState);
        }
    };
    for (const root of roots) {
        const rootState = state.stateById.get(root.id);
        if (rootState) append(root, rootState);
    }
}

function appendMesh(source, materialMap, document) {
    const vertexCount = (source.positions?.length || 0) / 3;
    const attributes = { POSITION: appendFloatAccessor(document, scaleVector3(source.positions || []), 3) };
    if (source.normals?.length === vertexCount * 3) attributes.NORMAL = appendFloatAccessor(document, source.normals, 3);
    if (source.uvs?.length === vertexCount * 2) attributes.TEXCOORD_0 = appendFloatAccessor(document, source.uvs, 2);
    if (source.joints0?.length === vertexCount * 4 && source.weights0?.length === vertexCount * 4) {
        attributes.JOINTS_0 = appendAccessor(document, { bytes: bytesFromU16(source.joints0), componentType: 5123, components: 4, count: vertexCount, normalized: false });
        attributes.WEIGHTS_0 = appendFloatAccessor(document, source.weights0, 4);
    }
    const targets = appendMorphTargets(source, vertexCount, document);
    const indices = source.indices || [];
    const groups = materialGroups(indices, source.materialIndices, source.material, materialMap, document, source.name);
    const primitives = groups.map((group) => ({
        attributes,
        indices: appendAccessor(document, { bytes: bytesFromU32(group.indices), componentType: 5125, components: 1, count: group.indices.length, normalized: false }),
        mode: 4,
        ...(group.material >= 0 ? { material: group.material } : {}),
        ...(targets.length > 0 ? { targets } : {}),
    }));
    const index = document.meshes.length;
    document.meshes.push({ name: source.name || `mesh_${index}`, weights: morphWeights(source), primitives });
    if (source.skin?.clusters?.some((cluster) => cluster.controlPointIndices?.length > 0) && (!source.joints0?.length || !source.weights0?.length)) {
        document.warnings.push(`FBX mesh ${source.name || index} lacks render JOINTS_0/WEIGHTS_0 expansion and cannot be skinned by the portable runtime`);
    }
    return index;
}

function appendSkin(sourceMesh, ownerState, state, document) {
    const clusters = sourceMesh.skin?.clusters || [];
    if (clusters.length === 0) return -1;
    const bindPose = new Map((sourceMesh.skin.bindPose || []).filter((entry) => validMatrix(entry.matrix)).map((entry) => [entry.nodeId, entry.matrix]));
    const joints = [];
    const matrices = [];
    for (const cluster of clusters) {
        const jointState = state.stateById.get(cluster.jointNodeId);
        if (!jointState) {
            document.warnings.push(`FBX skin cluster targets missing joint ${cluster.jointNodeId} and was omitted`);
            continue;
        }
        const meshBind = scaleMatrixTranslation(bindPose.get(ownerState.id) || cluster.meshBindTransform || identityMat4());
        const jointBind = scaleMatrixTranslation(jointState.hasComplexTransformStack
            ? (bindPose.get(cluster.jointNodeId) || cluster.jointBindTransform || identityMat4())
            : (cluster.jointBindTransform || bindPose.get(cluster.jointNodeId) || identityMat4()));
        const inverse = invertMat4(jointBind) || identityMat4();
        joints.push(jointState.documentNode);
        matrices.push(...(multiplyMat4(inverse, meshBind) || inverse));
    }
    if (joints.length === 0) return -1;
    const inverseBindMatrices = appendFloatAccessor(document, matrices, 16);
    const index = document.skins.length;
    document.skins.push({ name: `${sourceMesh.name || 'mesh'}_skin`, joints, inverseBindMatrices });
    return index;
}

function appendMorphTargets(source, vertexCount, document) {
    return (source.morphTargets || []).map((target) => {
        const position = new Float32Array(vertexCount * 3);
        const renderIndices = target.renderPointIndices || [];
        const renderDeltas = target.renderPositionDeltas || [];
        for (let entry = 0; entry < renderIndices.length; entry += 1) {
            const render = renderIndices[entry] * 3;
            const delta = entry * 3;
            if (render + 2 < position.length && delta + 2 < renderDeltas.length) position.set(scaleVector3(renderDeltas.slice(delta, delta + 3)), render);
        }
        const output = { POSITION: appendFloatAccessor(document, position, 3) };
        if (target.renderNormalDeltas?.length) {
            const normal = new Float32Array(vertexCount * 3);
            for (let entry = 0; entry < renderIndices.length; entry += 1) {
                const render = renderIndices[entry] * 3;
                const delta = entry * 3;
                if (render + 2 < normal.length && delta + 2 < target.renderNormalDeltas.length) normal.set(target.renderNormalDeltas.slice(delta, delta + 3), render);
            }
            output.NORMAL = appendFloatAccessor(document, normal, 3);
        }
        return output;
    });
}

function appendFbxAnimations(clips, state, document) {
    for (const clip of clips) {
        const adapted = adaptFbxAnimation(clip, state.stateById, state.stateByName);
        if (!adapted) continue;
        const byNode = new Map();
        for (const channel of adapted.channels) {
            if (!byNode.has(channel.node)) byNode.set(channel.node, []);
            byNode.get(channel.node).push(channel);
        }
        const samplers = [];
        const channels = [];
        for (const [node, nodeChannels] of byNode) {
            const time = nodeChannels[0].sampler.input;
            for (const path of ['translation', 'rotation', 'scale']) {
                const sourceChannel = nodeChannels.find((channel) => channel.path === path);
                const channel = sourceChannel ? scaleFbxAnimationChannel(sourceChannel) : constantFbxChannel(node, path, time);
                const sampler = appendAnimationSampler(document, channel.sampler, path === 'weights' ? channel.targetCount : undefined);
                const samplerIndex = samplers.length;
                samplers.push(sampler);
                channels.push({ sampler: samplerIndex, node: node.documentNode, path });
            }
            for (const channel of nodeChannels.filter((candidate) => candidate.path === 'weights')) {
                const samplerIndex = samplers.length;
                samplers.push(appendAnimationSampler(document, channel.sampler, channel.targetCount));
                channels.push({ sampler: samplerIndex, node: node.documentNode, path: 'weights' });
            }
        }
        if (channels.length > 0) document.animations.push({ name: adapted.name, duration: adapted.duration, samplers, channels });
    }
}

function constantFbxChannel(node, path, input) {
    const values = path === 'translation'
        ? node.animationTrs.translation.map((value) => value * FBX_CENTIMETERS_TO_METERS)
        : path === 'rotation' ? node.animationTrs.rotation : node.animationTrs[path];
    const output = new Float32Array(input.length * values.length);
    for (let index = 0; index < input.length; index += 1) output.set(values, index * values.length);
    return { path, sampler: { input, output, interpolation: 'LINEAR' } };
}

function scaleFbxAnimationChannel(channel) {
    if (channel.path !== 'translation') return channel;
    return {
        ...channel,
        sampler: { ...channel.sampler, output: scaleVector3(channel.sampler.output) },
    };
}

function appendAnimationSampler(document, sampler, targetCount) {
    const input = appendFloatAccessor(document, sampler.input, 1);
    const components = targetCount || (sampler.output.length / (sampler.input.length * ((sampler.interpolation || 'LINEAR') === 'CUBICSPLINE' ? 3 : 1)));
    const output = appendFloatAccessor(document, sampler.output, components);
    return { input, output, interpolation: sampler.interpolation || 'LINEAR' };
}

function materialGroups(indices, materialIndices, fallback, materialMap, document, name) {
    const source = Array.from(indices || []);
    const groups = new Map();
    for (let offset = 0; offset + 2 < source.length; offset += 3) {
        const sourceMaterial = materialIndices?.[offset / 3] ?? fallback;
        const material = Number.isInteger(sourceMaterial) ? (materialMap[sourceMaterial] ?? -1) : -1;
        if (!groups.has(material)) groups.set(material, []);
        groups.get(material).push(source[offset], source[offset + 1], source[offset + 2]);
    }
    if (groups.size > 1) document.warnings.push(`FBX mesh ${name || 'mesh'} material assignments were split into portable primitives`);
    return [...groups.entries()].map(([material, groupIndices]) => ({ material, indices: groupIndices }));
}

function morphWeights(mesh) {
    return Array.from(mesh?.morphTargets || [], (target) => (Number(target.defaultWeight) || 0) / 100);
}

function appendFloatAccessor(document, values, components) {
    return appendAccessor(document, { bytes: bytesFromF32(values), componentType: 5126, components, count: values.length / components, normalized: false });
}

function appendAccessor(document, accessor) {
    const index = document.accessors.length;
    document.accessors.push(accessor);
    return index;
}

function bytesFromF32(values) {
    const source = Float32Array.from(values);
    return new Uint8Array(source.buffer);
}

function bytesFromU16(values) {
    const source = Uint16Array.from(values);
    return new Uint8Array(source.buffer);
}

function bytesFromU32(values) {
    const source = Uint32Array.from(values);
    return new Uint8Array(source.buffer);
}

function decomposeMatrix(matrix) {
    const scale = [Math.hypot(matrix[0], matrix[1], matrix[2]) || 1, Math.hypot(matrix[4], matrix[5], matrix[6]) || 1, Math.hypot(matrix[8], matrix[9], matrix[10]) || 1];
    const m00 = matrix[0] / scale[0], m01 = matrix[4] / scale[1], m02 = matrix[8] / scale[2];
    const m10 = matrix[1] / scale[0], m11 = matrix[5] / scale[1], m12 = matrix[9] / scale[2];
    const m20 = matrix[2] / scale[0], m21 = matrix[6] / scale[1], m22 = matrix[10] / scale[2];
    const trace = m00 + m11 + m22;
    let rotation;
    if (trace > 0) { const s = Math.sqrt(trace + 1) * 2; rotation = [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s]; }
    else if (m00 > m11 && m00 > m22) { const s = Math.sqrt(1 + m00 - m11 - m22) * 2; rotation = [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]; }
    else if (m11 > m22) { const s = Math.sqrt(1 + m11 - m00 - m22) * 2; rotation = [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s]; }
    else { const s = Math.sqrt(1 + m22 - m00 - m11) * 2; rotation = [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s]; }
    const length = Math.hypot(...rotation) || 1;
    return { translation: [matrix[12], matrix[13], matrix[14]], rotation: rotation.map((value) => value / length), scale };
}

function cloneTrs(trs) {
    return { translation: [...trs.translation], rotation: [...trs.rotation], scale: [...trs.scale] };
}

function scaleVector3(values) {
    const output = new Float32Array(values.length);
    for (let index = 0; index < values.length; index += 1) output[index] = values[index] * FBX_CENTIMETERS_TO_METERS;
    return output;
}

function scaleMatrixTranslation(matrix) {
    const output = Float32Array.from(matrix);
    output[12] *= FBX_CENTIMETERS_TO_METERS;
    output[13] *= FBX_CENTIMETERS_TO_METERS;
    output[14] *= FBX_CENTIMETERS_TO_METERS;
    return output;
}

function validMatrix(matrix) {
    return Array.isArray(matrix) && matrix.length === 16 && matrix.every(Number.isFinite);
}

function defaultSampler() {
    return { wrapS: 10497, wrapT: 10497, minFilter: 9987, magFilter: 9729 };
}

function resolveResource(uri, resources) {
    if (!uri) return null;
    const value = resources?.[uri] || resources?.[basename(uri)];
    if (value instanceof Uint8Array) return value;
    if (value instanceof ArrayBuffer) return new Uint8Array(value);
    return null;
}

function basename(path) {
    const slash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    return slash >= 0 ? path.slice(slash + 1) : path;
}

function mimeFromUri(uri) {
    const extension = basename(uri || '').split('.').pop()?.toLowerCase();
    return { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', webp: 'image/webp' }[extension] || null;
}

function sniffMime(bytes) {
    if (bytes.length >= 4 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return 'image/png';
    if (bytes.length >= 2 && bytes[0] === 0xff && bytes[1] === 0xd8) return 'image/jpeg';
    if (bytes.length >= 4 && bytes[0] === 0x52 && bytes[1] === 0x49 && bytes[2] === 0x46 && bytes[3] === 0x46) return 'image/webp';
    return null;
}
