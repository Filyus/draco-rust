/**
 * SceneDocument -> glTF/GLB export boundary.
 *
 * This module only lowers the portable contract into a glTF JSON document and
 * its byte-backed resource map. Container validation and GLB construction are
 * deliberately delegated to GltfAsset (gltf-wasm / draco-gltf), rather than
 * reimplemented in JavaScript.
 */

import { componentByteWidth, readComponent } from './component-values.ts';
import { decomposeMat4 } from './mat4.ts';
import { assertValidSceneDocument } from './scene-document.ts';
import type {
  SceneAccessor,
  SceneAnimation,
  SceneDocument,
  SceneMaterial,
  SceneLight,
  SceneMesh,
  SceneNode,
  SceneSkin,
  TextureInfo,
} from './scene-document.ts';
import { writeMaterialExtensions } from './material-extensions.ts';
import { GLTF_TEXTURE_SOURCE_EXTENSIONS } from './gltf-interpretation.ts';
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
  byteStride?: number;
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
  extensions?: Record<string, unknown>;
  children?: number[];
  mesh?: number;
  skin?: number;
  weights?: number[];
  matrix?: number[];
  translation?: number[];
  rotation?: number[];
  scale?: number[];
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
  // Morph target positions carry bounds too, not just the base attribute.
  const positionAccessors = new Set(document.meshes.flatMap((mesh) => mesh.primitives.flatMap((primitive) => [
    primitive.attributes.POSITION,
    ...(primitive.targets || []).map((target) => target.POSITION),
  ])));
  const animationInputs = new Set(document.animations.flatMap((clip) => clip.samplers.map((sampler) => sampler.input)));
  const accessorTargets = geometryAccessorTargets(document.meshes);
  const vertexAttributes = vertexAttributeAccessors(document.meshes);
  const accessors = document.accessors.map((accessor, index) => lowerAccessor(
    accessor, index, binary, bufferViews, positionAccessors.has(index), animationInputs.has(index),
    accessorTargets.get(index), vertexAttributes.has(index),
  ));
  const images: { name: string; bufferView: number; mimeType: string }[] = [];
  const imageByResource = new Map<number, number>();
  const samplers: Record<string, number>[] = [];
  const samplerByKey = new Map<string, number>();
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
    // Samplers carry no identity in glTF, so textures that want the same
    // filtering share one record rather than each getting a copy of it.
    const settings = {
      wrapS: texture.sampler?.wrapS ?? 10497,
      wrapT: texture.sampler?.wrapT ?? 10497,
      minFilter: texture.sampler?.minFilter ?? 9987,
      magFilter: texture.sampler?.magFilter ?? 9729,
    };
    const key = `${settings.wrapS}:${settings.wrapT}:${settings.minFilter}:${settings.magFilter}`;
    let sampler = samplerByKey.get(key);
    if (sampler === undefined) {
      sampler = samplers.length;
      samplers.push(settings);
      samplerByKey.set(key, sampler);
    }
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
  // Declared from what was actually written, not from a hand-kept list: a
  // material extension that reaches lowerMaterial but not extensionsUsed
  // produces a file readers are entitled to ignore.
  const extensionsUsed = new Set<string>();
  for (const material of materials) {
    for (const extension of Object.keys(material.extensions || {})) extensionsUsed.add(extension);
  }
  if (materials.some(materialUsesTextureTransform)) extensionsUsed.add('KHR_texture_transform');
  // An alternate image source is written without a JPEG or PNG fallback,
  // because the document holds one encoding of the image and inventing a
  // second is not a serialization decision. Both extensions say what that
  // costs: with no fallback to fall back to, a reader that skips the extension
  // finds a texture with no source at all, so the extension may not be
  // declared optional. The official validator does not enforce this, which is
  // exactly why it has to be stated here rather than caught downstream.
  const extensionsRequired = new Set<string>();
  for (const extension of GLTF_TEXTURE_SOURCE_EXTENSIONS) {
    if (!textures.some((texture) => texture?.extensions?.[extension])) continue;
    extensionsUsed.add(extension);
    extensionsRequired.add(extension);
  }
  // Vertex attributes keep the component type they arrived in, so an asset that
  // came from gltfpack goes back out quantized. Core glTF allows only float for
  // most semantics; without this declaration the file is invalid, and the
  // accessors were already being written this way before it was added.
  const quantized = usesQuantizedAttributes(document);
  if (quantized) {
    extensionsUsed.add('KHR_mesh_quantization');
    extensionsRequired.add('KHR_mesh_quantization');
  }

  // KHR_lights_punctual keeps the lights at the root and has nodes point at
  // them, which is how the document holds them too.
  const lights = (document.lights ?? []).map(lowerLight);
  if (lights.length > 0) extensionsUsed.add('KHR_lights_punctual');
  // KHR_materials_variants states the names at the root and the choices on the
  // primitives, which is where the document keeps them too.
  const variants = (document.variants ?? []).map((name) => ({ name }));
  if (variants.length > 0) extensionsUsed.add('KHR_materials_variants');
  // EXT_mesh_gpu_instancing lives entirely on the node, so declaring it is the
  // only root-level trace it leaves.
  if (document.nodes.some((node) => node.instancing)) extensionsUsed.add('EXT_mesh_gpu_instancing');

  const manifest = {
    asset: { version: '2.0', generator: 'draco-rust SceneDocument exporter' },
    ...(lights.length > 0 || variants.length > 0
      ? {
        extensions: {
          ...(lights.length > 0 ? { KHR_lights_punctual: { lights } } : {}),
          ...(variants.length > 0 ? { KHR_materials_variants: { variants } } : {}),
        },
      }
      : {}),
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
    // What a reader may not skip: quantization, because ignoring it means
    // misreading the vertex data outright, and an alternate image source with
    // no fallback, because ignoring it leaves the texture with no image. The
    // material layers, by contrast, degrade to plain PBR and stay optional.
    ...(extensionsRequired.size > 0 ? { extensionsRequired: [...extensionsRequired] } : {}),
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
  isVertexAttribute: boolean,
) {
  const type = ACCESSOR_TYPES.get(accessor.components);
  if (!type) throw new Error(`SceneDocument accessor ${index} has unsupported ${accessor.components}-component shape`);
  // glTF requires every vertex attribute element to start on a 4-byte
  // boundary. Quantized attributes break that on their own — a vec3 of shorts
  // is six bytes — so they go out strided rather than tightly packed.
  const elementSize = (componentByteWidth(accessor.componentType) || 0) * accessor.components;
  const strided = isVertexAttribute && elementSize > 0 && elementSize % 4 !== 0;
  const byteStride = strided ? Math.ceil(elementSize / 4) * 4 : undefined;
  const bytes = byteStride ? padElements(accessor.bytes, accessor.count, elementSize, byteStride) : accessor.bytes;
  const bufferView = appendBufferView(binary, bufferViews, bytes, target, byteStride);
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
  byteStride?: number,
) {
  binary.align();
  const byteOffset = binary.length;
  binary.write(bytes);
  const output: GltfBufferView = { buffer: 0, byteOffset, byteLength: bytes.byteLength };
  if (target !== undefined) output.target = target;
  if (byteStride !== undefined) output.byteStride = byteStride;
  bufferViews.push(output);
  return bufferViews.length - 1;
}

