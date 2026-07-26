/**
 * glTF / GLB → Scene loader for the WebGL2 viewer.
 *
 * Uses the gltf-wasm API:
 *   - GltfAsset.withResources(bytes, resources, profile)
 *   - asset.readPrimitive(mesh, primitive) -> PackedGeometry (decoded, incl. Draco)
 *   - asset.readAccessor(index)            -> PackedAccessor (sparse + stride-resolved)
 *   - asset.bufferViewBytes(index)         -> Uint8Array (raw layout, for embedded images)
 *   - asset.json()                          -> lossless JSON document bytes
 *
 * Rust owns container/resource resolution and binary materialization. This
 * renderer adapter interprets scene, material, animation, and extension JSON
 * because support policy and fallback behavior are specific to the preview.
 */

import { componentByteSize, normalizeComponent, readComponent } from './component-values.ts';
import { mimeFromUri, resolveResource, sniffMime } from './scene-resources.ts';
import type { ResourceMap } from './scene-resources.ts';
import {
    buildFbxMeshSkin,
    buildFbxMaterials,
    buildFbxMorphTargets,
    buildFbxSkins,
    buildFbxTextures,
    buildFbxWorldMatrices,
    convertGltfVectorArrayToFbx,
    fbxRowMajorMatrix,
    extractGltfCubicSegment,
    quaternionKeysToFbxEuler,
} from './fbx-scene-adapter.ts';
import type { GltfAsset, GltfModule, PackedAccessor, PackedGeometry } from './wasm-modules.ts';
import type {
    Aabb, Renderable, RuntimeAccessor, ViewerClip, ViewerMesh, ViewerNode, ViewerSkin,
} from './viewer-scene.ts';

/**
 * The parsed glTF manifest and the FBX structures built from it: external
 * JSON on one side, the writer's own shapes on the other, both inspected
 * field by field rather than trusted.
 */
type GltfJson = any;

/** Diagnostics sink shared with the rest of the import path. */
interface ImportHooks {
    onLog?: (message: string, level: string) => void;
}

const GL = WebGL2RenderingContext;
// Extensions this module interprets on its own.
const SUPPORTED_EXTENSIONS = new Set([
    'KHR_materials_unlit',
    'KHR_texture_transform',
    // Consumed by the wasm reader: readPrimitive materializes Draco geometry,
    // and fails the load outright when it cannot. Nothing reaches here to ignore.
    'KHR_draco_mesh_compression',
]);
// Extensions whose only effect is naming an alternate image source, in the
// order the preview prefers them. Whether one of these is honored depends on a
// codec this module does not own, so it is reported from the decode result
// rather than asserted here.
const TEXTURE_SOURCE_EXTENSIONS = ['EXT_texture_webp', 'KHR_texture_basisu'];
// Morph targets the preview can blend in one frame. Mirrors the viewer's shader
// loop bound; a mesh may declare any number of targets as long as no single
// frame drives more than this many at once.
const MAX_ACTIVE_MORPH_TARGETS = 32;

/**
 * Build a Scene from a parsed glTF document.
 *
 * @param {Uint8Array} sourceData     The .gltf or .glb file bytes.
 * @param {Object} resources          Map of companion filename -> Uint8Array.
 * @param {Object} gltfModule         Imported gltf.js module.
 * @param {Object} hooks              { onLog(msg, type), loadImage(bytes, mime) }
 */
export async function buildSceneFromGltf(
    sourceData: Uint8Array,
    resources: ResourceMap,
    gltfModule: GltfModule,
    hooks: ImportHooks = {},
) {
    const log = (msg: string, type = 'info') => hooks.onLog?.(msg, type);

    const asset = gltfModule.GltfAsset.withResources(sourceData, resources, '2.1');
    try {
        const document: GltfJson = JSON.parse(new TextDecoder().decode(asset.json()));
        const warnings: string[] = [];
        const nodes = buildNodes(document.nodes || []);
        const meshes = buildMeshes(asset, document.meshes || [], warnings);
        initializeMorphWeights(nodes, meshes, warnings);
        const skins = buildSkins(asset, document.skins || [], nodes, warnings);
        const materials = buildMaterials(document.materials || []);
        const { textures, honoredSources } = await buildTextures(asset, document, resources, hooks);
        warnings.push(...extensionWarnings(document, honoredSources));
        const animations = buildAnimations(asset, document.animations || [], nodes, warnings);
        const scenes = document.scenes || [];
        const sceneIndex = typeof document.scene === 'number' ? document.scene : 0;
        const rootIndices = scenes[sceneIndex]?.nodes || scenes[0]?.nodes || [];

        const { renderables, aabb } = computeRenderables(
            nodes,
            meshes,
            skins,
            rootIndices,
        );

        if (animations.length > 0) {
            log(`Loaded ${animations.length} animation clip${animations.length === 1 ? '' : 's'}`, 'info');
        }

        return {
            nodes,
            rootIndices,
            meshes,
            skins,
            materials,
            textures,
            animations,
            renderables,
            aabb,
            warnings,
        };
    } finally {
        asset.free();
    }
}

