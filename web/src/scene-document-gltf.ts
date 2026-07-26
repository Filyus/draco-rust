/**
 * SceneDocument -> glTF/GLB export boundary.
 *
 * This module only lowers the portable contract into a glTF JSON document and
 * its byte-backed resource map. Container validation and GLB construction are
 * deliberately delegated to GltfAsset (gltf-wasm / draco-gltf), rather than
 * reimplemented in JavaScript.
 */

import { assertValidSceneDocument } from './scene-document.ts';
import type {
    SceneAccessor,
    SceneAnimation,
    SceneDocument,
    SceneMaterial,
    SceneMesh,
    SceneNode,
    SceneSkin,
    TextureInfo,
} from './scene-document.ts';
import type { GltfAsset, GltfModule } from './wasm-modules.ts';

/**
 * The lowered glTF pieces. These describe what this module writes, not the
 * whole glTF schema: anything the exporter never emits is deliberately absent.
 */
interface GltfBufferView {
    buffer: number;
    byteOffset: number;
    byteLength: number;
    target?: number;
}

interface GltfTextureInfo {
    index: number;
    texCoord: number;
    scale?: number;
    strength?: number;
    extensions?: Record<string, unknown>;
}

interface GltfMaterial {
    name: string;
    pbrMetallicRoughness: Record<string, unknown>;
    emissiveFactor: number[];
    alphaMode: string;
    doubleSided: boolean;
    alphaCutoff?: number;
    normalTexture?: GltfTextureInfo;
    occlusionTexture?: GltfTextureInfo;
    emissiveTexture?: GltfTextureInfo;
    extensions?: Record<string, unknown>;
}

interface GltfTexture {
    name: string;
    sampler: number;
    source?: number;
    extensions?: Record<string, { source: number }>;
}

/** A node carries either a matrix or a TRS triple, never both. */
interface GltfNode {
    name: string;
    children?: number[];
    mesh?: number;
    skin?: number;
    weights?: number[];
    matrix?: number[];
    translation?: number[];
    rotation?: number[];
    scale?: number[];
}

interface Trs {
    translation: number[];
    rotation: number[];
    scale: number[];
}

const ACCESSOR_TYPES = new Map<number, string>([
    [1, 'SCALAR'], [2, 'VEC2'], [3, 'VEC3'], [4, 'VEC4'], [9, 'MAT3'], [16, 'MAT4'],
]);

/**
 * Lower a portable scene to valid glTF 2.0 JSON plus its single binary
 * companion resource. This is useful for callers that need a JSON bundle;
 * use {@link serializeSceneDocumentToGlb} for a self-contained GLB.
 */
