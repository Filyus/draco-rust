/**
 * Portable SceneDocument -> typed FBX SceneInput adapter.
 *
 * This is the FBX export boundary: the document contract stays format-neutral,
 * while axis/unit conversion, Euler lowering, skin-cluster construction, and
 * optional source provenance are kept here. The returned object is consumed by
 * fbx-wasm.create_fbx_scene; no flattened mesh writer is involved.
 */

import { assertValidSceneDocument } from './scene-document.js';
import { invertMat4, multiplyMat4 } from './mat4.js';
import {
    convertGltfMatrixToFbx,
    convertGltfVectorArrayToFbx,
    quaternionKeysToFbxEuler,
} from './fbx-scene-adapter.js';
import { assertFbxProvenance } from './fbx-scene-provenance.js';

const COMPONENT_BYTES = new Map([
    [5120, 1], [5121, 1], [5122, 2], [5123, 2], [5125, 4], [5126, 4],
]);

/** Build a typed-writer SceneInput from a validated portable document. */
export function buildFbxSceneFromDocument(document, options = {}) {
    assertValidSceneDocument(document);
    const provenance = options.provenance || null;
    if (provenance) assertFbxProvenance(provenance);
    const sourceScene = provenance?.sourceScene || null;
    const sourceNodes = indexSourceNodes(sourceScene?.rootNodes || []);
    const sourceMeshes = indexSourceMeshes(sourceScene?.rootNodes || []);
    const sourceUnits = Boolean(provenance);
    const worlds = buildDocumentWorlds(document);
    const warnings = [...document.warnings];

    const scene = {
        ...(sourceScene?.globalSettings ? { globalSettings: sourceScene.globalSettings } : {}),
        rootNodes: document.rootNodes.map((index) => buildNode(
            document,
            index,
            sourceNodes,
            sourceMeshes,
            worlds,
            sourceUnits,
            warnings,
        )),
        materials: buildMaterials(document),
        textures: buildTextures(document),
        animations: sourceScene?.animations?.length
            ? structuredClone(sourceScene.animations)
            : buildAnimations(document, sourceUnits, warnings),
        warnings,
    };
    return scene;
}

function buildNode(document, index, sourceNodes, sourceMeshes, worlds, sourceUnits, warnings) {
    const node = document.nodes[index] || {};
    const source = sourceNodes.get(node.name);
    const matrix = source?.matrix?.length === 16
        ? Array.from(source.matrix)
        : convertNodeMatrix(node, sourceUnits);
    const meshes = node.mesh === undefined ? [] : buildNodeMeshes(
        document,
        node,
        node.mesh,
        index,
        sourceMeshes,
        worlds,
        sourceUnits,
        warnings,
    );
    return {
        id: index + 1,
        name: node.name || `node_${index}`,
        matrix,
        ...(source?.transformStack ? { transformStack: structuredClone(source.transformStack) } : {}),
        meshes,
        children: (node.children || []).map((child) => buildNode(
            document,
            child,
            sourceNodes,
            sourceMeshes,
            worlds,
            sourceUnits,
            warnings,
        )),
    };
}