/**
 * Decode a glTF document into the flat triangle meshes consumed by the
 * format-writer WASM modules. Unlike the preview scene this intentionally
 * drops materials, animation, skins, and node transforms: the OBJ/PLY/FBX
 * writers currently accept geometry buffers only.
 */
export function buildFlatMeshesFromGltf(
    sourceData: Uint8Array,
    resources: ResourceMap,
    gltfModule: GltfModule,
): GltfJson[] {
    const asset = gltfModule.GltfAsset.withResources(sourceData, resources, '2.1');
    try {
        const document = JSON.parse(new TextDecoder().decode(asset.json()));
        const definitions = document.meshes || [];
        const meshes = [];

        for (let meshIndex = 0; meshIndex < asset.meshCount(); meshIndex += 1) {
            const primitiveCount = asset.primitiveCount(meshIndex);
            for (let primitiveIndex = 0; primitiveIndex < primitiveCount; primitiveIndex += 1) {
                const packed = asset.readPrimitive(meshIndex, primitiveIndex);
                try {
                    const attributes = new Map();
                    for (let index = 0; index < packed.attributeCount(); index += 1) {
                        attributes.set(packed.attributeSemantic(index), {
                            bytes: new Uint8Array(packed.attributeBytes(index)),
                            componentType: packed.attributeComponentType(index),
                            components: packed.attributeComponents(index),
                            normalized: packed.attributeNormalized(index),
                            count: packed.attributeElementCount(index),
                        });
                    }

                    const position = attributes.get('POSITION');
                    if (!position || position.components !== 3 || position.count === 0) continue;
                    const sourceIndices = packed.hasIndices()
                        ? packedIndices(packed)
                        : Array.from({ length: position.count }, (_, index) => index);
                    const indices = triangleIndices(packed.mode(), sourceIndices);
                    if (indices.length === 0) continue;

                    const normal = attributes.get('NORMAL');
                    const texcoord = attributes.get('TEXCOORD_0');
                    const joints0 = attributes.get('JOINTS_0');
                    const weights0 = attributes.get('WEIGHTS_0');
                    const joints1 = attributes.get('JOINTS_1');
                    const weights1 = attributes.get('WEIGHTS_1');
                    const meshName = definitions[meshIndex]?.name || `mesh_${meshIndex}`;
                    meshes.push({
                        name: primitiveCount === 1 ? meshName : `${meshName}_${primitiveIndex}`,
                        positions: packedAttributeNumbers(position),
                        indices,
                        normals: normal?.components === 3 && normal.count === position.count
                            ? packedAttributeNumbers(normal)
                            : null,
                        uvs: texcoord?.components === 2 && texcoord.count === position.count
                            ? packedAttributeNumbers(texcoord)
                            : null,
                        joints0: joints0?.components === 4 && joints0.count === position.count
                            ? packedAttributeNumbers(joints0) : null,
                        weights0: weights0?.components === 4 && weights0.count === position.count
                            ? packedAttributeNumbers(weights0) : null,
                        joints1: joints1?.components === 4 && joints1.count === position.count
                            ? packedAttributeNumbers(joints1) : null,
                        weights1: weights1?.components === 4 && weights1.count === position.count
                            ? packedAttributeNumbers(weights1) : null,
                    });
                } finally {
                    packed.free();
                }
            }
        }
        return meshes;
    } finally {
        asset.free();
    }
}

