/**
 * SceneDocument -> viewer runtime adapter.
 *
 * This is intentionally format-neutral. Image decoding and GPU upload remain
 * the viewer's concern; textures retain their source byte resource here so a
 * browser-specific hydration step can be added without leaking DOM handles
 * into SceneDocument.
 */

import { componentByteSize, readComponent } from './component-values.ts';
import { cloneTrs, decomposeMat4 } from './mat4.ts';
import type { RuntimeAccessor } from './viewer-scene.ts';
import { assertValidSceneDocument } from './scene-document.ts';
import type {
    SceneAccessor,
    SceneAnimation,
    SceneDocument,
    SceneMaterial,
    SceneMesh,
    SceneNode,
    ScenePrimitive,
    SceneResource,
    SceneSkin,
    SceneTexture,
    TextureInfo,
} from './scene-document.ts';

const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];

/**
 * Build the current viewer's runtime Scene from a validated SceneDocument.
 *
 * The runtime shape stays inferred: the glTF loader builds a slightly
 * different node (it carries an `index`, not a rest/animation TRS pair), so
 * the one type both must satisfy belongs with the viewer that consumes them.
 */
export function buildViewerSceneFromDocument(document: SceneDocument) {
    const validation = assertValidSceneDocument(document);
    const accessors = document.accessors.map(toRuntimeAccessor);
    const meshes = document.meshes.map((mesh, meshIndex) => ({
        name: mesh.name || `mesh_${meshIndex}`,
        primitives: mesh.primitives.map((primitive) => adaptPrimitive(primitive, accessors)),
        aabb: meshAabb(mesh, accessors),
    }));
    const nodes = document.nodes.map((node) => adaptNode(node, document));
    const materials = document.materials.map(adaptMaterial);
    const textures = document.textures.map((texture, textureIndex) => adaptTexture(texture, document.resources, textureIndex));
    const skins = document.skins.map((skin, skinIndex) => adaptSkin(skin, nodes, accessors, skinIndex));

    nodes.forEach((node, index) => {
        const source = document.nodes[index];
        if (source.skin !== undefined) node.skinIndex = source.skin;
    });
    const renderables: { node: typeof nodes[number]; meshIndex: number; skinIndex: number }[] = [];
    nodes.forEach((node) => {
        if (node.meshIndex >= 0) renderables.push({ node, meshIndex: node.meshIndex, skinIndex: node.skinIndex });
    });

    const animations = document.animations.map((clip, clipIndex) => adaptAnimation(clip, nodes, accessors, clipIndex));
    const viewerWarnings = [...document.warnings, ...validation.warnings];
    for (const mesh of document.meshes) for (const primitive of mesh.primitives) {
        if (primitive.attributes.JOINTS_1 !== undefined || primitive.attributes.WEIGHTS_1 !== undefined) {
            viewerWarnings.push('Preview skinning uses the first four influences; additional influence sets remain available to exporters.');
            break;
        }
    }
    return {
        nodes,
        rootIndices: [...document.rootNodes],
        meshes,
        skins,
        materials,
        textures,
        animations,
        renderables,
        aabb: sceneAabb(meshes),
        warnings: viewerWarnings,
    };
}

function toRuntimeAccessor(accessor: SceneAccessor): RuntimeAccessor {
    return {
        bytes: new Uint8Array(accessor.bytes),
        componentType: accessor.componentType,
        components: accessor.components,
        normalized: Boolean(accessor.normalized),
        count: accessor.count,
    };
}

function adaptPrimitive(primitive: ScenePrimitive, accessors: RuntimeAccessor[]) {
    const attributes: Record<string, RuntimeAccessor> = {};
    for (const [semantic, accessorIndex] of Object.entries(primitive.attributes)) {
        attributes[semantic] = accessors[accessorIndex];
    }
    const runtime: {
        attributes: Record<string, RuntimeAccessor>;
        mode: number;
        materialIndex: number;
        indices?: RuntimeAccessor;
        morphPositions?: (RuntimeAccessor | null)[];
        morphNormals?: (RuntimeAccessor | null)[];
    } = {
        attributes,
        mode: primitive.mode ?? 4,
        materialIndex: primitive.material ?? 0,
    };
    if (primitive.indices !== undefined) runtime.indices = accessors[primitive.indices];
    if (primitive.targets?.length) {
        runtime.morphPositions = primitive.targets.map((target) => target.POSITION === undefined ? null : accessors[target.POSITION]);
        runtime.morphNormals = primitive.targets.map((target) => target.NORMAL === undefined ? null : accessors[target.NORMAL]);
    }
    return runtime;
}

function adaptNode(node: SceneNode, document: SceneDocument) {
    const mesh = node.mesh === undefined ? null : document.meshes[node.mesh];
    const weights = node.weights || mesh?.weights || [];
    const trs = node.matrix ? decomposeMat4(node.matrix) : {
        translation: [...(node.translation || [0, 0, 0])],
        rotation: [...(node.rotation || [0, 0, 0, 1])],
        scale: [...(node.scale || [1, 1, 1])],
    };
    return {
        name: node.name || 'node',
        trs: cloneTrs(trs),
        restTrs: cloneTrs(trs),
        animationTrs: cloneTrs(trs),
        localMatrix: node.matrix ? Float32Array.from(node.matrix) : null,
        children: [...(node.children || [])],
        weights: Float32Array.from(weights),
        meshIndex: node.mesh ?? -1,
        skinIndex: -1,
        world: new Float32Array(IDENTITY),
    };
}