function buildNodeMeshes(document, node, meshIndex, nodeIndex, sourceMeshes, worlds, sourceUnits, warnings) {
    const mesh = document.meshes[meshIndex];
    if (!mesh) return [];
    return mesh.primitives.map((primitive, primitiveIndex) => {
        const positions = readAccessorValues(document, primitive.attributes.POSITION);
        const indices = primitive.indices === undefined
            ? Array.from({ length: positions.length / 3 }, (_, value) => value)
            : readAccessorValues(document, primitive.indices).map((value) => Math.max(0, Math.trunc(value)));
        const sourceMesh = findSourceMesh(sourceMeshes, document.meshes, mesh.name, meshIndex, primitiveIndex);
        // With provenance present, an unmatched source mesh is the one case
        // where FBX-only data is genuinely lost: extra UV/normal/colour layers,
        // tangents, binormals, hard edges and creases all come from it. Without
        // provenance there is nothing to lose, so this is the only reachable
        // point at which the loss can be reported.
        if (sourceUnits && !sourceMesh) {
            warnings.push(`FBX mesh ${mesh.name ?? meshIndex} could not be matched to its source geometry, so FBX-only layers were not re-exported`);
        }
        const meshInput = {
            name: mesh.name ? `${mesh.name}_${primitiveIndex}` : `mesh_${meshIndex}_${primitiveIndex}`,
            positions: sourceUnits ? scaleValues(positions, 100) : convertGltfVectorArrayToFbx(positions),
            indices,
            normals: optionalAttribute(document, primitive, 'NORMAL', null),
            uvs: optionalAttribute(document, primitive, 'TEXCOORD_0', null),
            materialIndices: Array.from({ length: Math.floor(indices.length / 3) }, () => primitive.material ?? -1),
            morphTargets: buildMorphTargets(document, primitive, sourceUnits),
        };
        if (meshInput.normals && !sourceUnits) meshInput.normals = convertGltfVectorArrayToFbx(meshInput.normals);
        if (meshInput.uvs && !sourceUnits) meshInput.uvs = flipUvV(meshInput.uvs);
        if (!sourceUnits) {
            const uvSets = [];
            for (let set = 1; set < 8; set += 1) {
                const values = optionalAttribute(document, primitive, `TEXCOORD_${set}`, flipUvV);
                if (!values) break;
                uvSets.push({ name: `UVSet${set}`, mapping: 'ByPolygonVertex', reference: 'Direct', values, indices: [] });
            }
            if (uvSets.length > 0) meshInput.uvSets = uvSets;
        }
        if (sourceUnits && sourceMesh?.uvSets?.length) meshInput.uvSets = structuredClone(sourceMesh.uvSets);
        if (sourceUnits && sourceMesh?.normalSets?.length) meshInput.normalSets = structuredClone(sourceMesh.normalSets);
        if (sourceUnits && sourceMesh?.colorSets?.length) {
            meshInput.colorSets = structuredClone(sourceMesh.colorSets);
        } else {
            // COLOR_0 is corner-domain RGBA, which is exactly what
            // LayerElementColor stores as ByPolygonVertex/Direct.
            const colors = optionalAttribute(document, primitive, 'COLOR_0', null);
            if (colors?.length) {
                const vertexCount = positions.length / 3;
                // glTF allows COLOR_0 as VEC3 or VEC4; FBX stores RGBA.
                const rgba = colors.length === vertexCount * 4
                    ? Array.from(colors)
                    : Array.from({ length: vertexCount * 4 }, (_, index) => (
                        index % 4 === 3 ? 1 : colors[Math.floor(index / 4) * 3 + (index % 4)]
                    ));
                meshInput.colorSets = [{
                    name: 'Col', mapping: 'ByPolygonVertex', reference: 'Direct', values: rgba, indices: [],
                }];
            }
        }
        // Smoothing flags and crease weights address edges, polygons or control
        // points, none of which the portable document represents, so they only
        // travel on the FBX provenance path.
        if (sourceUnits && sourceMesh?.smoothingLayers?.length) {
            meshInput.smoothingLayers = structuredClone(sourceMesh.smoothingLayers);
        }
        if (sourceUnits && sourceMesh?.creaseLayers?.length) {
            meshInput.creaseLayers = structuredClone(sourceMesh.creaseLayers);
        }
        if (sourceUnits && sourceMesh?.tangentSets?.length) {
            meshInput.tangentSets = structuredClone(sourceMesh.tangentSets);
            if (sourceMesh.binormalSets?.length) meshInput.binormalSets = structuredClone(sourceMesh.binormalSets);
        } else {
            // glTF TANGENT is xyzw with the handedness sign in w, which is what
            // the writer splits back into Tangents and TangentsW.
            const tangents = optionalAttribute(document, primitive, 'TANGENT', null);
            if (tangents?.length === (positions.length / 3) * 4) {
                // Same Y-up to Z-up swap the normals get, applied to xyz only.
                // It is a rotation, so the handedness in w is unaffected.
                const values = Array.from(tangents);
                if (!sourceUnits) {
                    for (let offset = 0; offset + 3 < values.length; offset += 4) {
                        const y = values[offset + 1];
                        values[offset + 1] = values[offset + 2];
                        values[offset + 2] = -y;
                    }
                }
                meshInput.tangentSets = [{
                    name: '', mapping: 'ByPolygonVertex', reference: 'Direct',
                    // The sign came from glTF, so it is real data rather than
                    // the reader's default and is written out as TangentsW.
                    values, indices: [], hasHandedness: true,
                }];
            }
        }
        const skinIndex = node.skin;
        if (Number.isInteger(skinIndex) && document.skins[skinIndex]) {
            meshInput.skin = sourceUnits && sourceMesh?.skin
                ? structuredClone(sourceMesh.skin)
                : buildSkin(document, primitive, skinIndex, nodeIndex, worlds, sourceUnits, warnings);
        }
        if (sourceUnits && sourceMesh?.controlPoints?.length) {
            meshInput.controlPoints = sourceMesh.controlPoints.flat();
            meshInput.polygonVertexIndices = sourceMesh.polygonVertexIndices?.slice() || [];
        }
        return meshInput;
    });
}