/** Build the hierarchy and local transforms representable by FBX export. */
export function buildFbxSceneFromGltf(
    sourceData: Uint8Array,
    resources: ResourceMap,
    gltfModule: GltfModule,
    options: { legacyCompatibility?: boolean } = {},
) {
    const legacyCompatibility = options.legacyCompatibility === true;
    const asset = gltfModule.GltfAsset.withResources(sourceData, resources, '2.1');
    try {
        const document = JSON.parse(new TextDecoder().decode(asset.json()));
        const definitions = document.meshes || [];
        const flatMeshes = buildFlatMeshesFromGltf(sourceData, resources, gltfModule);
        const meshesByDefinition = definitions.map((definition: GltfJson, meshIndex: number) => {
            const primitives = definition.primitives || [];
            return flatMeshes.splice(0, primitives.length).map((mesh: GltfJson, primitiveIndex: number) => {
                const material = primitives[primitiveIndex]?.material;
                return {
                    ...mesh,
                    positions: convertGltfVectorArrayToFbx(mesh.positions),
                    normals: mesh.normals && convertGltfVectorArrayToFbx(mesh.normals),
                    // glTF texture coordinates are consumed with a top-left
                    // image origin in the web preview. FBX's UV convention as
                    // interpreted by Blender uses the opposite V direction.
                    // Convert only at the glTF -> FBX boundary; FBX re-export
                    // must preserve the coordinates it originally read.
                    uvs: mesh.uvs
                        ? mesh.uvs.map((value: number, component: number) => (component % 2 === 1 ? 1 - value : value))
                        : null,
                    materialIndices: typeof material === 'number'
                        ? Array(mesh.indices.length / 3).fill(material)
                        : [],
                    morphTargets: buildFbxMorphTargets(
                        asset,
                        primitives[primitiveIndex]?.targets || [],
                        definitions[meshIndex]?.weights || [],
                        readAccessorAsTyped,
                    ),
                };
            });
        });
        const nodes: GltfJson[] = document.nodes || [];
        const warnings: string[] = [];
        const sceneIndex = typeof document.scene === 'number' ? document.scene : 0;
        const roots = document.scenes?.[sceneIndex]?.nodes || document.scenes?.[0]?.nodes
            || nodes.map((_: GltfJson, index: number) => index).filter((index: number) => !nodes.some((node: GltfJson) => node.children?.includes(index)));
        const nodeRecords = buildNodes(nodes);
        // The FBX animation bridge needs a concrete morph-weight count even
        // when glTF omits the optional node.weights array.
        nodeRecords.forEach((node, index) => {
            const definition = nodes[index] || {};
            const meshDefinition = typeof definition.mesh === 'number'
                ? definitions[definition.mesh]
                : null;
            const targetCount = meshDefinition?.primitives?.reduce(
                (count: number, primitive: GltfJson) => Math.max(count, (primitive.targets || []).length),
                0,
            ) || 0;
            if (targetCount > 0 && node.weights!.length === 0) {
                node.weights = Float32Array.from(
                    Array.from({ length: targetCount }, (_, target) =>
                        Number(meshDefinition.weights?.[target]) || 0),
                );
            }
        });
        const worlds = buildFbxWorldMatrices(nodes, roots, composeTrs);
        const skins = buildFbxSkins(
            asset, document.skins || [], worlds, warnings, readAccessorAsTyped, composeTrs,
        );
        const buildNode = (index: number, isRoot = false): GltfJson => {
            const node = nodes[index] || {};
            return {
                id: index + 1,
                name: node.name || `node_${index}`,
                matrix: scaleFbxRootMatrix(
                    fbxRowMajorMatrix(node, composeTrs),
                ),
                meshes: typeof node.mesh === 'number'
                    ? (meshesByDefinition[node.mesh] || []).map((mesh: GltfJson) => ({
                        ...mesh,
                        skin: typeof node.skin === 'number'
                            ? buildFbxMeshSkin(
                                mesh, skins[node.skin], index + 1, worlds[index], composeTrs,
                            )
                            : null,
                        morphTargets: (mesh.morphTargets || []).map((target: GltfJson, targetIndex: number) => ({
                            ...target,
                            defaultWeight: Number(node.weights?.[targetIndex] ?? target.defaultWeight) * 100,
                        })),
                    }))
                    : [],
                children: (node.children || []).map((child: number) => buildNode(child, false)),
            };
        };
        return {
            rootNodes: roots.map((root: number) => buildNode(root, true)),
            materials: buildFbxMaterials(document.materials || []),
            textures: buildFbxTextures(asset, document, resources, resolveUriBytes),
            animations: buildFbxAnimations(
                asset, document.animations || [], nodeRecords, warnings, legacyCompatibility,
            ),
            warnings,
        };
    } finally {
        asset.free();
    }
}

/** Convert glTF TRS clips to the FBX animation contract used by fbx-wasm. */
function buildFbxAnimations(
    asset: GltfAsset,
    definitions: GltfJson[],
    nodes: GltfJson[],
    warnings: string[],
    legacyCompatibility = false,
): GltfJson[] {
    return buildAnimations(asset, definitions, nodes, warnings).map((animation) => {
        const channels = animation.channels.flatMap((channel: GltfJson): GltfJson[] => {
            if (channel.path === 'weights') {
                const targetCount = channel.targetCount || 0;
                const interpolation = String(channel.sampler.interpolation || 'LINEAR').toLowerCase();
                const cubic = interpolation === 'cubicspline';
                const result: GltfJson[] = [];
                for (let target = 0; target < targetCount; target++) {
                    const values = Array.from<number>(channel.sampler.output);
                    const output = [];
                    const inTangents = [];
                    const outTangents = [];
                    if (cubic) {
                        for (let frame = 0; frame < channel.sampler.input.length; frame++) {
                            const base = frame * targetCount * 3;
                            output.push((values[base + targetCount + target] || 0) * 100);
                            inTangents.push((values[base + target] || 0) * 100);
                            outTangents.push((values[base + targetCount * 2 + target] || 0) * 100);
                        }
                    } else {
                        for (let frame = 0; frame < channel.sampler.input.length; frame++) {
                            output.push((values[frame * targetCount + target] || 0) * 100);
                        }
                    }
                    result.push({
                        nodeName: channel.node.name,
                        nodeId: channel.node.index! + 1,
                        morphTargetIndex: target,
                        path: 'morphweight',
                        sampler: {
                            input: Array.from(channel.sampler.input),
                            output,
                            interpolation: legacyCompatibility
                                ? 'linear'
                                : (cubic ? 'cubic' : (interpolation === 'step' ? 'step' : 'linear')),
                            inTangents: !legacyCompatibility && cubic ? inTangents : null,
                            outTangents: !legacyCompatibility && cubic ? outTangents : null,
                        },
                    });
                }
                return result;
            }
            if (!['translation', 'rotation', 'scale'].includes(channel.path)) return [];
            const interpolation = String(channel.sampler.interpolation || 'LINEAR').toLowerCase();
            const input = Array.from<number>(channel.sampler.input);
            const values = Array.from<number>(channel.sampler.output);
            const cubic = interpolation === 'cubicspline';
            const keyValues = cubic
                ? extractGltfCubicSegment(values, channel.path === 'rotation' ? 4 : 3, 1)
                : values;
            const output = channel.path === 'rotation'
                ? quaternionKeysToFbxEuler(keyValues)
                : channel.path === 'translation'
                    ? convertGltfVectorArrayToFbx(keyValues)
                    : convertGltfScaleKeysToFbx(keyValues);
            if (output.length !== input.length * 3) {
                warnings.push(`Animation ${animation.name}: invalid ${channel.path} sampler was skipped for FBX export`);
                return [];
            }
            return [{
                nodeName: channel.node.name,
                nodeId: channel.node.index! + 1,
                path: channel.path,
                sampler: {
                    input,
                    output,
                    interpolation: legacyCompatibility ? 'linear' : (cubic ? 'cubic' : (interpolation === 'step' ? 'step' : 'linear')),
                    // glTF cubic tangents are preserved for vector channels.
                    // Quaternion -> Euler conversion is non-linear, so its
                    // derivative cannot be represented component-wise in FBX.
                    inTangents: !legacyCompatibility && cubic && channel.path !== 'rotation'
                        ? extractGltfCubicSegment(values, 3, 0) : null,
                    outTangents: !legacyCompatibility && cubic && channel.path !== 'rotation'
                        ? extractGltfCubicSegment(values, 3, 2) : null,
                },
            }];
        });
        return channels.length > 0 ? { name: animation.name, duration: animation.duration, channels } : null;
    }).filter(Boolean);
}

