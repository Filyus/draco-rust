/**
 * glTF/GLB -> SceneDocument import boundary.
 *
 * This adapter owns glTF accessor/resource interpretation and deliberately
 * returns only the portable byte-backed contract. It neither creates browser
 * images nor imports any FBX compatibility policy.
 */

import { assertValidSceneDocument, createSceneDocument } from './scene-document.ts';
import type {
    AnimationChannel, AnimationSampler, AttributeMap, ComponentType, SceneAccessor, SceneDocument,
    SceneNode, ScenePrimitive, TextureInfo,
} from './scene-document.ts';
import {
    appendAccessor, basename, mimeFromUri, resolveResource, sniffMime,
} from './scene-resources.ts';
import type { ResourceMap } from './scene-resources.ts';
import type { GltfAsset, GltfModule } from './wasm-modules.ts';

/**
 * Anything read out of the parsed glTF manifest is external JSON: it is
 * inspected field by field rather than trusted, so it stays loosely typed here
 * the same way SceneDocument validation treats its input.
 */
type GltfJson = any;

/**
 * The packed readers hand back a plain number. Any width outside the contract
 * is rejected by assertValidSceneDocument before the document is returned, so
 * this narrows a boundary value rather than assuming one.
 */
function componentType(value: number): ComponentType {
    return value as ComponentType;
}

const SUPPORTED_EXTENSIONS = new Set([
    'KHR_materials_unlit', 'KHR_texture_transform',
    'KHR_texture_basisu', 'EXT_texture_webp',
    // Draco is decompression, not a document feature: readPrimitive resolves it
    // into ordinary accessors, so the portable document loses nothing.
    'KHR_draco_mesh_compression',
]);

/** Extract a portable SceneDocument from an existing GltfAsset-capable module. */
export function buildSceneDocumentFromGltf(
    sourceData: Uint8Array,
    resources: Record<string, Uint8Array>,
    gltfModule: GltfModule,
): SceneDocument {
    const asset = gltfModule.GltfAsset.withResources(sourceData, resources, '2.1');
    try {
        const manifest: GltfJson = JSON.parse(new TextDecoder().decode(asset.json()));
        const document = createSceneDocument({ warnings: extensionWarnings(manifest) });
        const accessorBySource = new Map<number, number>();
        const imageResources = collectImageResources(asset, manifest.images || [], resources, document);
        const textureBySource = collectTextures(manifest.textures || [], manifest.samplers || [], imageResources, document);
        collectMaterials(manifest.materials || [], textureBySource, document);
        collectMeshes(asset, manifest.meshes || [], document, accessorBySource);
        collectNodes(manifest, document);
        collectSkins(asset, manifest.skins || [], document, accessorBySource);
        collectAnimations(asset, manifest.animations || [], document, accessorBySource);
        assertValidSceneDocument(document);
        return document;
    } finally {
        asset.free();
    }
}

function collectImageResources(
    asset: GltfAsset,
    images: GltfJson[],
    resources: ResourceMap,
    document: SceneDocument,
): number[] {
    return images.map((image, imageIndex) => {
        let bytes: Uint8Array | null = null;
        if (typeof image.bufferView === 'number') bytes = new Uint8Array(asset.bufferViewBytes(image.bufferView));
        else if (typeof image.uri === 'string') bytes = resolveResource(image.uri, resources);
        if (!bytes) {
            document.warnings.push(`glTF image ${imageIndex} could not be resolved and was omitted`);
            return -1;
        }
        const resourceIndex = document.resources.length;
        document.resources.push({
            name: image.name || basename(image.uri) || `image_${imageIndex}`,
            mimeType: image.mimeType || sniffMime(bytes) || mimeFromUri(image.uri) || 'application/octet-stream',
            bytes: new Uint8Array(bytes),
        });
        return resourceIndex;
    });
}