export function lowerSceneDocumentToGltf(document: SceneDocument) {
    const validation = assertValidSceneDocument(document);
    const warnings = [...document.warnings, ...validation.warnings];
    const animatedNodes = new Set(document.animations.flatMap((clip) => clip.channels.map((channel) => channel.node)));
    const binary = new BinaryBuilder();
    const bufferViews: GltfBufferView[] = [];
    const positionAccessors = new Set(document.meshes.flatMap((mesh) => mesh.primitives.map((primitive) => primitive.attributes.POSITION)));
    const animationInputs = new Set(document.animations.flatMap((clip) => clip.samplers.map((sampler) => sampler.input)));
    const accessorTargets = geometryAccessorTargets(document.meshes);
    const accessors = document.accessors.map((accessor, index) => lowerAccessor(
        accessor, index, binary, bufferViews, positionAccessors.has(index), animationInputs.has(index), accessorTargets.get(index),
    ));
    const images: { name: string; bufferView: number; mimeType: string }[] = [];
    const imageByResource = new Map<number, number>();
    const samplers: Record<string, number>[] = [];
    const textures: (GltfTexture | null)[] = document.textures.map((texture, index) => {
        const resource = document.resources[texture.resource];
        const sourceExtension = textureSourceExtension(resource?.mimeType);
        if (!resource || !sourceExtension) {
            warnings.push(`SceneDocument texture ${index} was omitted: resource ${texture.resource} has no embeddable image MIME type`);
            return null;
        }
        let image = imageByResource.get(texture.resource);
        if (image === undefined) {
            const bufferView = appendBufferView(binary, bufferViews, resource.bytes);
            image = images.length;
            images.push({ name: resource.name || `image_${image}`, bufferView, mimeType: resource.mimeType });
            imageByResource.set(texture.resource, image);
        }
        const sampler = samplers.length;
        samplers.push({
            wrapS: texture.sampler?.wrapS ?? 10497,
            wrapT: texture.sampler?.wrapT ?? 10497,
            minFilter: texture.sampler?.minFilter ?? 9987,
            magFilter: texture.sampler?.magFilter ?? 9729,
        });
        return {
            name: texture.name || `texture_${index}`,
            sampler,
            ...(sourceExtension === 'core' ? { source: image } : { extensions: { [sourceExtension]: { source: image } } }),
        };
    });
    const materials = document.materials.map((material, index) => lowerMaterial(material, index, textures, warnings));
    const meshes = document.meshes.map((mesh, index) => lowerMesh(mesh, index, accessors.length, materials.length, warnings));
    const nodes = document.nodes.map((node, index) => lowerNode(node, index, animatedNodes, meshes.length, document.skins.length, warnings));
    const skins = document.skins.map((skin, index) => lowerSkin(skin, index, accessors.length));
    const animations = document.animations.map((clip, index) => lowerAnimation(clip, index, accessors.length, nodes.length));
    const extensionsUsed = new Set<string>();
    if (materials.some((material) => material.extensions?.KHR_materials_unlit)) extensionsUsed.add('KHR_materials_unlit');
    if (materials.some(materialUsesTextureTransform)) extensionsUsed.add('KHR_texture_transform');
    if (textures.some((texture) => texture?.extensions?.KHR_texture_basisu)) extensionsUsed.add('KHR_texture_basisu');
    if (textures.some((texture) => texture?.extensions?.EXT_texture_webp)) extensionsUsed.add('EXT_texture_webp');

    const manifest = {
        asset: { version: '2.0', generator: 'draco-rust SceneDocument exporter' },
        buffers: [{ uri: 'scene.bin', byteLength: binary.length }],
        bufferViews,
        accessors,
        ...(images.length > 0 ? { images } : {}),
        ...(samplers.length > 0 ? { samplers } : {}),
        ...(textures.some(Boolean) ? { textures: textures.filter(Boolean) } : {}),
        ...(materials.length > 0 ? { materials } : {}),
        ...(meshes.length > 0 ? { meshes } : {}),
        ...(nodes.length > 0 ? { nodes } : {}),
        ...(skins.length > 0 ? { skins } : {}),
        ...(animations.length > 0 ? { animations } : {}),
        scenes: [{ nodes: [...document.rootNodes] }],
        scene: 0,
        ...(extensionsUsed.size > 0 ? { extensionsUsed: [...extensionsUsed] } : {}),
    };
    const json = new TextEncoder().encode(JSON.stringify(manifest));
    return {
        json,
        resources: { 'scene.bin': binary.toBytes() },
        warnings,
        capabilities: { ...validation.capabilities, gltf20: true, glb: true },
    };
}

/** Create a typed GltfAsset from a portable document. The caller owns it. */
export function createGltfAssetFromSceneDocument(document: SceneDocument, gltfModule: GltfModule) {
    if (!gltfModule?.GltfAsset?.withResources) throw new Error('gltf-wasm GltfAsset.withResources is required for SceneDocument export');
    const lowered = lowerSceneDocumentToGltf(document);
    const asset = gltfModule.GltfAsset.withResources(lowered.json, lowered.resources, '2.0');
    if (typeof asset.validate === 'function') asset.validate('2.0');
    return { asset, ...lowered };
}

/** Serialize a portable scene through typed gltf-wasm into a validated GLB v2. */
export function serializeSceneDocumentToGlb(document: SceneDocument, gltfModule: GltfModule) {
    const { asset, json, resources, warnings, capabilities } = createGltfAssetFromSceneDocument(document, gltfModule);
    try {
        return { binary: asset.glb(2), json, resources, warnings, capabilities };
    } finally {
        asset.free();
    }
}

function lowerAccessor(
    accessor: SceneAccessor,
    index: number,
    binary: BinaryBuilder,
    bufferViews: GltfBufferView[],
    isPosition: boolean,
    isAnimationInput: boolean,
    target: number | undefined,
) {
    const type = ACCESSOR_TYPES.get(accessor.components);
    if (!type) throw new Error(`SceneDocument accessor ${index} has unsupported ${accessor.components}-component shape`);
    const bufferView = appendBufferView(binary, bufferViews, accessor.bytes, target);
    const bounds = (isPosition || isAnimationInput) && !accessor.min && !accessor.max ? accessorBounds(accessor) : null;
    return {
        bufferView,
        componentType: accessor.componentType,
        count: accessor.count,
        type,
        ...(accessor.normalized ? { normalized: true } : {}),
        ...(accessor.min ? { min: [...accessor.min] } : bounds ? { min: bounds.min } : {}),
        ...(accessor.max ? { max: [...accessor.max] } : bounds ? { max: bounds.max } : {}),
    };
}