function convertGltfScaleKeysToFbx(values: number[]): number[] {
    const converted = Array.from(values);
    for (let offset = 0; offset + 2 < converted.length; offset += 3) {
        [converted[offset + 1], converted[offset + 2]] = [converted[offset + 2], converted[offset + 1]];
    }
    return converted;
}

// Scale is carried by `UnitScaleFactor = 100.0` in the writer's GlobalSettings
// (Blender reads it as the centimeters->meters factor). This function used to
// pre-multiply root matrices by 100 for Blender's legacy importer, which
// would now make every imported scene 100× too large. We keep the signature
// for the call site, but the matrix is returned unchanged.
function scaleFbxRootMatrix(matrix: number[]): number[] {
    return matrix;
}

function composeTrs(translation: ArrayLike<number>, rotation: ArrayLike<number>, scale: ArrayLike<number>): number[] {
    const [x, y, z, w] = Array.from(rotation);
    const [sx, sy, sz] = Array.from(scale);
    const xx = x * x; const yy = y * y; const zz = z * z;
    const xy = x * y; const xz = x * z; const yz = y * z;
    const wx = w * x; const wy = w * y; const wz = w * z;
    return [
        (1 - 2 * (yy + zz)) * sx, (2 * (xy + wz)) * sx, (2 * (xz - wy)) * sx, 0,
        (2 * (xy - wz)) * sy, (1 - 2 * (xx + zz)) * sy, (2 * (yz + wx)) * sy, 0,
        (2 * (xz + wy)) * sz, (2 * (yz - wx)) * sz, (1 - 2 * (xx + yy)) * sz, 0,
        translation[0], translation[1], translation[2], 1,
    ];
}

function packedAttributeNumbers(attribute: GltfJson): number[] {
    const componentSize = componentByteSize(attribute.componentType);
    const elementCount = attribute.count * attribute.components;
    if (attribute.bytes.byteLength !== elementCount * componentSize) {
        throw new Error('Packed glTF attribute has an invalid byte length');
    }
    const view = new DataView(
        attribute.bytes.buffer,
        attribute.bytes.byteOffset,
        attribute.bytes.byteLength,
    );
    const result = new Array(elementCount);
    for (let index = 0; index < elementCount; index += 1) {
        const value = readComponent(view, index * componentSize, attribute.componentType);
        result[index] = attribute.normalized
            ? normalizeComponent(value, attribute.componentType)
            : value;
    }
    return result;
}

function packedIndices(packed: PackedGeometry): number[] {
    const bytes = new Uint8Array(packed.indexBytes());
    const componentType = packed.indexComponentType();
    const componentSize = componentByteSize(componentType);
    if (bytes.byteLength !== packed.indexCount() * componentSize) {
        throw new Error('Packed glTF indices have an invalid byte length');
    }
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    return Array.from(
        { length: packed.indexCount() },
        (_, index) => readComponent(view, index * componentSize, componentType),
    );
}

function triangleIndices(mode: number, source: number[]): number[] {
    if (mode === 4) return source.slice(0, source.length - (source.length % 3));
    const result: number[] = [];
    if (mode === 5) {
        for (let index = 2; index < source.length; index += 1) {
            const a = source[index - 2];
            const b = source[index - 1];
            const c = source[index];
            if (a === b || b === c || a === c) continue;
            // Pushed directly rather than spreading a fresh triple: this runs
            // once per strip triangle, and the fan branch below already does.
            if (index % 2 === 0) result.push(a, b, c);
            else result.push(b, a, c);
        }
    } else if (mode === 6) {
        for (let index = 2; index < source.length; index += 1) {
            const a = source[0];
            const b = source[index - 1];
            const c = source[index];
            if (a !== b && b !== c && a !== c) result.push(a, b, c);
        }
    }
    return result;
}