function collectTextures(
    textures: GltfJson[],
    samplers: GltfJson[],
    imageResources: number[],
    document: SceneDocument,
): number[] {
    return textures.map((texture, textureIndex) => {
        const source = textureSource(texture);
        const resource = imageResources[source];
        if (!Number.isInteger(resource) || resource < 0) {
            document.warnings.push(`glTF texture ${textureIndex} has no supported image source and was omitted`);
            return -1;
        }
        const sourceSampler = Number.isInteger(texture.sampler) ? samplers[texture.sampler] : null;
        const textureIndexInDocument = document.textures.length;
        document.textures.push({
            name: texture.name || `texture_${textureIndex}`,
            resource,
            sampler: {
                wrapS: sourceSampler?.wrapS ?? 10497,
                wrapT: sourceSampler?.wrapT ?? 10497,
                minFilter: sourceSampler?.minFilter ?? 9987,
                magFilter: sourceSampler?.magFilter ?? 9729,
            },
        });
        return textureIndexInDocument;
    });
}

function collectMaterials(materials: GltfJson[], textureBySource: number[], document: SceneDocument) {
    document.materials.push(...materials.map((material, materialIndex) => {
        const pbr = material.pbrMetallicRoughness || {};
        const baseColor = textureInfo(pbr.baseColorTexture, textureBySource);
        const metallicRoughness = textureInfo(pbr.metallicRoughnessTexture, textureBySource);
        const normal = textureInfo(material.normalTexture, textureBySource);
        const emissive = textureInfo(material.emissiveTexture, textureBySource);
        const occlusion = textureInfo(material.occlusionTexture, textureBySource);
        return {
            name: material.name || `material_${materialIndex}`,
            baseColorFactor: Array.from<number>(pbr.baseColorFactor || [1, 1, 1, 1]),
            metallicFactor: pbr.metallicFactor ?? 1,
            roughnessFactor: pbr.roughnessFactor ?? 1,
            emissiveFactor: Array.from<number>(material.emissiveFactor || [0, 0, 0]),
            ...(baseColor ? { baseColorTexture: baseColor } : {}),
            ...(metallicRoughness ? { metallicRoughnessTexture: metallicRoughness } : {}),
            ...(normal ? { normalTexture: normal } : {}),
            ...(emissive ? { emissiveTexture: emissive } : {}),
            ...(occlusion ? { occlusionTexture: occlusion } : {}),
            doubleSided: Boolean(material.doubleSided),
            alphaMode: material.alphaMode || 'OPAQUE',
            alphaCutoff: material.alphaCutoff ?? 0.5,
            unlit: Boolean(material.extensions?.KHR_materials_unlit),
        };
    }));
}

function collectMeshes(
    asset: GltfAsset,
    meshes: GltfJson[],
    document: SceneDocument,
    accessorBySource: Map<number, number>,
) {
    document.meshes.push(...meshes.map((mesh, meshIndex) => ({
        name: mesh.name || `mesh_${meshIndex}`,
        weights: Array.from<number>(mesh.weights || []),
        primitives: (mesh.primitives || []).map((primitive: GltfJson, primitiveIndex: number) => {
            const packed = asset.readPrimitive(meshIndex, primitiveIndex);
            try {
                const attributes: AttributeMap = {};
                for (let index = 0; index < packed.attributeCount(); index += 1) {
                    attributes[packed.attributeSemantic(index)] = appendAccessor(document, {
                        bytes: new Uint8Array(packed.attributeBytes(index)),
                        componentType: componentType(packed.attributeComponentType(index)),
                        components: packed.attributeComponents(index),
                        count: packed.attributeElementCount(index),
                        normalized: packed.attributeNormalized(index),
                    });
                }
                const targets = (primitive.targets || []).map((target: GltfJson) => {
                    const converted: AttributeMap = {};
                    for (const semantic of ['POSITION', 'NORMAL', 'TANGENT']) {
                        if (typeof target[semantic] === 'number') converted[semantic] = sourceAccessor(asset, target[semantic], document, accessorBySource);
                    }
                    return converted;
                });
                const result: ScenePrimitive = {
                    attributes,
                    mode: packed.mode(),
                    ...(typeof primitive.material === 'number' ? { material: primitive.material } : {}),
                    ...(targets.length > 0 ? { targets } : {}),
                };
                if (packed.hasIndices()) {
                    result.indices = appendAccessor(document, {
                        bytes: new Uint8Array(packed.indexBytes()),
                        componentType: componentType(packed.indexComponentType()),
                        components: 1,
                        count: packed.indexCount(),
                        normalized: false,
                    });
                }
                return result;
            } finally {
                packed.free();
            }
        }),
    })));
}