function appendBufferView(
    binary: BinaryBuilder,
    bufferViews: GltfBufferView[],
    bytes: Uint8Array,
    target?: number,
) {
    binary.align();
    const byteOffset = binary.length;
    binary.write(bytes);
    const output: GltfBufferView = { buffer: 0, byteOffset, byteLength: bytes.byteLength };
    if (target !== undefined) output.target = target;
    bufferViews.push(output);
    return bufferViews.length - 1;
}

function lowerMaterial(
    material: SceneMaterial,
    index: number,
    textures: (GltfTexture | null)[],
    warnings: string[],
): GltfMaterial {
    const texture = (info: TextureInfo | undefined, label: string): GltfTextureInfo | null => {
        if (!info) return null;
        if (!Number.isInteger(info.texture) || !textures[info.texture]) {
            warnings.push(`SceneDocument material ${index} ${label} texture was omitted because its source is unavailable`);
            return null;
        }
        const output: GltfTextureInfo = { index: textureIndex(textures, info.texture), texCoord: info.texCoord ?? 0 };
        if (info.transform) {
            output.extensions = { KHR_texture_transform: {
                offset: [...(info.transform.offset || [0, 0])],
                scale: [...(info.transform.scale || [1, 1])],
                rotation: info.transform.rotation ?? 0,
                ...(info.transform.texCoord === undefined ? {} : { texCoord: info.transform.texCoord }),
            } };
        }
        return output;
    };
    const baseColorTexture = texture(material.baseColorTexture, 'base color');
    const metallicRoughnessTexture = texture(material.metallicRoughnessTexture, 'metallic-roughness');
    const output: GltfMaterial = {
        name: material.name || `material_${index}`,
        pbrMetallicRoughness: {
            baseColorFactor: [...(material.baseColorFactor || [1, 1, 1, 1])],
            metallicFactor: material.metallicFactor ?? 1,
            roughnessFactor: material.roughnessFactor ?? 1,
            ...(baseColorTexture ? { baseColorTexture } : {}),
            ...(metallicRoughnessTexture ? { metallicRoughnessTexture } : {}),
        },
        emissiveFactor: [...(material.emissiveFactor || [0, 0, 0])],
        alphaMode: material.alphaMode || 'OPAQUE',
        doubleSided: Boolean(material.doubleSided),
    };
    const normalTexture = texture(material.normalTexture, 'normal');
    const occlusionTexture = texture(material.occlusionTexture, 'occlusion');
    const emissiveTexture = texture(material.emissiveTexture, 'emissive');
    if (normalTexture) output.normalTexture = normalTexture;
    if (occlusionTexture) output.occlusionTexture = occlusionTexture;
    if (emissiveTexture) output.emissiveTexture = emissiveTexture;
    if (normalTexture && material.normalTexture?.scale !== undefined) normalTexture.scale = material.normalTexture.scale;
    if (occlusionTexture && material.occlusionTexture?.strength !== undefined) occlusionTexture.strength = material.occlusionTexture.strength;
    if (material.unlit) output.extensions = { KHR_materials_unlit: {} };
    if (output.alphaMode === 'MASK') output.alphaCutoff = material.alphaCutoff ?? 0.5;
    return output;
}

function textureIndex(textures: (GltfTexture | null)[], source: number) {
    let index = 0;
    for (let current = 0; current < source; current += 1) if (textures[current]) index += 1;
    return index;
}