function optionalAttribute(document, primitive, semantic, transform) {
    const index = primitive.attributes?.[semantic];
    if (!Number.isInteger(index)) return null;
    const values = readAccessorValues(document, index);
    return transform ? transform(values) : values;
}

function buildSkin(document, primitive, skinIndex, nodeIndex, worlds, sourceUnits, warnings) {
    const skin = document.skins[skinIndex];
    const joints = skin.joints || [];
    const influenceSets = [];
    for (let set = 0; set < 8; set += 1) {
        const jointValues = optionalAttribute(document, primitive, `JOINTS_${set}`, null);
        const weightValues = optionalAttribute(document, primitive, `WEIGHTS_${set}`, null);
        if (jointValues === null && weightValues === null) break;
        if (!jointValues || !weightValues || jointValues.length !== weightValues.length || jointValues.length % 4 !== 0) {
            warnings.push(`FBX export skin ${skinIndex} has unaligned JOINTS_${set}/WEIGHTS_${set} attributes`);
            continue;
        }
        influenceSets.push({ jointValues, weightValues });
    }
    if (influenceSets.length === 0) {
        warnings.push(`FBX export skin ${skinIndex} lacks aligned joint/weight attributes`);
        return null;
    }
    const inverseBinds = skin.inverseBindMatrices === undefined
        ? [] : readAccessorMatrices(document, skin.inverseBindMatrices);
    const meshWorld = worlds[nodeIndex] || identityMatrix();
    const clusters = joints.map((jointIndex, jointSlot) => {
        const jointWorld = inverseBinds[jointSlot] ? (invertMat4(inverseBinds[jointSlot]) || identityMatrix()) : worlds[jointIndex] || identityMatrix();
        const convertedJoint = convertMatrixForFbx(jointWorld, sourceUnits);
        const convertedMesh = convertMatrixForFbx(meshWorld, sourceUnits);
        const meshBind = multiplyMat4(convertedMesh, invertMat4(convertedJoint) || identityMatrix());
        const controlPointIndices = [];
        const weights = [];
        const vertexCount = influenceSets[0].jointValues.length / 4;
        for (let vertex = 0; vertex < vertexCount; vertex += 1) {
            for (const { jointValues, weightValues } of influenceSets) {
                for (let component = 0; component < 4; component += 1) {
                    if (Math.trunc(jointValues[vertex * 4 + component]) !== jointSlot) continue;
                    const weight = Number(weightValues[vertex * 4 + component]) || 0;
                    if (weight <= 0) continue;
                    controlPointIndices.push(vertex);
                    weights.push(weight);
                }
            }
        }
        return {
            jointNodeId: jointIndex + 1,
            controlPointIndices,
            weights,
            meshBindTransform: meshBind,
            jointBindTransform: convertedJoint,
        };
    }).filter((cluster) => cluster.weights.length > 0);
    const bindPose = [
        { nodeId: nodeIndex + 1, matrix: convertMatrixForFbx(meshWorld, sourceUnits) },
        ...joints.map((jointIndex, slot) => ({
            nodeId: jointIndex + 1,
            matrix: convertMatrixForFbx(
                inverseBinds[slot] ? (invertMat4(inverseBinds[slot]) || identityMatrix()) : worlds[jointIndex] || identityMatrix(),
                sourceUnits,
            ),
        })),
    ];
    return { clusters, bindPose };
}

function buildMorphTargets(document, primitive, sourceUnits) {
    return (primitive.targets || []).flatMap((target, index) => {
        if (target.POSITION === undefined) return [];
        const position = readAccessorValues(document, target.POSITION);
        const normal = target.NORMAL === undefined ? null : readAccessorValues(document, target.NORMAL);
        return [{
            name: `target_${index}`,
            controlPointIndices: Array.from({ length: position.length / 3 }, (_, value) => value),
            positionDeltas: sourceUnits ? scaleValues(position, 100) : convertGltfVectorArrayToFbx(position),
            ...(normal ? { normalDeltas: sourceUnits ? normal : convertGltfVectorArrayToFbx(normal) } : {}),
            defaultWeight: 0,
            fullWeight: 100,
        }];
    });
}