function collectNodes(manifest: GltfJson, document: SceneDocument) {
    const meshes: GltfJson[] = manifest.meshes || [];
    document.nodes.push(...(manifest.nodes || []).map((node: GltfJson, nodeIndex: number) => {
        const output: SceneNode = {
            name: node.name || `node_${nodeIndex}`,
            children: Array.from<number>(node.children || []),
            ...(typeof node.mesh === 'number' ? { mesh: node.mesh } : {}),
            ...(typeof node.skin === 'number' ? { skin: node.skin } : {}),
        };
        if (Array.isArray(node.matrix) && node.matrix.length === 16) output.matrix = Array.from(node.matrix);
        else {
            output.translation = Array.from(node.translation || [0, 0, 0]);
            output.rotation = Array.from(node.rotation || [0, 0, 0, 1]);
            output.scale = Array.from(node.scale || [1, 1, 1]);
        }
        const targetCount = typeof node.mesh === 'number'
            ? Math.max(0, ...(meshes[node.mesh]?.primitives || []).map((primitive: GltfJson) => primitive.targets?.length || 0))
            : 0;
        if (targetCount > 0) {
            const source = node.weights || meshes[node.mesh]?.weights || [];
            output.weights = Array.from({ length: targetCount }, (_, index) => Number(source[index]) || 0);
        }
        return output;
    }));
    const sceneIndex = typeof manifest.scene === 'number' ? manifest.scene : 0;
    const configured = manifest.scenes?.[sceneIndex]?.nodes || manifest.scenes?.[0]?.nodes;
    document.rootNodes.push(...(configured || rootNodes(document.nodes)));
}

function collectSkins(
    asset: GltfAsset,
    skins: GltfJson[],
    document: SceneDocument,
    accessorBySource: Map<number, number>,
) {
    document.skins.push(...skins.map((skin, skinIndex) => ({
        name: skin.name || `skin_${skinIndex}`,
        joints: Array.from<number>(skin.joints || []),
        ...(typeof skin.inverseBindMatrices === 'number'
            ? { inverseBindMatrices: sourceAccessor(asset, skin.inverseBindMatrices, document, accessorBySource) }
            : {}),
        ...(typeof skin.skeleton === 'number' ? { skeleton: skin.skeleton } : {}),
    })));
}

function collectAnimations(
    asset: GltfAsset,
    animations: GltfJson[],
    document: SceneDocument,
    accessorBySource: Map<number, number>,
) {
    document.animations.push(...animations.map((animation, animationIndex) => {
        const samplers: AnimationSampler[] = [];
        const samplerBySource = new Map<number, number>();
        const channels: AnimationChannel[] = [];
        for (const channel of animation.channels || []) {
            const target = channel.target || {};
            if (!['translation', 'rotation', 'scale', 'weights'].includes(target.path) || !Number.isInteger(target.node)) {
                document.warnings.push(`glTF animation ${animation.name || animationIndex} has an unsupported channel and it was omitted`);
                continue;
            }
            const source = animation.samplers?.[channel.sampler];
            if (!source || !Number.isInteger(source.input) || !Number.isInteger(source.output)) {
                document.warnings.push(`glTF animation ${animation.name || animationIndex} has an invalid sampler and it was omitted`);
                continue;
            }
            let samplerIndex = samplerBySource.get(channel.sampler);
            if (samplerIndex === undefined) {
                const input = sourceAccessor(asset, source.input, document, accessorBySource);
                let output = sourceAccessor(asset, source.output, document, accessorBySource);
                if (target.path === 'weights') {
                    const targetCount = document.nodes[target.node]?.weights?.length || 0;
                    const sourceOutput = document.accessors[output];
                    if (targetCount === 0 || sourceOutput.count % targetCount !== 0) {
                        document.warnings.push(`glTF animation ${animation.name || animationIndex} has an unrepresentable weight sampler and it was omitted`);
                        continue;
                    }
                    output = appendAccessor(document, {
                        ...sourceOutput,
                        bytes: new Uint8Array(sourceOutput.bytes),
                        components: targetCount,
                        count: sourceOutput.count / targetCount,
                    });
                }
                samplerIndex = samplers.length;
                samplers.push({ input, output, interpolation: source.interpolation || 'LINEAR' });
                samplerBySource.set(channel.sampler, samplerIndex);
            }
            channels.push({ sampler: samplerIndex, node: target.node, path: target.path });
        }
        const duration = Math.max(0, ...samplers.map((sampler) => lastTime(document.accessors[sampler.input])));
        return { name: animation.name || `animation_${animationIndex}`, duration, samplers, channels };
    }).filter((animation) => animation.channels.length > 0));
}