/**
 * Report the extensions the preview did not act on.
 *
 * @param {Object} document          The glTF JSON document.
 * @param {Map<string, boolean>} honoredSources
 *   Per alternate-source extension, whether every texture that selected an
 *   image through it ended up with a decoded bitmap.
 */
function extensionWarnings(document: GltfJson, honoredSources: Map<unknown, unknown>): string[] {
    const warnings = [];
    const honored = (extension: string) => SUPPORTED_EXTENSIONS.has(extension)
        || honoredSources.get(extension) === true;
    const unsupported = (document.extensionsUsed || [])
        .filter((extension: string) => !honored(extension));
    if (unsupported.length > 0) {
        warnings.push(`Unsupported glTF extensions ignored: ${unsupported.join(', ')}`);
    }
    if ((document.extensionsRequired || []).some((extension: string) => !honored(extension))) {
        warnings.push(
            'Model requires extensions that this viewer ignores; rendering may be incomplete',
        );
    }
    return warnings;
}

export function buildNodes(defs: GltfJson[]): ViewerNode[] {
    return defs.map((def, index) => {
        const trs = {
            translation: def.translation ? Array.from<number>(def.translation) : [0, 0, 0],
            rotation: def.rotation ? Array.from<number>(def.rotation) : [0, 0, 0, 1],
            scale: def.scale ? Array.from<number>(def.scale) : [1, 1, 1],
        };
        return {
            name: def.name || `node_${index}`,
            trs,
            // A node may use a static matrix instead of TRS. Keep it intact
            // rather than silently rendering the node at the origin.
            localMatrix: Array.isArray(def.matrix) && def.matrix.length === 16
                ? Float32Array.from(def.matrix)
                : null,
            children: (def.children || []).slice(),
            meshIndex: typeof def.mesh === 'number' ? def.mesh : -1,
            skinIndex: typeof def.skin === 'number' ? def.skin : -1,
            weights: Array.isArray(def.weights) ? def.weights.slice() : [],
            world: new Float32Array(16),
            index,
        };
    });
}

function buildMeshes(asset: GltfAsset, defs: GltfJson[], warnings: string[]): ViewerMesh[] {
    return defs.map((def, meshIndex) => {
        const primitives: GltfJson[] = [];
        for (let p = 0; p < def.primitives.length; p++) {
            const packed = asset.readPrimitive(meshIndex, p);
            try {
                const attributes: Record<string, RuntimeAccessor> = {};
                for (let i = 0; i < packed.attributeCount(); i++) {
                    const semantic = packed.attributeSemantic(i);
                    attributes[semantic] = {
                        bytes: new Uint8Array(packed.attributeBytes(i)),
                        componentType: packed.attributeComponentType(i),
                        components: packed.attributeComponents(i),
                        normalized: packed.attributeNormalized(i),
                        count: packed.attributeElementCount(i),
                    };
                }
                const primitive: GltfJson = {
                    attributes,
                    mode: packed.mode(),
                    materialIndex: typeof def.primitives[p].material === 'number'
                        ? def.primitives[p].material
                        : -1,
                    morphPositions: [],
                    morphNormals: [],
                };
                const targets = def.primitives[p].targets || [];
                for (let targetIndex = 0; targetIndex < targets.length; targetIndex++) {
                    const accessorIndex = targets[targetIndex].POSITION;
                    if (typeof accessorIndex !== 'number') {
                        primitive.morphPositions.push(null);
                        continue;
                    }
                    const target = readAccessorAsTyped(asset, accessorIndex);
                    if (target.componentType !== 5126 || target.components !== 3
                        || target.count !== attributes.POSITION.count) {
                        warnings.push(
                            `Morph target ${targetIndex} on mesh ${meshIndex} primitive ${p} has an unsupported POSITION accessor and was ignored`,
                        );
                        primitive.morphPositions.push(null);
                        continue;
                    }
                        primitive.morphPositions.push(target);
                }
                for (let targetIndex = 0; targetIndex < targets.length; targetIndex++) {
                    const accessorIndex = targets[targetIndex].NORMAL;
                    if (typeof accessorIndex !== 'number') {
                        primitive.morphNormals.push(null);
                        continue;
                    }
                    const target = readAccessorAsTyped(asset, accessorIndex);
                    if (target.componentType !== 5126 || target.components !== 3
                        || target.count !== attributes.POSITION.count) {
                        warnings.push(
                            `Morph normal ${targetIndex} on mesh ${meshIndex} primitive ${p} has an unsupported accessor and was ignored`,
                        );
                        primitive.morphNormals.push(null);
                        continue;
                    }
                    primitive.morphNormals.push(target);
                }
                if (targets.some((target: GltfJson) => typeof target.TANGENT === 'number')) {
                    warnings.push(
                        `Morph tangents on mesh ${meshIndex} primitive ${p} are ignored because the preview derives its tangent frame from deformed geometry and UVs`,
                    );
                }
                if (packed.hasIndices()) {
                    primitive.indices = {
                        bytes: new Uint8Array(packed.indexBytes()),
                        componentType: packed.indexComponentType(),
                        count: packed.indexCount(),
                    };
                }
                primitives.push(primitive);
            } finally {
                packed.free();
            }
        }
        return {
            name: def.name || `mesh_${meshIndex}`,
            primitives,
            weights: Array.isArray(def.weights) ? def.weights.slice() : [],
            aabb: meshAabb(primitives),
        };
    });
}