function buildMaterials(document) {
    return document.materials.map((material, index) => {
        const base = material.baseColorFactor || [1, 1, 1, 1];
        const textures = [];
        const add = (info, slot) => {
            if (Number.isInteger(info?.texture)) textures.push({ slot, textureIndex: info.texture });
        };
        add(material.baseColorTexture, 'diffuse');
        add(material.normalTexture, 'normal');
        add(material.emissiveTexture, 'emissive');
        add(material.metallicRoughnessTexture, 'roughness');
        return {
            name: material.name || `material_${index}`,
            shadingModel: 'Phong',
            diffuse: base.slice(0, 3),
            diffuseFactor: 1,
            emissive: material.emissiveFactor || [0, 0, 0],
            emissiveFactor: 1,
            reflectionFactor: material.metallicFactor ?? 0,
            shininess: Math.max(0, Math.min(1, material.roughnessFactor ?? 1)) * -100 + 100,
            opacity: base[3] ?? 1,
            textures,
        };
    });
}

function buildTextures(document) {
    return document.textures.map((texture, index) => {
        const resource = document.resources[texture.resource];
        return {
            name: texture.name || resource?.name || `texture_${index}`,
            content: resource?.bytes ? Array.from(resource.bytes) : null,
            filename: resource?.name || null,
        };
    });
}

function buildAnimations(document, sourceUnits, warnings) {
    return document.animations.map((clip) => ({
        name: clip.name,
        duration: clip.duration,
        channels: clip.channels.flatMap((channel) => {
            const sampler = clip.samplers[channel.sampler];
            if (!sampler || !document.nodes[channel.node]) return [];
            const input = readAccessorValues(document, sampler.input);
            const cubic = sampler.interpolation === 'CUBICSPLINE';
            const components = channel.path === 'rotation' ? 4 : channel.path === 'weights'
                ? document.accessors[sampler.output].components : 3;
            const values = readAccessorValues(document, sampler.output);
            const keyValues = cubic ? extractCubic(values, components, 1) : values;
            let output;
            if (channel.path === 'rotation') output = quaternionKeysToFbxEuler(keyValues);
            else if (channel.path === 'translation') output = convertGltfVectorArrayToFbx(keyValues);
            else output = keyValues;
            if (channel.path === 'weights') {
                const targetCount = components;
                for (let target = 0; target < targetCount; target += 1) {
                    const scalar = cubic ? keyValues.filter((_, index) => index % targetCount === target) : values.filter((_, index) => index % targetCount === target);
                    // Weight channels are already normalized in SceneDocument;
                    // FBX writer samples them as percentages.
                    output = scalar.map((value) => value * 100);
                    return [{
                        nodeName: document.nodes[channel.node].name,
                        nodeId: channel.node + 1,
                        morphTargetIndex: target,
                        path: 'morphweight',
                        sampler: { input, output, interpolation: 'linear' },
                    }];
                }
            }
            if (output.length !== input.length * 3) {
                warnings.push(`Animation ${clip.name}: ${channel.path} sampler was omitted from FBX export`);
                return [];
            }
            return [{
                nodeName: document.nodes[channel.node].name,
                nodeId: channel.node + 1,
                path: channel.path,
                sampler: {
                    input,
                    output: sourceUnits && channel.path === 'translation' ? scaleValues(output, 100) : output,
                    interpolation: cubic && channel.path !== 'rotation' ? 'cubic' : 'linear',
                    inTangents: null,
                    outTangents: null,
                },
            }];
        }),
    }));
}

function extractCubic(values, components, segment) {
    const stride = components * 3;
    const output = [];
    for (let offset = 0; offset + stride <= values.length; offset += stride) output.push(...values.slice(offset + components * segment, offset + components * (segment + 1)));
    return output;
}

function convertNodeMatrix(node, sourceUnits) {
    const matrix = node.matrix || composeMatrix(node.translation, node.rotation, node.scale);
    return convertMatrixForFbx(matrix, sourceUnits);
}

function convertMatrixForFbx(matrix, sourceUnits) {
    if (sourceUnits) {
        const output = Array.from(matrix);
        output[12] *= 100;
        output[13] *= 100;
        output[14] *= 100;
        return output;
    }
    return convertGltfMatrixToFbx(matrix);
}

function buildDocumentWorlds(document) {
    const worlds = Array.from({ length: document.nodes.length }, () => null);
    const visit = (index, parent) => {
        if (worlds[index]) return;
        const node = document.nodes[index] || {};
        const local = node.matrix || composeMatrix(node.translation, node.rotation, node.scale);
        worlds[index] = parent ? multiplyMat4(parent, local) : Array.from(local);
        for (const child of node.children || []) visit(child, worlds[index]);
    };
    for (const root of document.rootNodes) visit(root, null);
    document.nodes.forEach((_, index) => { if (!worlds[index]) visit(index, null); });
    return worlds;
}