function adaptSkin(
    skin: SceneSkin,
    nodes: ReturnType<typeof adaptNode>[],
    accessors: RuntimeAccessor[],
    skinIndex: number,
) {
    const inverseBinds = skin.inverseBindMatrices === undefined
        ? null : matrixAccessor(accessors[skin.inverseBindMatrices], skin.joints.length);
    return {
        name: skin.name || `skin_${skinIndex}`,
        joints: skin.joints.map((jointIndex, index) => ({
            node: nodes[jointIndex],
            inverseBind: inverseBinds ? inverseBinds[index] : Float32Array.from(IDENTITY),
        })),
    };
}

function adaptAnimation(
    clip: SceneAnimation,
    nodes: ReturnType<typeof adaptNode>[],
    accessors: RuntimeAccessor[],
    clipIndex: number,
) {
    return {
        name: clip.name || `animation_${clipIndex}`,
        duration: clip.duration,
        channels: clip.channels.map((channel) => {
            const sampler = clip.samplers[channel.sampler];
            const input = floatAccessor(accessors[sampler.input]);
            const output = floatAccessor(accessors[sampler.output]);
            return {
                node: nodes[channel.node],
                path: channel.path,
                targetCount: channel.path === 'weights' ? accessors[sampler.output].components : 3,
                sampler: { input, output, interpolation: sampler.interpolation || 'LINEAR' },
            };
        }),
    };
}

function adaptMaterial(material: SceneMaterial, index: number) {
    return {
        name: material.name || `material_${index}`,
        baseColorFactor: [...(material.baseColorFactor || [1, 1, 1, 1])],
        baseColorTexture: textureIndex(material.baseColorTexture),
        baseColorTexCoord: material.baseColorTexture?.texCoord || 0,
        baseColorTextureTransform: material.baseColorTexture?.transform || { offset: [0, 0], scale: [1, 1], rotation: 0 },
        metallic: material.metallicFactor ?? 1,
        roughness: material.roughnessFactor ?? 1,
        metallicRoughnessTexture: textureInfo(material.metallicRoughnessTexture),
        emissiveFactor: [...(material.emissiveFactor || [0, 0, 0])],
        emissiveTexture: textureInfo(material.emissiveTexture),
        normalTexture: textureInfo(material.normalTexture),
        occlusionTexture: textureInfo(material.occlusionTexture),
        doubleSided: Boolean(material.doubleSided),
        alphaMode: material.alphaMode || 'OPAQUE',
        alphaCutoff: material.alphaCutoff ?? 0.5,
        unlit: Boolean(material.unlit),
    };
}

function adaptTexture(texture: SceneTexture, resources: SceneResource[], index: number) {
    const resource = resources[texture.resource];
    return {
        name: texture.name || resource.name || `texture_${index}`,
        resource: texture.resource,
        mimeType: resource.mimeType,
        bytes: new Uint8Array(resource.bytes),
        ...texture.sampler,
    };
}

function textureIndex(info: TextureInfo | undefined) {
    return info ? info.texture : null;
}

function textureInfo(info: TextureInfo | undefined) {
    return info ? {
        index: info.texture,
        texCoord: info.texCoord || 0,
        ...(info.transform ? { transform: structuredClone(info.transform) } : {}),
        ...(info.scale === undefined ? {} : { scale: info.scale }),
        ...(info.strength === undefined ? {} : { strength: info.strength }),
    } : null;
}

function floatAccessor(accessor: RuntimeAccessor): Float32Array {
    const view = new DataView(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength);
    const values = new Float32Array(accessor.count * accessor.components);
    for (let index = 0; index < values.length; index += 1) values[index] = view.getFloat32(index * 4, true);
    return values;
}

function matrixAccessor(accessor: RuntimeAccessor, count: number): Float32Array[] {
    const values = floatAccessor(accessor);
    return Array.from({ length: count }, (_, index) => Float32Array.from(values.subarray(index * 16, index * 16 + 16)));
}

function meshAabb(mesh: SceneMesh, accessors: RuntimeAccessor[]) {
    const aabb = { min: [Infinity, Infinity, Infinity], max: [-Infinity, -Infinity, -Infinity] };
    for (const primitive of mesh.primitives) {
        const accessor = accessors[primitive.attributes.POSITION];
        if (!accessor || accessor.components !== 3) continue;
        const values = readAccessor(accessor);
        for (let index = 0; index < values.length; index += 3) {
            for (let component = 0; component < 3; component += 1) {
                aabb.min[component] = Math.min(aabb.min[component], values[index + component]);
                aabb.max[component] = Math.max(aabb.max[component], values[index + component]);
            }
        }
    }
    return Number.isFinite(aabb.min[0]) ? aabb : { min: [-0.5, -0.5, -0.5], max: [0.5, 0.5, 0.5] };
}

function sceneAabb(meshes: { aabb: { min: number[]; max: number[] } }[]) {
    const aabb = { min: [Infinity, Infinity, Infinity], max: [-Infinity, -Infinity, -Infinity] };
    for (const mesh of meshes) for (let component = 0; component < 3; component += 1) {
        aabb.min[component] = Math.min(aabb.min[component], mesh.aabb.min[component]);
        aabb.max[component] = Math.max(aabb.max[component], mesh.aabb.max[component]);
    }
    return Number.isFinite(aabb.min[0]) ? aabb : { min: [-0.5, -0.5, -0.5], max: [0.5, 0.5, 0.5] };
}

function readAccessor(accessor: RuntimeAccessor): number[] {
    const bytes = componentByteSize(accessor.componentType);
    const view = new DataView(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength);
    const values = new Array(accessor.count * accessor.components);
    for (let index = 0; index < values.length; index += 1) {
        values[index] = readComponent(view, index * bytes, accessor.componentType);
    }
    return values;
}