function initializeMorphWeights(nodes: ViewerNode[], meshes: ViewerMesh[], warnings: string[]) {
    for (const node of nodes) {
        const mesh = meshes[node.meshIndex];
        if (!mesh) continue;
        const targetCount = Math.max(0, ...mesh.primitives.map((primitive) => primitive.morphPositions!.length));
        if (targetCount === 0) continue;
        const source = node.weights!.length > 0 ? node.weights! : mesh.weights!;
        node.weights = Float32Array.from(
            Array.from({ length: targetCount }, (_, index) => Number(source[index]) || 0),
        );
        // The preview blends the strongest-weighted targets per frame, so a long
        // target list is fine as long as few of them are active at once.
        const activeTargets = node.weights.reduce((total, weight) => total + (weight ? 1 : 0), 0);
        if (activeTargets > MAX_ACTIVE_MORPH_TARGETS) {
            warnings.push(
                `Morph mesh ${mesh.name} holds ${activeTargets} non-zero weights; the preview blends the ${MAX_ACTIVE_MORPH_TARGETS} strongest`,
            );
        }
    }
}

export function readAccessorAsTyped(asset: GltfAsset, index: number) {
    const packed = asset.readAccessor(index);
    try {
        const componentType = packed.componentType();
        const components = packed.components();
        const count = packed.count();
        const bytes = new Uint8Array(packed.bytes());
        const typedView = bytesAsTyped(componentType, bytes);
        return {
            componentType,
            components,
            count,
            normalized: packed.normalized(),
            bytes,
            data: typedView,
        };
    } finally {
        packed.free();
    }
}

function bytesAsTyped(componentType: number, bytes: Uint8Array) {
    switch (componentType) {
        case 5120: return new Int8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        case 5121: return new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        case 5122: return new Int16Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 2);
        case 5123: return new Uint16Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 2);
        case 5125: return new Uint32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 4);
        case 5126: return new Float32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 4);
        default: return new Float32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 4);
    }
}

function buildSkins(asset: GltfAsset, defs: GltfJson[], nodes: ViewerNode[], warnings: string[]): ViewerSkin[] {
    return defs.map((def, skinIndex) => {
        const joints = (def.joints || []).map((jointNodeIndex: number) => ({
            node: nodes[jointNodeIndex],
            inverseBind: identityMat4(),
        }));
        if (typeof def.inverseBindMatrices === 'number') {
            try {
                const accessor = readAccessorAsTyped(asset, def.inverseBindMatrices);
                if (accessor.componentType === 5126 && accessor.components === 16) {
                    for (let i = 0; i < joints.length && i < accessor.count; i++) {
                        const src = accessor.data.subarray(i * 16, (i + 1) * 16);
                        joints[i].inverseBind = Float32Array.from(src);
                    }
                }
            } catch (error) {
                warnings.push(`Failed to read skin inverse bind matrices: ${(error as Error).message}`);
            }
        }
        return {
            name: def.name || `skin_${skinIndex}`,
            joints,
        };
    });
}

function identityMat4(): Float32Array {
    const m = new Float32Array(16);
    m[0] = m[5] = m[10] = m[15] = 1;
    return m;
}

function buildMaterials(defs: GltfJson[]): GltfJson[] {
    const fallback = {
        baseColorFactor: [1, 1, 1, 1],
        doubleSided: false,
        alphaMode: 'OPAQUE',
        unlit: false,
    };
    const list: GltfJson[] = defs.map((def, idx) => {
        const pbr = def.pbrMetallicRoughness || {};
        const baseColor = pbr.baseColorTexture || null;
        const transform = baseColor?.extensions?.KHR_texture_transform || {};
        return {
            name: def.name || `material_${idx}`,
            baseColorFactor: pbr.baseColorFactor || [1, 1, 1, 1],
            baseColorTexture: typeof baseColor?.index === 'number' ? baseColor.index : null,
            baseColorTexCoord: transform.texCoord ?? baseColor?.texCoord ?? 0,
            baseColorTextureTransform: {
                offset: transform.offset || [0, 0],
                scale: transform.scale || [1, 1],
                rotation: transform.rotation ?? 0,
            },
            metallic: pbr.metallicFactor ?? 1,
            roughness: pbr.roughnessFactor ?? 1,
            metallicRoughnessTexture: textureBinding(pbr.metallicRoughnessTexture),
            emissiveFactor: def.emissiveFactor || [0, 0, 0],
            emissiveTexture: textureBinding(def.emissiveTexture),
            normalTexture: textureBinding(def.normalTexture, 'scale', 1),
            occlusionTexture: textureBinding(def.occlusionTexture, 'strength', 1),
            doubleSided: !!def.doubleSided,
            alphaMode: def.alphaMode || 'OPAQUE',
            alphaCutoff: def.alphaCutoff ?? 0.5,
            unlit: !!def.extensions?.KHR_materials_unlit,
        };
    });
    list.push(fallback);
    return list;
}

