/**
 * glTF/GLB -> SceneDocument import boundary.
 *
 * This adapter owns glTF accessor/resource interpretation and deliberately
 * returns only the portable byte-backed contract. It neither creates browser
 * images nor imports any FBX compatibility policy.
 */

import {
  MATERIAL_TEXTURE_SLOTS, assertValidSceneDocument, createSceneDocument,
} from './scene-document.ts';
import type {
  AnimationChannel, AnimationSampler, AttributeMap, ComponentType, SceneAccessor, SceneDocument,
  SceneMaterial, SceneNode, ScenePrimitive, TextureInfo,
} from './scene-document.ts';
import {
  GLTF_TEXTURE_SOURCE_EXTENSIONS,
  gltfExtensionWarnings,
  isInterpretedGltfExtension,
  readGltfMaterial,
  resolveSampler,
  resolveTextureSource,
} from './gltf-interpretation.ts';
import type { InterpretedTexture } from './gltf-interpretation.ts';
import {
  appendAccessor, basename, bytesFromF32, mimeFromUri, resolveResource, sniffMime,
} from './scene-resources.ts';
import type { ResourceMap } from './scene-resources.ts';
import { assertConverterProfile } from './wasm-modules.ts';
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

/** Extract a portable SceneDocument from an existing GltfAsset-capable module. */
export function buildSceneDocumentFromGltf(
  sourceData: Uint8Array,
  resources: Record<string, Uint8Array>,
  gltfModule: GltfModule,
): SceneDocument {
  const asset = gltfModule.GltfAsset.withResources(sourceData, resources, '2.1');
  try {
    assertConverterProfile(asset);
    const manifest: GltfJson = JSON.parse(new TextDecoder().decode(asset.json()));
    const document = createSceneDocument({ warnings: extensionWarnings(manifest) });
    const accessorBySource = new Map<string, number>();
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
    const { source } = resolveTextureSource(texture);
    const resource = imageResources[source];
    if (!Number.isInteger(resource) || resource < 0) {
      document.warnings.push(`glTF texture ${textureIndex} has no supported image source and was omitted`);
      return -1;
    }
    const textureIndexInDocument = document.textures.length;
    document.textures.push({
      name: texture.name || `texture_${textureIndex}`,
      resource,
      sampler: resolveSampler(Number.isInteger(texture.sampler) ? samplers[texture.sampler] : null),
    });
    return textureIndexInDocument;
  });
}

/**
 * Lower interpreted glTF materials into portable ones.
 *
 * Extension-borne values are written only when they differ from the value the
 * core model implies, so a document built from an asset without those
 * extensions is byte-for-byte what it was before they were representable — and
 * so the writer can decide what to emit by looking at the material alone.
 */
function collectMaterials(materials: GltfJson[], textureBySource: number[], document: SceneDocument) {
  document.materials.push(...materials.map((definition, materialIndex) => {
    const source = readGltfMaterial(definition, materialIndex);
    const slot = (binding: InterpretedTexture | null) => portableTextureInfo(binding, textureBySource);
    const material: SceneMaterial = {
      name: source.name,
      baseColorFactor: [...source.baseColorFactor],
      metallicFactor: source.metallicFactor,
      roughnessFactor: source.roughnessFactor,
      emissiveFactor: [...source.emissiveFactor],
      doubleSided: source.doubleSided,
      alphaMode: source.alphaMode as SceneMaterial['alphaMode'],
      alphaCutoff: source.alphaCutoff,
      unlit: source.unlit,
    };
    for (const key of MATERIAL_TEXTURE_SLOTS) {
      const info = slot(source[key]);
      if (info) material[key] = info;
    }
    if (source.emissiveStrength !== 1) material.emissiveStrength = source.emissiveStrength;
    if (source.ior !== 1.5) material.ior = source.ior;
    if (source.specularFactor !== 1) material.specularFactor = source.specularFactor;
    if (source.specularColorFactor.some((value) => value !== 1)) {
      material.specularColorFactor = [...source.specularColorFactor];
    }
    if (source.clearcoatFactor !== 0) material.clearcoatFactor = source.clearcoatFactor;
    if (source.clearcoatRoughnessFactor !== 0) {
      material.clearcoatRoughnessFactor = source.clearcoatRoughnessFactor;
    }
    return material;
  }));
}

/**
 * Move a texture binding into document index space.
 *
 * A texture whose image the document could not carry was dropped by
 * `collectTextures`; the slot that referenced it is dropped too rather than
 * pointing at whatever texture inherited its index.
 */
function portableTextureInfo(
  binding: InterpretedTexture | null,
  textureBySource: number[],
): TextureInfo | null {
  if (!binding) return null;
  const texture = textureBySource[binding.index];
  if (!Number.isInteger(texture) || texture < 0) return null;
  return {
    texture,
    texCoord: binding.texCoord,
    ...(binding.transform ? { transform: { ...binding.transform } } : {}),
    ...(binding.scale === undefined ? {} : { scale: binding.scale }),
    ...(binding.strength === undefined ? {} : { strength: binding.strength }),
  };
}