function composeMatrix(translation = [0, 0, 0], rotation = [0, 0, 0, 1], scale = [1, 1, 1]) {
    const [x, y, z, w] = rotation;
    const [sx, sy, sz] = scale;
    return [
        (1 - 2 * (y * y + z * z)) * sx, (2 * (x * y + z * w)) * sx, (2 * (x * z - y * w)) * sx, 0,
        (2 * (x * y - z * w)) * sy, (1 - 2 * (x * x + z * z)) * sy, (2 * (y * z + x * w)) * sy, 0,
        (2 * (x * z + y * w)) * sz, (2 * (y * z - x * w)) * sz, (1 - 2 * (x * x + y * y)) * sz, 0,
        translation[0], translation[1], translation[2], 1,
    ];
}

function readAccessorValues(document, index) {
    const accessor = document.accessors[index];
    if (!accessor) return [];
    const bytes = COMPONENT_BYTES.get(accessor.componentType);
    if (!bytes) throw new Error(`Unsupported SceneDocument accessor component type ${accessor.componentType}`);
    const view = new DataView(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength);
    const values = [];
    for (let item = 0; item < accessor.count * accessor.components; item += 1) {
        const value = readComponent(view, item * bytes, accessor.componentType);
        values.push(accessor.normalized ? normalizeComponent(value, accessor.componentType) : value);
    }
    return values;
}

function readAccessorMatrices(document, index) {
    const accessor = document.accessors[index];
    if (!accessor || accessor.components !== 16) return [];
    const values = readAccessorValues(document, index);
    return Array.from({ length: accessor.count }, (_, item) => values.slice(item * 16, item * 16 + 16));
}

function readComponent(view, offset, componentType) {
    switch (componentType) {
        case 5120: return view.getInt8(offset);
        case 5121: return view.getUint8(offset);
        case 5122: return view.getInt16(offset, true);
        case 5123: return view.getUint16(offset, true);
        case 5125: return view.getUint32(offset, true);
        case 5126: return view.getFloat32(offset, true);
        default: throw new Error(`Unsupported SceneDocument component type ${componentType}`);
    }
}

function normalizeComponent(value, componentType) {
    if (componentType === 5120) return Math.max(-1, value / 127);
    if (componentType === 5121) return value / 255;
    if (componentType === 5122) return Math.max(-1, value / 32767);
    if (componentType === 5123) return value / 65535;
    return value;
}

function scaleValues(values, factor) {
    return Array.from(values, (value) => value * factor);
}

function flipUvV(values) {
    const output = Array.from(values);
    for (let index = 1; index < output.length; index += 2) output[index] = 1 - output[index];
    return output;
}

function identityMatrix() {
    return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
}

function indexSourceNodes(roots) {
    const map = new Map();
    const visit = (node) => {
        if (node.name) map.set(node.name, node);
        for (const child of node.children || []) visit(child);
    };
    roots.forEach(visit);
    return map;
}

/**
 * Index the source meshes provenance can restore from, by name and by position.
 *
 * Name alone is not enough: an FBX Geometry need not be named, and the document
 * then synthesizes `mesh_<position>` for it. Keying only by name silently
 * skipped every unnamed geometry -- 229 of 655 meshes across the ufbx corpus --
 * so their UV, normal, colour and tangent layers, skin and control points all
 * fell back to the lossy portable path.
 *
 * The positional list is only usable when both traversals agree on how many
 * meshes there are; the document builder drops nodes it has no state for, which
 * would otherwise shift the pairing and restore the wrong mesh's layers.
 */
function indexSourceMeshes(roots) {
    const byName = new Map();
    const ordered = [];
    const visit = (node) => {
        for (const mesh of node.meshes || []) {
            if (mesh.name) byName.set(mesh.name, mesh);
            ordered.push(mesh);
        }
        for (const child of node.children || []) visit(child);
    };
    roots.forEach(visit);
    return { byName, ordered };
}

/** Resolve the source mesh a document mesh came from, or undefined. */
function findSourceMesh(sourceMeshes, documentMeshes, name, meshIndex, primitiveIndex) {
    const byName = sourceMeshes.byName.get(name)
        || sourceMeshes.byName.get(`${name}_${primitiveIndex}`);
    if (byName) return byName;
    if (sourceMeshes.ordered.length !== documentMeshes.length) return undefined;
    const positional = sourceMeshes.ordered[meshIndex];
    // A named source mesh would already have matched above, so a name here
    // means the two orderings disagree and the pairing cannot be trusted.
    return positional && !positional.name ? positional : undefined;
}