function textureBinding(info: GltfJson, scalarName: string | null = null, fallback = 1): GltfJson {
    if (!info || typeof info.index !== 'number') return null;
    const binding: GltfJson = {
        index: info.index,
        texCoord: info.texCoord ?? 0,
    };
    if (scalarName) binding[scalarName] = info[scalarName] ?? fallback;
    return binding;
}

async function buildTextures(asset: GltfAsset, manifest: GltfJson, resources: ResourceMap, hooks: ImportHooks) {
    const images = await decodeImages(asset, manifest.images || [], resources, hooks);
    const samplers = (manifest.samplers || []).map((s: GltfJson) => ({
        wrapS: s.wrapS ?? GL.REPEAT,
        wrapT: s.wrapT ?? GL.REPEAT,
        minFilter: s.minFilter ?? GL.LINEAR_MIPMAP_LINEAR,
        magFilter: s.magFilter ?? GL.LINEAR,
    }));
    const defaultSampler = {
        wrapS: GL.REPEAT,
        wrapT: GL.REPEAT,
        minFilter: GL.LINEAR_MIPMAP_LINEAR,
        magFilter: GL.LINEAR,
    };

    // An alternate-source extension is honored only while every texture that
    // reads through it produced a bitmap: the codec belongs to the host, so
    // this is an observation rather than a support claim.
    const honoredSources = new Map();
    const textures = (manifest.textures || []).map((tex: GltfJson, idx: number) => {
        const samplerIndex = typeof tex.sampler === 'number' ? tex.sampler : -1;
        const sampler = samplerIndex >= 0 ? samplers[samplerIndex] : defaultSampler;
        const { source, extension } = textureSource(tex);
        const image = source >= 0 ? images[source] : null;
        if (extension) {
            honoredSources.set(
                extension,
                (honoredSources.get(extension) ?? true) && Boolean(image?.bitmap),
            );
        }
        return {
            name: tex.name || `texture_${idx}`,
            image: image?.bitmap || null,
            flipY: false,
            wrapS: sampler.wrapS,
            wrapT: sampler.wrapT,
            minFilter: sampler.minFilter,
            magFilter: sampler.magFilter,
        };
    });
    return { textures, honoredSources };
}

/**
 * Resolve the image a texture reads.
 *
 * @returns {{ source: number, extension: string|null }}
 *   The image index, or -1, and the alternate-source extension that named it.
 */
function textureSource(texture: GltfJson) {
    if (Number.isInteger(texture.source)) return { source: texture.source, extension: null };
    for (const extension of TEXTURE_SOURCE_EXTENSIONS) {
        const source = texture.extensions?.[extension]?.source;
        if (Number.isInteger(source)) return { source, extension };
    }
    return { source: -1, extension: null };
}

async function decodeImages(asset: GltfAsset, defs: GltfJson[], resources: ResourceMap, hooks: ImportHooks) {
    return Promise.all(
        defs.map(async (def) => {
            try {
                if (typeof def.bufferView === 'number') {
                    const bytes = new Uint8Array(asset.bufferViewBytes(def.bufferView));
                    const mime = def.mimeType || sniffMime(bytes);
                    const bitmap = await loadImageBytes(bytes, mime, hooks);
                    return { bitmap, mime };
                }
                if (def.uri) {
                    const bytes = resolveUriBytes(def.uri, resources);
                    if (!bytes) {
                        hooks.onLog?.(`Image not found: ${def.uri}`, 'warning');
                        return { bitmap: null, mime: null };
                    }
                    const mime = def.mimeType || sniffMime(bytes) || mimeFromUri(def.uri);
                    const bitmap = await loadImageBytes(bytes, mime, hooks);
                    return { bitmap, mime };
                }
                return { bitmap: null, mime: null };
            } catch (error) {
                hooks.onLog?.(`Failed to decode image: ${(error as Error).message}`, 'warning');
                return { bitmap: null, mime: null };
            }
        }),
    );
}

async function loadImageBytes(bytes: Uint8Array, mime: string, hooks: ImportHooks) {
    if (mime === 'image/ktx2') {
        hooks.onLog?.('KTX2 textures require a transcoder; skipping image', 'warning');
        return null;
    }
    // BlobPart excludes SharedArrayBuffer-backed views; these never are.
    const blob = new Blob([bytes as BlobPart], { type: mime || 'application/octet-stream' });
    try {
        return await createImageBitmap(blob);
    } catch (error) {
        hooks.onLog?.(`createImageBitmap failed: ${(error as Error).message}`, 'warning');
        return null;
    }
}

/** Re-exported under its historical name for existing importers. */
export const resolveUriBytes = resolveResource;

/**
 * Most morph weights a weights sampler ever holds at once. Cubic keyframes store
 * [inTangent, value, outTangent], so only their middle block is a pose, and an
 * interpolated segment can carry both of its endpoint poses at the same time.
 */