function lowerMesh(
    mesh: SceneMesh,
    index: number,
    accessorCount: number,
    materialCount: number,
    warnings: string[],
) {
    return {
        name: mesh.name || `mesh_${index}`,
        ...(mesh.weights?.length ? { weights: [...mesh.weights] } : {}),
        primitives: mesh.primitives.map((primitive, primitiveIndex) => {
            for (const accessor of Object.values(primitive.attributes)) assertAccessor(accessor, accessorCount, `mesh ${index} primitive ${primitiveIndex} attribute`);
            if (primitive.indices !== undefined) assertAccessor(primitive.indices, accessorCount, `mesh ${index} primitive ${primitiveIndex} indices`);
            if (primitive.material !== undefined && (primitive.material < 0 || primitive.material >= materialCount)) throw new Error(`SceneDocument mesh ${index} primitive ${primitiveIndex} has invalid material`);
            const targets = (primitive.targets || []).map((target, targetIndex) => {
                const output: Record<string, number> = {};
                for (const semantic of ['POSITION', 'NORMAL', 'TANGENT']) {
                    if (target[semantic] !== undefined) {
                        assertAccessor(target[semantic], accessorCount, `mesh ${index} primitive ${primitiveIndex} target ${targetIndex}`);
                        output[semantic] = target[semantic];
                    }
                }
                for (const semantic of Object.keys(target)) if (!(semantic in output)) warnings.push(`SceneDocument mesh ${index} primitive ${primitiveIndex} morph ${semantic} was omitted outside glTF core target semantics`);
                if (Object.keys(output).length === 0) throw new Error(`SceneDocument mesh ${index} primitive ${primitiveIndex} has an empty glTF morph target`);
                return output;
            });
            return {
                attributes: { ...primitive.attributes },
                ...(primitive.indices === undefined ? {} : { indices: primitive.indices }),
                ...(primitive.material === undefined ? {} : { material: primitive.material }),
                mode: primitive.mode ?? 4,
                ...(targets.length ? { targets } : {}),
            };
        }),
    };
}

function lowerNode(
    node: SceneNode,
    index: number,
    animatedNodes: Set<number>,
    meshCount: number,
    skinCount: number,
    warnings: string[],
) {
    const output: GltfNode = {
        name: node.name || `node_${index}`,
        ...(node.children?.length ? { children: [...node.children] } : {}),
        ...(node.mesh === undefined ? {} : { mesh: node.mesh }),
        ...(node.skin === undefined ? {} : { skin: node.skin }),
        ...(node.weights?.length ? { weights: [...node.weights] } : {}),
    };
    if (node.mesh !== undefined && (node.mesh < 0 || node.mesh >= meshCount)) throw new Error(`SceneDocument node ${index} has invalid mesh`);
    if (node.skin !== undefined && (node.skin < 0 || node.skin >= skinCount)) throw new Error(`SceneDocument node ${index} has invalid skin`);
    if (node.matrix && !animatedNodes.has(index)) output.matrix = [...node.matrix];
    else if (node.matrix) {
        Object.assign(output, decomposeMatrix(node.matrix));
        warnings.push(`SceneDocument node ${index} matrix was baked to local TRS for animated glTF export`);
    } else {
        output.translation = [...(node.translation || [0, 0, 0])];
        output.rotation = [...(node.rotation || [0, 0, 0, 1])];
        output.scale = [...(node.scale || [1, 1, 1])];
    }
    return output;
}

function lowerSkin(skin: SceneSkin, index: number, accessorCount: number) {
    if (skin.inverseBindMatrices !== undefined) assertAccessor(skin.inverseBindMatrices, accessorCount, `skin ${index} inverse bind matrices`);
    return {
        name: skin.name || `skin_${index}`,
        joints: [...skin.joints],
        ...(skin.inverseBindMatrices === undefined ? {} : { inverseBindMatrices: skin.inverseBindMatrices }),
        ...(skin.skeleton === undefined ? {} : { skeleton: skin.skeleton }),
    };
}

function lowerAnimation(clip: SceneAnimation, index: number, accessorCount: number, nodeCount: number) {
    const samplers = clip.samplers.map((sampler, samplerIndex) => {
        assertAccessor(sampler.input, accessorCount, `animation ${index} sampler ${samplerIndex} input`);
        assertAccessor(sampler.output, accessorCount, `animation ${index} sampler ${samplerIndex} output`);
        return { input: sampler.input, output: sampler.output, interpolation: sampler.interpolation || 'LINEAR' };
    });
    return {
        name: clip.name || `animation_${index}`,
        samplers,
        channels: clip.channels.map((channel, channelIndex) => {
            if (!Number.isInteger(channel.node) || channel.node < 0 || channel.node >= nodeCount) throw new Error(`SceneDocument animation ${index} channel ${channelIndex} has invalid node`);
            return { sampler: channel.sampler, target: { node: channel.node, path: channel.path } };
        }),
    };
}

function assertAccessor(index: number, count: number, label: string) {
    if (!Number.isInteger(index) || index < 0 || index >= count) throw new Error(`SceneDocument ${label} references an invalid accessor`);
}

function geometryAccessorTargets(meshes: SceneMesh[]) {
    const targets = new Map<number, number>();
    for (const mesh of meshes) for (const primitive of mesh.primitives) {
        for (const accessor of Object.values(primitive.attributes)) {
            if (!targets.has(accessor)) targets.set(accessor, 34962);
        }
        if (primitive.indices !== undefined && !targets.has(primitive.indices)) targets.set(primitive.indices, 34963);
    }
    return targets;
}