/** Re-lay elements at `byteStride`, leaving the padding zeroed. */
function padElements(bytes: Uint8Array, count: number, elementSize: number, byteStride: number) {
  const padded = new Uint8Array(count * byteStride);
  for (let element = 0; element < count; element += 1) {
    padded.set(bytes.subarray(element * elementSize, (element + 1) * elementSize), element * byteStride);
  }
  return padded;
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
  const extensions = writeMaterialExtensions(material, (property, scale) => {
    const source = material[property as keyof SceneMaterial] as TextureInfo | undefined;
    const info = texture(source, slotLabel(property));
    // A normal map's scale sits beside the binding rather than inside it, and
    // clearcoat is the one extension that brings a normal map of its own.
    if (info && scale && source?.scale !== undefined) info.scale = source.scale;
    return info;
  });
  if (Object.keys(extensions).length > 0) output.extensions = extensions;
  if (output.alphaMode === 'MASK') output.alphaCutoff = material.alphaCutoff ?? 0.5;
  return output;
}

/**
 * The human wording a texture-slot warning uses.
 *
 * The property name is the table's key; a reader of the warning wants the slot
 * the way the glTF spec names it, so `clearcoatNormalTexture` reads back as
 * "clearcoat normal".
 */
/**
 * One light, in the shape the extension states.
 *
 * A spot's cone lives in a nested object there but flat on the portable light,
 * because only spots have one and a nested object of two angles is a shape
 * every consumer would have to unwrap. Defaults are omitted: the extension's
 * own are what an absent field means.
 */
/**
 * A primitive's variant choices, grouped the way the extension writes them.
 *
 * The document maps variant to material because that is the question a
 * consumer asks; the extension lists material to variants because a material
 * is usually shared by several. Same information, and this is where it turns
 * around.
 */
function lowerVariantMappings(selected: Record<number, number> | undefined) {
  if (!selected) return null;
  const byMaterial = new Map<number, number[]>();
  for (const [variant, material] of Object.entries(selected)) {
    byMaterial.set(material, [...(byMaterial.get(material) ?? []), Number(variant)]);
  }
  const mappings = [...byMaterial].map(([material, variants]) => ({ material, variants: variants.sort((a, b) => a - b) }));
  return mappings.length > 0 ? { KHR_materials_variants: { mappings } } : null;
}

function lowerLight(light: SceneLight, index: number) {
  const output: Record<string, unknown> = {
    type: light.type,
    name: light.name || `light_${index}`,
    color: [...(light.color || [1, 1, 1])],
    intensity: light.intensity ?? 1,
  };
  if (light.range !== undefined) output.range = light.range;
  if (light.type === 'spot') {
    output.spot = {
      innerConeAngle: light.innerConeAngle ?? 0,
      outerConeAngle: light.outerConeAngle ?? Math.PI / 4,
    };
  }
  return output;
}

