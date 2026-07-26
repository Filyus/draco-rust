/**
 * One reading of glTF material, texture and extension JSON.
 *
 * Two consumers interpret the same document: `gltf-loader.ts` builds the WebGL
 * preview scene, `gltf-scene-document.ts` builds the portable SceneDocument.
 * They produce deliberately different shapes — a renderer record versus an
 * interchange record — but they must agree on what the JSON *says*, and while
 * each owned its own reader they did not: the preview grew four material
 * extensions the document silently dropped.
 *
 * So this module answers only "what does the document declare", in glTF's own
 * index space, and each consumer projects that into its own shape. It stays
 * free of the browser, of wasm and of both output models, which is also what
 * lets node tests import it without stubbing WebGL.
 */

/**
 * Loosely typed on purpose: everything here is external JSON, inspected field
 * by field rather than trusted.
 */
type GltfJson = any;

/** Extensions both consumers interpret from the JSON themselves. */
export const GLTF_INTERPRETED_EXTENSIONS: ReadonlySet<string> = new Set([
  'KHR_materials_unlit',
  'KHR_materials_clearcoat',
  'KHR_materials_ior',
  'KHR_materials_specular',
  'KHR_materials_emissive_strength',
  'KHR_texture_transform',
]);

/**
 * Extensions the Rust reader resolves before any of this runs: Draco and
 * meshopt payloads arrive decoded, and quantized attributes arrive in their
 * storage type, which both consumers carry as-is. Nothing is lost, so neither
 * consumer should report them as ignored.
 */
export const GLTF_READER_RESOLVED_EXTENSIONS: ReadonlySet<string> = new Set([
  'KHR_draco_mesh_compression',
  'EXT_meshopt_compression',
  'KHR_mesh_quantization',
]);

/**
 * Extensions whose only effect is naming an alternate image source, in
 * preference order.
 *
 * Deliberately not part of the two sets above: whether one is honored is a
 * per-consumer question. The SceneDocument carries the bytes whatever the
 * codec, so it always honors them; the preview honors one only when the
 * browser actually decoded every image that came through it.
 */
export const GLTF_TEXTURE_SOURCE_EXTENSIONS = ['EXT_texture_webp', 'KHR_texture_basisu'] as const;

/** glTF sampler defaults, spelled as numbers so this module needs no GL context. */
const DEFAULT_SAMPLER = {
  wrapS: 10497,
  wrapT: 10497,
  minFilter: 9987,
  magFilter: 9729,
} as const;

/** A texture binding in glTF index space, with its transform already read. */
export interface InterpretedTexture {
  index: number;
  texCoord: number;
  transform?: {
    offset: number[];
    scale: number[];
    rotation: number;
    texCoord?: number;
  };
  /** normalTexture only. */
  scale?: number;
  /** occlusionTexture only. */
  strength?: number;
}

/**
 * A material as the document declares it: factors resolved to their defaults,
 * texture slots in glTF index space, extensions folded in.
 *
 * Absent extensions produce the values that reproduce the core
 * metallic-roughness model exactly, so a consumer never needs to ask whether an
 * extension was present.
 */
export interface InterpretedMaterial {
  name: string;
  baseColorFactor: number[];
  metallicFactor: number;
  roughnessFactor: number;
  emissiveFactor: number[];
  emissiveStrength: number;
  ior: number;
  specularFactor: number;
  specularColorFactor: number[];
  clearcoatFactor: number;
  clearcoatRoughnessFactor: number;
  alphaMode: string;
  alphaCutoff: number;
  doubleSided: boolean;
  unlit: boolean;
  baseColorTexture: InterpretedTexture | null;
  metallicRoughnessTexture: InterpretedTexture | null;
  normalTexture: InterpretedTexture | null;
  emissiveTexture: InterpretedTexture | null;
  occlusionTexture: InterpretedTexture | null;
  specularTexture: InterpretedTexture | null;
  specularColorTexture: InterpretedTexture | null;
  clearcoatTexture: InterpretedTexture | null;
  clearcoatRoughnessTexture: InterpretedTexture | null;
  clearcoatNormalTexture: InterpretedTexture | null;
}