function collectMeshes(
  asset: GltfAsset,
  meshes: GltfJson[],
  document: SceneDocument,
  accessorBySource: Map<string, number>,
) {
  document.meshes.push(...meshes.map((mesh, meshIndex) => ({
    name: mesh.name || `mesh_${meshIndex}`,
    weights: Array.from<number>(mesh.weights || []),
    primitives: (mesh.primitives || []).map((primitive: GltfJson, primitiveIndex: number) => {
      const packed = asset.readPrimitive(meshIndex, primitiveIndex);
      try {
        const attributes: AttributeMap = {};
        for (let index = 0; index < packed.attributeCount(); index += 1) {
          attributes[packed.attributeSemantic(index)] = reuseAccessor(
            document,
            accessorBySource,
            'attribute',
            packed.attributeSourceAccessor(index),
            () => ({
              bytes: new Uint8Array(packed.attributeBytes(index)),
              componentType: componentType(packed.attributeComponentType(index)),
              components: packed.attributeComponents(index),
              count: packed.attributeElementCount(index),
              normalized: packed.attributeNormalized(index),
            }),
          );
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
          result.indices = reuseAccessor(
            document,
            accessorBySource,
            'indices',
            packed.indexSourceAccessor(),
            () => ({
              bytes: new Uint8Array(packed.indexBytes()),
              componentType: componentType(packed.indexComponentType()),
              components: 1,
              count: packed.indexCount(),
              normalized: false,
            }),
          );
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
  accessorBySource: Map<string, number>,
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
  accessorBySource: Map<string, number>,
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
        let output = dequantizedOutput(document, sourceAccessor(asset, source.output, document, accessorBySource));
        if (output < 0) {
          document.warnings.push(`glTF animation ${animation.name || animationIndex} stores its output in an unsupported component type and it was omitted`);
          continue;
        }
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

/** Normalized integer ranges glTF allows for animation sampler outputs. */
const NORMALIZED_RANGES: Record<number, { max: number; signed: boolean }> = {
  5120: { max: 127, signed: true },
  5121: { max: 255, signed: false },
  5122: { max: 32767, signed: true },
  5123: { max: 65535, signed: false },
};

/**
 * Expands a normalized sampler output to float, or returns -1 when the
 * component type has no defined float meaning.
 *
 * glTF stores rotations and morph weights as normalized integers too, while a
 * SceneDocument sampler is always float.
 */
function dequantizedOutput(document: SceneDocument, index: number): number {
  const accessor = document.accessors[index];
  if (accessor.componentType === 5126) return index;
  const range = NORMALIZED_RANGES[accessor.componentType];
  if (!range || !accessor.normalized) return -1;
  const size = range.max > 255 ? 2 : 1;
  const total = accessor.count * accessor.components;
  const view = new DataView(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength);
  if (total * size > accessor.bytes.byteLength) return -1;
  const values = new Float32Array(total);
  for (let i = 0; i < total; i += 1) {
    const raw = size === 1
      ? (range.signed ? view.getInt8(i) : view.getUint8(i))
      : (range.signed ? view.getInt16(i * 2, true) : view.getUint16(i * 2, true));
    values[i] = range.signed ? Math.max(raw / range.max, -1) : raw / range.max;
  }
  return appendAccessor(document, {
    bytes: bytesFromF32(values),
    componentType: 5126,
    components: accessor.components,
    count: accessor.count,
    normalized: false,
  });
}

function sourceAccessor(
  asset: GltfAsset,
  sourceIndex: number,
  document: SceneDocument,
  cache: Map<string, number>,
): number {
  return reuseAccessor(document, cache, 'raw', sourceIndex, () => {
    const packed = asset.readAccessor(sourceIndex);
    try {
      return {
        bytes: new Uint8Array(packed.bytes()),
        componentType: componentType(packed.componentType()),
        components: packed.components(),
        count: packed.count(),
        normalized: packed.normalized(),
      };
    } finally {
      packed.free();
    }
  });
}

/**
 * Append an accessor, or point at the one a previous reader already appended.
 *
 * Primitives share source accessors constantly — a mesh split by material is
 * the standard case — and the materialized bytes cannot say so, which is why
 * the reader reports the source index alongside them. Without this, a mesh of
 * seven primitives over five accessors produced thirty-five copies of the same
 * vertex data, in the document, in every GLB written from it, and in every FBX.
 *
 * The role belongs in the key. `geometryAccessorTargets` assigns one
 * bufferView target per accessor, first writer winning, so an accessor read
 * once as an attribute and once as an index stream must stay two document
 * accessors — sharing it would emit one buffer view with the wrong target.
 *
 * A negative source index means the reader could not name one, which is every
 * attribute of a Draco primitive: its bytes come from the codec stream, and two
 * primitives naming one accessor say nothing about whether they match.
 */
function reuseAccessor(
  document: SceneDocument,
  cache: Map<string, number>,
  role: 'attribute' | 'indices' | 'raw',
  sourceIndex: number,
  read: () => Omit<SceneAccessor, 'min' | 'max'>,
): number {
  if (!Number.isInteger(sourceIndex) || sourceIndex < 0) return appendAccessor(document, read());
  const key = `${role}:${sourceIndex}`;
  const cached = cache.get(key);
  if (cached !== undefined) return cached;
  const index = appendAccessor(document, read());
  cache.set(key, index);
  return index;
}

function lastTime(accessor: SceneAccessor | undefined): number {
  if (!accessor || accessor.componentType !== 5126 || accessor.components !== 1 || accessor.count === 0) return 0;
  const view = new DataView(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength);
  return view.getFloat32((accessor.count - 1) * 4, true);
}

function extensionWarnings(manifest: GltfJson): string[] {
  return gltfExtensionWarnings(
    manifest,
    // The document carries image bytes whatever the codec, so an alternate
    // image source is always honored here — unlike in the preview, which can
    // only claim one once the browser has decoded it.
    (extension) => isInterpretedGltfExtension(extension)
      || (GLTF_TEXTURE_SOURCE_EXTENSIONS as readonly string[]).includes(extension),
    {
      ignored: (names) => `Unsupported glTF extensions omitted from SceneDocument: ${names}`,
      required: 'glTF requires extensions outside the portable SceneDocument subset',
    },
  );
}

function rootNodes(nodes: SceneNode[]): number[] {
  const children = new Set(nodes.flatMap((node) => node.children || []));
  return nodes.map((_, index) => index).filter((index) => !children.has(index));
}