function slotLabel(property: string): string {
  return property
    .replace(/Texture$/, '')
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .toLowerCase();
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
        ...(lowerVariantMappings(primitive.variantMaterials)
          ? { extensions: lowerVariantMappings(primitive.variantMaterials)! }
          : {}),
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
    // Both node extensions share one object: a node may well place a light and
    // draw its mesh many times.
    ...(node.light === undefined && !node.instancing ? {} : {
      extensions: {
        ...(node.light === undefined ? {} : { KHR_lights_punctual: { light: node.light } }),
        ...(node.instancing
          ? { EXT_mesh_gpu_instancing: { attributes: { ...node.instancing.attributes } } }
          : {}),
      },
    }),
  };
  if (node.mesh !== undefined && (node.mesh < 0 || node.mesh >= meshCount)) throw new Error(`SceneDocument node ${index} has invalid mesh`);
  if (node.skin !== undefined && (node.skin < 0 || node.skin >= skinCount)) throw new Error(`SceneDocument node ${index} has invalid skin`);
  if (node.matrix && !animatedNodes.has(index)) output.matrix = [...node.matrix];
  else if (node.matrix) {
    Object.assign(output, decomposeMat4(node.matrix));
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

/** Accessors read as vertex attributes, morph target deltas included. */
function vertexAttributeAccessors(meshes: SceneMesh[]) {
  const attributes = new Set<number>();
  for (const mesh of meshes) for (const primitive of mesh.primitives) {
    for (const accessor of Object.values(primitive.attributes)) attributes.add(accessor);
    for (const target of primitive.targets || []) {
      for (const accessor of Object.values(target)) attributes.add(accessor);
    }
  }
  return attributes;
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

function materialUsesTextureTransform(material: GltfMaterial) {
  return loweredTextureInfos(material).some((info) => Boolean(info?.extensions?.KHR_texture_transform));
}

/** Every textureInfo a lowered material carries, layered extensions included. */
function loweredTextureInfos(material: GltfMaterial): (GltfTextureInfo | undefined)[] {
  const pbr = material.pbrMetallicRoughness as {
    baseColorTexture?: GltfTextureInfo;
    metallicRoughnessTexture?: GltfTextureInfo;
  };
  const layered = Object.values(material.extensions || {})
    .flatMap((extension) => Object.values(extension as Record<string, unknown>))
    .filter((value): value is GltfTextureInfo => (
      Boolean(value) && typeof value === 'object' && Number.isInteger((value as GltfTextureInfo).index)
    ));
  return [
    pbr.baseColorTexture,
    pbr.metallicRoughnessTexture,
    material.normalTexture,
    material.occlusionTexture,
    material.emissiveTexture,
    ...layered,
  ];
}

/**
 * Component types core glTF 2.0 accepts for a vertex attribute.
 *
 * Anything outside them is what KHR_mesh_quantization exists to permit. An
 * unknown semantic is nobody's business here and passes.
 */
function isCoreAttributeAccessor(semantic: string, accessor: SceneAccessor | undefined) {
  if (!accessor) return true;
  const type = accessor.componentType;
  const normalized = Boolean(accessor.normalized);
  const smallNormalized = (type === 5121 || type === 5123) && normalized;
  if (semantic === 'POSITION' || semantic === 'NORMAL' || semantic === 'TANGENT') return type === 5126;
  if (semantic.startsWith('TEXCOORD_') || semantic.startsWith('COLOR_') || semantic.startsWith('WEIGHTS_')) {
    return type === 5126 || smallNormalized;
  }
  if (semantic.startsWith('JOINTS_')) return type === 5121 || type === 5123;
  return true;
}

/**
 * Whether any vertex attribute needs KHR_mesh_quantization to be legal.
 *
 * Only attributes: indices are integer in core glTF, and skin joints are
 * unsigned byte or short by definition, so neither implies the extension.
 */
function usesQuantizedAttributes(document: SceneDocument) {
  for (const mesh of document.meshes) for (const primitive of mesh.primitives) {
    for (const [semantic, accessor] of Object.entries(primitive.attributes)) {
      if (!isCoreAttributeAccessor(semantic, document.accessors[accessor])) return true;
    }
    for (const target of primitive.targets || []) {
      for (const [semantic, accessor] of Object.entries(target)) {
        if (!isCoreAttributeAccessor(semantic, document.accessors[accessor])) return true;
      }
    }
  }
  return false;
}

function textureSourceExtension(mimeType: string | undefined) {
  if (mimeType === 'image/png' || mimeType === 'image/jpeg') return 'core';
  if (mimeType === 'image/webp') return 'EXT_texture_webp';
  if (mimeType === 'image/avif') return 'EXT_texture_avif';
  if (mimeType === 'image/ktx2') return 'KHR_texture_basisu';
  return null;
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