function accessorBounds(accessor: SceneAccessor) {
    const width = componentByteWidth(accessor.componentType);
    if (!width || !Number.isInteger(accessor.components) || accessor.components < 1 || accessor.components > 4) throw new Error('glTF bounds require a scalar or vector numeric accessor');
    const view = new DataView(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength);
    const min = Array.from({ length: accessor.components }, () => Infinity);
    const max = Array.from({ length: accessor.components }, () => -Infinity);
    for (let row = 0; row < accessor.count; row += 1) for (let component = 0; component < accessor.components; component += 1) {
        const value = readComponent(view, (row * accessor.components + component) * width, accessor.componentType);
        if (!Number.isFinite(value)) throw new Error('glTF bounded accessor contains a non-finite value');
        min[component] = Math.min(min[component], value);
        max[component] = Math.max(max[component], value);
    }
    return { min, max };
}

function componentByteWidth(componentType: number) {
    return new Map([[5120, 1], [5121, 1], [5122, 2], [5123, 2], [5125, 4], [5126, 4]]).get(componentType);
}

function readComponent(view: DataView, offset: number, componentType: number) {
    if (componentType === 5126) return view.getFloat32(offset, true);
    if (componentType === 5125) return view.getUint32(offset, true);
    if (componentType === 5123) return view.getUint16(offset, true);
    if (componentType === 5122) return view.getInt16(offset, true);
    if (componentType === 5121) return view.getUint8(offset);
    return view.getInt8(offset);
}

function materialUsesTextureTransform(material: GltfMaterial) {
    const pbr = material.pbrMetallicRoughness as {
        baseColorTexture?: GltfTextureInfo;
        metallicRoughnessTexture?: GltfTextureInfo;
    };
    const infos = [pbr.baseColorTexture, pbr.metallicRoughnessTexture, material.normalTexture, material.occlusionTexture, material.emissiveTexture];
    return infos.some((info) => Boolean(info?.extensions?.KHR_texture_transform));
}

function textureSourceExtension(mimeType: string | undefined) {
    if (mimeType === 'image/png' || mimeType === 'image/jpeg') return 'core';
    if (mimeType === 'image/webp') return 'EXT_texture_webp';
    if (mimeType === 'image/ktx2') return 'KHR_texture_basisu';
    return null;
}

function decomposeMatrix(matrix: number[]): Trs {
    const scale = [Math.hypot(matrix[0], matrix[1], matrix[2]) || 1, Math.hypot(matrix[4], matrix[5], matrix[6]) || 1, Math.hypot(matrix[8], matrix[9], matrix[10]) || 1];
    const m00 = matrix[0] / scale[0], m01 = matrix[4] / scale[1], m02 = matrix[8] / scale[2];
    const m10 = matrix[1] / scale[0], m11 = matrix[5] / scale[1], m12 = matrix[9] / scale[2];
    const m20 = matrix[2] / scale[0], m21 = matrix[6] / scale[1], m22 = matrix[10] / scale[2];
    const trace = m00 + m11 + m22;
    let rotation: number[];
    if (trace > 0) { const s = Math.sqrt(trace + 1) * 2; rotation = [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s]; }
    else if (m00 > m11 && m00 > m22) { const s = Math.sqrt(1 + m00 - m11 - m22) * 2; rotation = [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]; }
    else if (m11 > m22) { const s = Math.sqrt(1 + m11 - m00 - m22) * 2; rotation = [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s]; }
    else { const s = Math.sqrt(1 + m22 - m00 - m11) * 2; rotation = [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s]; }
    const length = Math.hypot(...rotation) || 1;
    return { translation: [matrix[12], matrix[13], matrix[14]], rotation: rotation.map((value) => value / length), scale };
}

class BinaryBuilder {
    #chunks: Uint8Array[] = [];
    length = 0;

    align() {
        const padding = (4 - (this.length % 4)) % 4;
        if (padding) this.write(new Uint8Array(padding));
    }

    write(bytes: Uint8Array) {
        const copy = new Uint8Array(bytes);
        this.#chunks.push(copy);
        this.length += copy.byteLength;
    }

    toBytes() {
        const output = new Uint8Array(this.length);
        let offset = 0;
        for (const chunk of this.#chunks) {
            output.set(chunk, offset);
            offset += chunk.byteLength;
        }
        return output;
    }
}