/** Read one `materials[]` entry, extensions included. */
export function readGltfMaterial(def: GltfJson, index: number): InterpretedMaterial {
  const material = def || {};
  const pbr = material.pbrMetallicRoughness || {};
  const extensions = material.extensions || {};
  const clearcoat = extensions.KHR_materials_clearcoat || {};
  const specular = extensions.KHR_materials_specular || {};
  return {
    name: material.name || `material_${index}`,
    baseColorFactor: numbers(pbr.baseColorFactor, [1, 1, 1, 1]),
    metallicFactor: pbr.metallicFactor ?? 1,
    roughnessFactor: pbr.roughnessFactor ?? 1,
    emissiveFactor: numbers(material.emissiveFactor, [0, 0, 0]),
    emissiveStrength: extensions.KHR_materials_emissive_strength?.emissiveStrength ?? 1,
    // 1.5 is the index of refraction the core model implies (f0 = 0.04), so a
    // material without KHR_materials_ior is shaded exactly as before.
    ior: extensions.KHR_materials_ior?.ior ?? 1.5,
    specularFactor: specular.specularFactor ?? 1,
    specularColorFactor: numbers(specular.specularColorFactor, [1, 1, 1]),
    // Absent clearcoat means no coat at all, not a coat of default roughness.
    clearcoatFactor: clearcoat.clearcoatFactor ?? 0,
    clearcoatRoughnessFactor: clearcoat.clearcoatRoughnessFactor ?? 0,
    alphaMode: material.alphaMode || 'OPAQUE',
    alphaCutoff: material.alphaCutoff ?? 0.5,
    doubleSided: Boolean(material.doubleSided),
    unlit: Boolean(extensions.KHR_materials_unlit),
    baseColorTexture: readTexture(pbr.baseColorTexture),
    metallicRoughnessTexture: readTexture(pbr.metallicRoughnessTexture),
    normalTexture: readTexture(material.normalTexture, { scale: 1 }),
    emissiveTexture: readTexture(material.emissiveTexture),
    occlusionTexture: readTexture(material.occlusionTexture, { strength: 1 }),
    specularTexture: readTexture(specular.specularTexture),
    specularColorTexture: readTexture(specular.specularColorTexture),
    clearcoatTexture: readTexture(clearcoat.clearcoatTexture),
    clearcoatRoughnessTexture: readTexture(clearcoat.clearcoatRoughnessTexture),
    clearcoatNormalTexture: readTexture(clearcoat.clearcoatNormalTexture, { scale: 1 }),
  };
}

/**
 * Read one texture binding.
 *
 * `KHR_texture_transform` is read on every slot, not just base color: the
 * extension applies wherever a textureInfo does, and a consumer that ignores
 * the transform on a normal map should do so knowingly rather than because the
 * reader never told it.
 */
function readTexture(
  info: GltfJson,
  scalars: { scale?: number; strength?: number } = {},
): InterpretedTexture | null {
  if (!info || !Number.isInteger(info.index)) return null;
  const binding: InterpretedTexture = {
    index: info.index,
    texCoord: info.texCoord ?? 0,
  };
  const transform = info.extensions?.KHR_texture_transform;
  if (transform) {
    // The extension's own texCoord, when present, replaces the slot's.
    binding.texCoord = transform.texCoord ?? binding.texCoord;
    binding.transform = {
      offset: numbers(transform.offset, [0, 0]),
      scale: numbers(transform.scale, [1, 1]),
      rotation: transform.rotation ?? 0,
      ...(transform.texCoord === undefined ? {} : { texCoord: transform.texCoord }),
    };
  }
  if (scalars.scale !== undefined) binding.scale = info.scale ?? scalars.scale;
  if (scalars.strength !== undefined) binding.strength = info.strength ?? scalars.strength;
  return binding;
}

/**
 * Resolve the image a texture reads.
 *
 * @returns The `images[]` index, or -1, and the alternate-source extension
 *   that named it.
 */
export function resolveTextureSource(texture: GltfJson): { source: number; extension: string | null } {
  if (Number.isInteger(texture?.source)) return { source: texture.source, extension: null };
  for (const extension of GLTF_TEXTURE_SOURCE_EXTENSIONS) {
    const source = texture?.extensions?.[extension]?.source;
    if (Number.isInteger(source)) return { source, extension };
  }
  return { source: -1, extension: null };
}

/** Resolve one `samplers[]` entry against the glTF defaults. */
export function resolveSampler(def: GltfJson) {
  return {
    wrapS: def?.wrapS ?? DEFAULT_SAMPLER.wrapS,
    wrapT: def?.wrapT ?? DEFAULT_SAMPLER.wrapT,
    minFilter: def?.minFilter ?? DEFAULT_SAMPLER.minFilter,
    magFilter: def?.magFilter ?? DEFAULT_SAMPLER.magFilter,
  };
}

/**
 * Report the extensions a consumer did not act on.
 *
 * The predicate and the wording both belong to the caller: "ignored by the
 * preview" and "omitted from the portable document" are different statements
 * about the same list, and a user reading one should not be told the other.
 */
export function gltfExtensionWarnings(
  manifest: GltfJson,
  isHonored: (extension: string) => boolean,
  messages: { ignored: (names: string) => string; required: string },
): string[] {
  const warnings: string[] = [];
  const unsupported = (manifest?.extensionsUsed || []).filter((extension: string) => !isHonored(extension));
  if (unsupported.length > 0) warnings.push(messages.ignored(unsupported.join(', ')));
  if ((manifest?.extensionsRequired || []).some((extension: string) => !isHonored(extension))) {
    warnings.push(messages.required);
  }
  return warnings;
}

/** Whether an extension needs no interpretation beyond what both readers do. */
export function isInterpretedGltfExtension(extension: string): boolean {
  return GLTF_INTERPRETED_EXTENSIONS.has(extension) || GLTF_READER_RESOLVED_EXTENSIONS.has(extension);
}

function numbers(value: GltfJson, fallback: number[]): number[] {
  return Array.isArray(value) ? Array.from(value, Number) : [...fallback];
}