function peakActiveMorphWeights(sampler: GltfJson, targetCount: number): number {
    if (targetCount <= 0 || !sampler.output) return 0;
    const interpolation = String(sampler.interpolation || 'LINEAR').toUpperCase();
    const cubic = interpolation === 'CUBICSPLINE';
    const stride = cubic ? targetCount * 3 : targetCount;
    const offset = cubic ? targetCount : 0;
    const keyframes = Math.floor(sampler.output.length / stride);
    const poses = [];
    for (let key = 0; key < keyframes; key++) {
        const pose = [];
        for (let i = 0; i < targetCount; i++) {
            if (sampler.output[key * stride + offset + i]) pose.push(i);
        }
        poses.push(pose);
    }
    const blends = interpolation !== 'STEP';
    let peak = 0;
    for (let key = 0; key < poses.length; key++) {
        const segment = blends && key + 1 < poses.length
            ? new Set([...poses[key], ...poses[key + 1]])
            : new Set(poses[key]);
        if (segment.size > peak) peak = segment.size;
    }
    return peak;
}

export function buildAnimations(asset: GltfAsset, defs: GltfJson[], nodes: ViewerNode[], warnings: string[]): GltfJson[] {
    return defs.map((def, animIndex) => {
        const name = def.name || `animation_${animIndex}`;
        const samplers = (def.samplers || []).map((s: GltfJson) => {
            const input = readAccessorAsTyped(asset, s.input);
            const output = readAccessorAsTyped(asset, s.output);
            return {
                input: input.data,
                output: output.data,
                interpolation: s.interpolation || 'LINEAR',
            };
        });

        const channels = (def.channels || []).map((ch: GltfJson) => {
            const target = ch.target || {};
            const node = nodes[target.node];
            const sampler = samplers[ch.sampler];
            if (!node || !sampler) return null;
            const targetCount = target.path === 'weights' ? node.weights!.length : 0;
            if (!['translation', 'rotation', 'scale', 'weights'].includes(target.path)
                || (target.path === 'weights' && targetCount === 0)) {
                warnings.push(
                    `Animation ${name}: ${target.path} channels are not supported by the preview and were ignored`,
                );
                return null;
            }
            if (target.path === 'weights') {
                const active = peakActiveMorphWeights(sampler, targetCount);
                if (active > MAX_ACTIVE_MORPH_TARGETS) {
                    warnings.push(
                        `Animation ${name}: a weights keyframe drives ${active} targets at once; the preview blends the ${MAX_ACTIVE_MORPH_TARGETS} strongest`,
                    );
                }
            }
            return {
                node,
                path: target.path,
                sampler,
                targetCount,
            };
        }).filter(Boolean);

        let duration = 0;
        for (const { sampler: s } of channels) {
            if (s.input.length > 0) duration = Math.max(duration, s.input[s.input.length - 1]);
        }

        if (channels.length === 0) return null;

        return {
            name,
            duration,
            channels,
        };
    }).filter(Boolean);
}

function computeRenderables(nodes: ViewerNode[], meshes: ViewerMesh[], skins: ViewerSkin[], rootIndices: number[]) {
    const aabb = {
        min: [Infinity, Infinity, Infinity],
        max: [-Infinity, -Infinity, -Infinity],
    };
    const renderables: Renderable[] = [];

    const visited = new Set<number>();
    function walk(nodeIndex: number) {
        if (visited.has(nodeIndex)) return;
        visited.add(nodeIndex);
        const node = nodes[nodeIndex];
        if (!node) return;
        if (node.meshIndex >= 0 && meshes[node.meshIndex]) {
            const skinIndex = node.skinIndex;
            renderables.push({ node, meshIndex: node.meshIndex, skinIndex });
            accumulateAabb(aabb, meshes[node.meshIndex]);
        }
        for (const child of node.children) walk(child);
    }
    for (const root of rootIndices) walk(root);

    if (!isFinite(aabb.min[0])) {
        aabb.min = [-0.5, -0.5, -0.5];
        aabb.max = [0.5, 0.5, 0.5];
    }

    return { renderables, aabb };
}

function accumulateAabb(box: Aabb, mesh: Pick<ViewerMesh, 'primitives'>) {
    for (const prim of mesh.primitives) {
        const pos = prim.attributes.POSITION;
        if (!pos) continue;
        const view = bytesAsTyped(pos.componentType, pos.bytes as Uint8Array);
        const components = pos.components;
        for (let i = 0; i < pos.count; i++) {
            const x = view[i * components];
            const y = view[i * components + 1];
            const z = view[i * components + 2];
            if (x < box.min[0]) box.min[0] = x;
            if (y < box.min[1]) box.min[1] = y;
            if (z < box.min[2]) box.min[2] = z;
            if (x > box.max[0]) box.max[0] = x;
            if (y > box.max[1]) box.max[1] = y;
            if (z > box.max[2]) box.max[2] = z;
        }
    }
}

function meshAabb(primitives: GltfJson[]): Aabb {
    const aabb = {
        min: [Infinity, Infinity, Infinity],
        max: [-Infinity, -Infinity, -Infinity],
    };
    accumulateAabb(aabb, { primitives });
    if (!isFinite(aabb.min[0])) {
        aabb.min = [-0.5, -0.5, -0.5];
        aabb.max = [0.5, 0.5, 0.5];
    }
    return aabb;
}