function sourceAccessor(
    asset: GltfAsset,
    sourceIndex: number,
    document: SceneDocument,
    cache: Map<number, number>,
): number {
    const cached = cache.get(sourceIndex);
    if (cached !== undefined) return cached;
    const packed = asset.readAccessor(sourceIndex);
    try {
        const index = appendAccessor(document, {
            bytes: new Uint8Array(packed.bytes()),
            componentType: componentType(packed.componentType()),
            components: packed.components(),
            count: packed.count(),
            normalized: packed.normalized(),
        });
        cache.set(sourceIndex, index);
        return index;
    } finally {
        packed.free();
    }
}

function textureInfo(info: GltfJson, textureBySource: number[]): TextureInfo | null {
    if (!info || !Number.isInteger(info.index)) return null;
    const texture = textureBySource[info.index];
    if (!Number.isInteger(texture) || texture < 0) return null;
    const result: TextureInfo = { texture, texCoord: info.texCoord ?? 0 };
    const transform = info.extensions?.KHR_texture_transform;
    if (transform) {
        result.texCoord = transform.texCoord ?? result.texCoord;
        result.transform = {
            offset: Array.from(transform.offset || [0, 0]),
            scale: Array.from(transform.scale || [1, 1]),
            rotation: transform.rotation ?? 0,
            ...(transform.texCoord === undefined ? {} : { texCoord: transform.texCoord }),
        };
    }
    if (info.scale !== undefined) result.scale = info.scale;
    if (info.strength !== undefined) result.strength = info.strength;
    return result;
}

function textureSource(texture: GltfJson): number {
    if (Number.isInteger(texture.source)) return texture.source;
    for (const extension of ['KHR_texture_basisu', 'EXT_texture_webp']) {
        const source = texture.extensions?.[extension]?.source;
        if (Number.isInteger(source)) return source;
    }
    return -1;
}

function lastTime(accessor: SceneAccessor | undefined): number {
    if (!accessor || accessor.componentType !== 5126 || accessor.components !== 1 || accessor.count === 0) return 0;
    const view = new DataView(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength);
    return view.getFloat32((accessor.count - 1) * 4, true);
}

function extensionWarnings(manifest: GltfJson): string[] {
    const warnings: string[] = [];
    const unsupported = (manifest.extensionsUsed || []).filter((extension: string) => !SUPPORTED_EXTENSIONS.has(extension));
    if (unsupported.length > 0) warnings.push(`Unsupported glTF extensions omitted from SceneDocument: ${unsupported.join(', ')}`);
    if ((manifest.extensionsRequired || []).some((extension: string) => !SUPPORTED_EXTENSIONS.has(extension))) warnings.push('glTF requires extensions outside the portable SceneDocument subset');
    return warnings;
}

function rootNodes(nodes: SceneNode[]): number[] {
    const children = new Set(nodes.flatMap((node) => node.children || []));
    return nodes.map((_, index) => index).filter((index) => !children.has(index));
}

