/**
 * Portable, source-neutral scene contract.
 *
 * This module intentionally contains only structured data and typed byte
 * arrays. It must not retain parser objects, browser images, WebGL resources,
 * or WASM handles. Format importers adapt their own semantics into this
 * contract; renderers and exporters consume it without knowing the source
 * format.
 */

import { componentByteWidth } from './component-values.ts';
import { MATERIAL_EXTENSION_DEFAULTS, MATERIAL_EXTENSION_TEXTURE_SLOTS } from './material-extensions.ts';
import type { MaterialExtensionValues } from './material-extensions.ts';

export const SCENE_DOCUMENT_VERSION = 1;

/** glTF component type enum; the only widths this contract carries. */
export type ComponentType = 5120 | 5121 | 5122 | 5123 | 5125 | 5126;

export type AlphaMode = 'OPAQUE' | 'MASK' | 'BLEND';
export type Interpolation = 'STEP' | 'LINEAR' | 'CUBICSPLINE';
export type AnimationPath = 'translation' | 'rotation' | 'scale' | 'weights';

export interface SceneResource {
  mimeType: string;
  bytes: Uint8Array;
  name?: string;
}

export interface TextureSampler {
  wrapS?: number;
  wrapT?: number;
  minFilter?: number;
  magFilter?: number;
}

export interface SceneTexture {
  resource: number;
  name?: string;
  sampler?: TextureSampler;
}

export interface TextureTransform {
  offset?: number[];
  scale?: number[];
  rotation?: number;
  texCoord?: number;
}

export interface TextureInfo {
  texture: number;
  texCoord?: number;
  transform?: TextureTransform;
  /** Normal map scale; only meaningful on a normalTexture. */
  scale?: number;
  /** Occlusion strength; only meaningful on an occlusionTexture. */
  strength?: number;
}

/**
 * The portable material.
 *
 * Beyond core metallic-roughness it carries the layered extensions both the
 * preview and the glTF writer understand. Every one of those is optional and
 * absence must mean the value that reproduces the core model exactly, so a
 * document from a format that has no such concept — FBX — is unchanged by
 * their existence.
 */
export interface SceneMaterial extends MaterialExtensionValues<TextureInfo> {
  name?: string;
  baseColorFactor?: number[];
  metallicFactor?: number;
  roughnessFactor?: number;
  emissiveFactor?: number[];
  baseColorTexture?: TextureInfo;
  metallicRoughnessTexture?: TextureInfo;
  normalTexture?: TextureInfo;
  emissiveTexture?: TextureInfo;
  occlusionTexture?: TextureInfo;
  alphaMode?: AlphaMode;
  alphaCutoff?: number;
  doubleSided?: boolean;
}

/**
 * Every texture slot a material can bind, in one place.
 *
 * Validation, the glTF writer and the texture-transform survey all walk this
 * list; spelled out separately in each, a new slot reaches one of them and not
 * the others. Only the core slots are named here — the rest come from the
 * extension table, so a new layered extension reaches all three by being
 * declared once.
 */
export const MATERIAL_TEXTURE_SLOTS = [
  'baseColorTexture',
  'metallicRoughnessTexture',
  'normalTexture',
  'emissiveTexture',
  'occlusionTexture',
  ...MATERIAL_EXTENSION_TEXTURE_SLOTS,
] as const;

export interface SceneAccessor {
  bytes: Uint8Array;
  componentType: ComponentType;
  components: number;
  count: number;
  normalized?: boolean;
  min?: number[];
  max?: number[];
}

/** Attribute semantic -> accessor index. */
export type AttributeMap = Record<string, number>;

export interface ScenePrimitive {
  attributes: AttributeMap;
  indices?: number;
  material?: number;
  mode?: number;
  targets?: AttributeMap[];
}

export interface SceneMesh {
  name?: string;
  primitives: ScenePrimitive[];
  weights?: number[];
}

export interface SceneNode {
  name?: string;
  /** Mutually exclusive with translation/rotation/scale. */
  matrix?: number[];
  translation?: number[];
  rotation?: number[];
  scale?: number[];
  mesh?: number;
  skin?: number;
  weights?: number[];
  children?: number[];
}

export interface SceneSkin {
  name?: string;
  joints: number[];
  inverseBindMatrices?: number;
  skeleton?: number;
}

export interface AnimationSampler {
  input: number;
  output: number;
  interpolation?: Interpolation;
}

export interface AnimationChannel {
  sampler: number;
  node: number;
  path: AnimationPath;
}

export interface SceneAnimation {
  name?: string;
  duration: number;
  samplers: AnimationSampler[];
  channels: AnimationChannel[];
}

export interface SceneDocument {
  version: number;
  resources: SceneResource[];
  textures: SceneTexture[];
  materials: SceneMaterial[];
  accessors: SceneAccessor[];
  meshes: SceneMesh[];
  nodes: SceneNode[];
  rootNodes: number[];
  skins: SceneSkin[];
  animations: SceneAnimation[];
  warnings: string[];
}

/** What a document actually exercises, for exporters that must gate features. */
export interface SceneCapabilities {
  resources: boolean;
  textures: boolean;
  materials: boolean;
  skins: boolean;
  morphTargets: boolean;
  animations: boolean;
  matrixNodes: boolean;
  cubicAnimation: boolean;
  maxSkinJoints: number;
  maxMorphTargets: number;
}

export interface ValidationResult {
  ok: boolean;
  errors: string[];
  warnings: string[];
  capabilities: SceneCapabilities;
}

/**
 * The validators below exist precisely because their input is untrusted, so
 * they walk it as loosely typed data and report what is wrong instead of
 * assuming the declared shape. Only the exported entry points narrow.
 */
type Untrusted = any;

const ATTRIBUTE_COMPONENTS = new Map<string, number>([
  ['POSITION', 3], ['NORMAL', 3], ['TANGENT', 4], ['TEXCOORD_0', 2],
  ['TEXCOORD_1', 2], ['TEXCOORD_2', 2], ['TEXCOORD_3', 2],
  ['TEXCOORD_4', 2], ['TEXCOORD_5', 2], ['TEXCOORD_6', 2], ['TEXCOORD_7', 2],
  ['COLOR_0', 3], ['JOINTS_0', 4], ['WEIGHTS_0', 4],
  ['JOINTS_1', 4], ['WEIGHTS_1', 4],
]);

const ANIMATION_PATHS = new Set(['translation', 'rotation', 'scale', 'weights']);
const INTERPOLATIONS = new Set(['STEP', 'LINEAR', 'CUBICSPLINE']);

/** Triangles a primitive of `mode` draws from `elementCount` vertices or indices. */
export function triangleCountForMode(mode: number, elementCount: number) {
  switch (mode) {
    case 4: // TRIANGLES
      return Math.floor(elementCount / 3);
    case 5: // TRIANGLE_STRIP
    case 6: // TRIANGLE_FAN
      return Math.max(0, elementCount - 2);
    default:
      return 0;
  }
}

/**
 * Count what the summary panel reports about a document's geometry.
 *
 * A pure query over the document rather than over the source file: the same
 * numbers used to come from a second full walk of the asset, opened only to
 * recount what the document already holds.
 */
export function summarizeSceneDocumentGeometry(document: SceneDocument) {
  let vertexCount = 0;
  let triangleCount = 0;
  let hasNormals = false;
  let hasUvs = false;
  for (const mesh of document.meshes) {
    for (const primitive of mesh.primitives) {
      const position = document.accessors[primitive.attributes.POSITION];
      const indices = primitive.indices === undefined ? undefined : document.accessors[primitive.indices];
      vertexCount += position?.count ?? 0;
      triangleCount += triangleCountForMode(primitive.mode ?? 4, indices?.count ?? position?.count ?? 0);
      for (const semantic of Object.keys(primitive.attributes)) {
        if (semantic === 'NORMAL') hasNormals = true;
        else if (semantic.startsWith('TEXCOORD_')) hasUvs = true;
      }
    }
  }
  return { vertexCount, triangleCount, hasNormals, hasUvs };
}

/** Return an empty, transferable SceneDocument. */
export function createSceneDocument(overrides: Partial<SceneDocument> = {}): SceneDocument {
  return {
    version: SCENE_DOCUMENT_VERSION,
    resources: [],
    textures: [],
    materials: [],
    accessors: [],
    meshes: [],
    nodes: [],
    rootNodes: [],
    skins: [],
    animations: [],
    warnings: [],
    ...overrides,
  };
}

/**
 * Validate the portable scene contract without mutating it.
 *
 * `errors` are structural violations. `warnings` describe valid data that a
 * later glTF exporter or renderer may need to bake, limit, or reject under its
 * own capability policy.
 */
export function validateSceneDocument(input: unknown): ValidationResult {
  const document: Untrusted = input;
  const errors: string[] = [];
  const warnings: string[] = [];
  const capabilities = emptyCapabilities();
  if (!document || typeof document !== 'object') {
    return { ok: false, errors: ['SceneDocument must be an object'], warnings, capabilities };
  }
  if (document.version !== SCENE_DOCUMENT_VERSION) {
    errors.push(`Unsupported SceneDocument version ${String(document.version)}`);
  }
  for (const field of ['resources', 'textures', 'materials', 'accessors', 'meshes', 'nodes', 'rootNodes', 'skins', 'animations', 'warnings']) {
    if (!Array.isArray(document[field])) errors.push(`SceneDocument.${field} must be an array`);
  }
  if (errors.length > 0) return { ok: false, errors, warnings, capabilities };

  capabilities.resources = document.resources.length > 0;
  capabilities.textures = document.textures.length > 0;
  capabilities.materials = document.materials.length > 0;
  document.resources.forEach((resource: Untrusted, index: number) => validateResource(resource, index, errors));
  document.textures.forEach((texture: Untrusted, index: number) => validateTexture(texture, index, document.resources.length, errors));
  document.materials.forEach((material: Untrusted, index: number) => validateMaterial(material, index, document.textures.length, errors));
  document.accessors.forEach((accessor: Untrusted, index: number) => validateAccessor(accessor, index, errors));

  document.meshes.forEach((mesh: Untrusted, meshIndex: number) => {
    if (!mesh || typeof mesh !== 'object' || !Array.isArray(mesh.primitives) || mesh.primitives.length === 0) {
      errors.push(`meshes[${meshIndex}] must contain at least one primitive`);
      return;
    }
    if (mesh.weights !== undefined) validateWeights(mesh.weights, `meshes[${meshIndex}].weights`, errors);
    mesh.primitives.forEach((primitive: Untrusted, primitiveIndex: number) => {
      validatePrimitive(primitive, `meshes[${meshIndex}].primitives[${primitiveIndex}]`, document, errors, warnings, capabilities);
    });
  });

  const parentCount = new Uint32Array(document.nodes.length);
  document.nodes.forEach((node: Untrusted, nodeIndex: number) => {
    validateNode(node, nodeIndex, document, parentCount, errors, warnings, capabilities);
  });
  parentCount.forEach((count, nodeIndex) => {
    if (count > 1) errors.push(`nodes[${nodeIndex}] has multiple parents`);
  });
  document.rootNodes.forEach((nodeIndex: Untrusted, rootIndex: number) => {
    validateIndex(nodeIndex, document.nodes.length, `rootNodes[${rootIndex}]`, errors);
    if (Number.isInteger(nodeIndex) && nodeIndex >= 0 && nodeIndex < parentCount.length && parentCount[nodeIndex] > 0) {
      errors.push(`rootNodes[${rootIndex}] also has a parent`);
    }
  });
  validateNodeCycles(document.nodes, errors);

  document.skins.forEach((skin: Untrusted, index: number) => validateSkin(skin, index, document, errors, capabilities));
  document.animations.forEach((animation: Untrusted, index: number) => validateAnimation(animation, index, document, errors, warnings, capabilities));
  document.warnings.forEach((warning: Untrusted, index: number) => {
    if (typeof warning !== 'string') errors.push(`warnings[${index}] must be a string`);
  });

  return { ok: errors.length === 0, errors, warnings, capabilities };
}

/**
 * Throw a concise Error when a SceneDocument is structurally invalid.
 *
 * Callers read the returned capabilities and warnings, so this cannot be an
 * `asserts` signature: TypeScript forbids assertion functions from returning a
 * value. Narrowing therefore happens where a caller declares a SceneDocument
 * parameter, with this call as the runtime guarantee behind it.
 */
export function assertValidSceneDocument(document: unknown): ValidationResult {
  const result = validateSceneDocument(document);
  if (!result.ok) throw new Error(`Invalid SceneDocument: ${result.errors.join('; ')}`);
  return result;
}

/** Copy typed resources/accessors so the caller can transfer or retain safely. */
export function cloneSceneDocument(document: SceneDocument): SceneDocument {
  assertValidSceneDocument(document);
  return {
    ...document,
    resources: document.resources.map((resource) => ({ ...resource, bytes: new Uint8Array(resource.bytes) })),
    textures: document.textures.map((texture) => ({ ...texture, sampler: texture.sampler && { ...texture.sampler } })),
    materials: document.materials.map((material) => structuredClone(material)),
    accessors: document.accessors.map((accessor) => ({ ...accessor, bytes: new Uint8Array(accessor.bytes), min: accessor.min && [...accessor.min], max: accessor.max && [...accessor.max] })),
    meshes: structuredClone(document.meshes),
    nodes: structuredClone(document.nodes),
    rootNodes: [...document.rootNodes],
    skins: structuredClone(document.skins),
    animations: structuredClone(document.animations),
    warnings: [...document.warnings],
  };
}

/** Report transferable ArrayBuffers without retaining browser/runtime handles. */
export function sceneDocumentTransferables(document: SceneDocument): ArrayBufferLike[] {
  assertValidSceneDocument(document);
  const buffers = new Set<ArrayBufferLike>();
  for (const resource of document.resources) buffers.add(resource.bytes.buffer);
  for (const accessor of document.accessors) buffers.add(accessor.bytes.buffer);
  return [...buffers];
}

function emptyCapabilities(): SceneCapabilities {
  return {
    resources: false,
    textures: false,
    materials: false,
    skins: false,
    morphTargets: false,
    animations: false,
    matrixNodes: false,
    cubicAnimation: false,
    maxSkinJoints: 0,
    maxMorphTargets: 0,
  };
}

function validateResource(resource: Untrusted, index: number, errors: string[]) {
  const label = `resources[${index}]`;
  if (!resource || typeof resource !== 'object') return errors.push(`${label} must be an object`);
  if (typeof resource.mimeType !== 'string' || resource.mimeType.length === 0) errors.push(`${label}.mimeType must be a non-empty string`);
  if (!isBytes(resource.bytes)) errors.push(`${label}.bytes must be a Uint8Array`);
  if (resource.name !== undefined && typeof resource.name !== 'string') errors.push(`${label}.name must be a string when present`);
}

function validateTexture(texture: Untrusted, index: number, resourceCount: number, errors: string[]) {
  const label = `textures[${index}]`;
  if (!texture || typeof texture !== 'object') return errors.push(`${label} must be an object`);
  validateIndex(texture.resource, resourceCount, `${label}.resource`, errors);
  if (texture.name !== undefined && typeof texture.name !== 'string') errors.push(`${label}.name must be a string when present`);
  if (texture.sampler !== undefined && (!texture.sampler || typeof texture.sampler !== 'object')) errors.push(`${label}.sampler must be an object when present`);
}

function validateMaterial(material: Untrusted, index: number, textureCount: number, errors: string[]) {
  const label = `materials[${index}]`;
  if (!material || typeof material !== 'object') return errors.push(`${label} must be an object`);
  validateNumberArray(material.baseColorFactor, 4, `${label}.baseColorFactor`, errors, [1, 1, 1, 1]);
  validateFiniteNumber(material.metallicFactor, `${label}.metallicFactor`, errors, 1);
  validateFiniteNumber(material.roughnessFactor, `${label}.roughnessFactor`, errors, 1);
  validateNumberArray(material.emissiveFactor, 3, `${label}.emissiveFactor`, errors, [0, 0, 0]);
  // The layered extension fields and what absence means for each come from
  // the one table the readers and writers share.
  for (const [property, fallback] of Object.entries(MATERIAL_EXTENSION_DEFAULTS)) {
    if (typeof fallback === 'boolean') continue;
    if (Array.isArray(fallback)) {
      validateNumberArray(material[property], fallback.length, `${label}.${property}`, errors, fallback);
    } else {
      validateFiniteNumber(material[property], `${label}.${property}`, errors, fallback);
    }
  }
  for (const key of MATERIAL_TEXTURE_SLOTS) {
    if (material[key] !== undefined && material[key] !== null) validateTextureInfo(material[key], `${label}.${key}`, textureCount, errors);
  }
  if (material.alphaMode !== undefined && !['OPAQUE', 'MASK', 'BLEND'].includes(material.alphaMode)) errors.push(`${label}.alphaMode is invalid`);
}

function validateTextureInfo(info: Untrusted, label: string, textureCount: number, errors: string[]) {
  if (!info || typeof info !== 'object') return errors.push(`${label} must be an object`);
  validateIndex(info.texture, textureCount, `${label}.texture`, errors);
  if (info.texCoord !== undefined && (!Number.isInteger(info.texCoord) || info.texCoord < 0)) errors.push(`${label}.texCoord must be a non-negative integer`);
  if (info.transform !== undefined) {
    if (!info.transform || typeof info.transform !== 'object') errors.push(`${label}.transform must be an object when present`);
    else {
      validateNumberArray(info.transform.offset, 2, `${label}.transform.offset`, errors, [0, 0]);
      validateNumberArray(info.transform.scale, 2, `${label}.transform.scale`, errors, [1, 1]);
      validateFiniteNumber(info.transform.rotation, `${label}.transform.rotation`, errors, 0);
      if (info.transform.texCoord !== undefined && (!Number.isInteger(info.transform.texCoord) || info.transform.texCoord < 0)) errors.push(`${label}.transform.texCoord must be a non-negative integer`);
    }
  }
  if (info.scale !== undefined) validateFiniteNumber(info.scale, `${label}.scale`, errors);
  if (info.strength !== undefined) validateFiniteNumber(info.strength, `${label}.strength`, errors);
}

function validateAccessor(accessor: Untrusted, index: number, errors: string[]) {
  const label = `accessors[${index}]`;
  if (!accessor || typeof accessor !== 'object') return errors.push(`${label} must be an object`);
  if (!isBytes(accessor.bytes)) errors.push(`${label}.bytes must be a Uint8Array`);
  const componentBytes = componentByteWidth(accessor.componentType);
  if (!componentBytes) errors.push(`${label}.componentType is unsupported`);
  if (!Number.isInteger(accessor.components) || accessor.components <= 0) errors.push(`${label}.components must be a positive integer`);
  if (!Number.isInteger(accessor.count) || accessor.count < 0) errors.push(`${label}.count must be a non-negative integer`);
  if (componentBytes && Number.isInteger(accessor.components) && Number.isInteger(accessor.count) && isBytes(accessor.bytes)) {
    const expected = componentBytes * accessor.components * accessor.count;
    if (accessor.bytes.byteLength !== expected) errors.push(`${label}.bytes length ${accessor.bytes.byteLength} does not match ${expected}`);
  }
  if (accessor.normalized !== undefined && typeof accessor.normalized !== 'boolean') errors.push(`${label}.normalized must be boolean when present`);
  if (accessor.min !== undefined) validateNumberArray(accessor.min, accessor.components, `${label}.min`, errors);
  if (accessor.max !== undefined) validateNumberArray(accessor.max, accessor.components, `${label}.max`, errors);
}

function validatePrimitive(
  primitive: Untrusted,
  label: string,
  document: Untrusted,
  errors: string[],
  warnings: string[],
  capabilities: SceneCapabilities,
) {
  if (!primitive || typeof primitive !== 'object') return errors.push(`${label} must be an object`);
  if (primitive.mode !== undefined && primitive.mode !== 4) warnings.push(`${label}.mode=${primitive.mode} will require triangulation before glTF export`);
  if (!primitive.attributes || typeof primitive.attributes !== 'object') {
    errors.push(`${label}.attributes must be an object`);
    return;
  }
  const position = primitive.attributes.POSITION;
  validateIndex(position, document.accessors.length, `${label}.attributes.POSITION`, errors);
  if (Number.isInteger(position) && document.accessors[position] && document.accessors[position].components !== 3) errors.push(`${label}.attributes.POSITION must use a vec3 accessor`);
  for (const [semantic, accessorIndex] of Object.entries<Untrusted>(primitive.attributes)) {
    validateIndex(accessorIndex, document.accessors.length, `${label}.attributes.${semantic}`, errors);
    const expected = ATTRIBUTE_COMPONENTS.get(semantic);
    const components = Number.isInteger(accessorIndex) ? document.accessors[accessorIndex]?.components : undefined;
    const validColor = semantic === 'COLOR_0' && (components === 3 || components === 4);
    if (expected && components !== undefined && components !== expected && !validColor) {
      errors.push(`${label}.attributes.${semantic} must use ${expected} components`);
    }
  }
  if (primitive.indices !== undefined) validateIndex(primitive.indices, document.accessors.length, `${label}.indices`, errors);
  if (primitive.material !== undefined) validateIndex(primitive.material, document.materials.length, `${label}.material`, errors);
  if (primitive.targets !== undefined) {
    if (!Array.isArray(primitive.targets)) {
      errors.push(`${label}.targets must be an array`);
    } else {
      capabilities.morphTargets = capabilities.morphTargets || primitive.targets.length > 0;
      capabilities.maxMorphTargets = Math.max(capabilities.maxMorphTargets, primitive.targets.length);
      primitive.targets.forEach((target: Untrusted, targetIndex: number) => {
        if (!target || typeof target !== 'object' || Object.keys(target).length === 0) errors.push(`${label}.targets[${targetIndex}] must contain attributes`);
        else for (const [semantic, accessorIndex] of Object.entries(target)) {
          if (!['POSITION', 'NORMAL', 'TANGENT'].includes(semantic)) warnings.push(`${label}.targets[${targetIndex}].${semantic} is not in the portable morph subset`);
          validateIndex(accessorIndex, document.accessors.length, `${label}.targets[${targetIndex}].${semantic}`, errors);
        }
      });
    }
  }
}

function validateNode(
  node: Untrusted,
  index: number,
  document: Untrusted,
  parentCount: Uint32Array,
  errors: string[],
  warnings: string[],
  capabilities: SceneCapabilities,
) {
  const label = `nodes[${index}]`;
  if (!node || typeof node !== 'object') return errors.push(`${label} must be an object`);
  const hasMatrix = node.matrix !== undefined;
  const hasTrs = node.translation !== undefined || node.rotation !== undefined || node.scale !== undefined;
  if (hasMatrix && hasTrs) errors.push(`${label} must use either matrix or TRS, not both`);
  if (hasMatrix) {
    validateNumberArray(node.matrix, 16, `${label}.matrix`, errors);
    capabilities.matrixNodes = true;
    warnings.push(`${label} uses a matrix local transform; animated matrix nodes must be baked to TRS for GLB export`);
  } else {
    validateNumberArray(node.translation, 3, `${label}.translation`, errors, [0, 0, 0]);
    validateQuaternion(node.rotation, `${label}.rotation`, errors);
    validateNumberArray(node.scale, 3, `${label}.scale`, errors, [1, 1, 1]);
  }
  if (node.mesh !== undefined) validateIndex(node.mesh, document.meshes.length, `${label}.mesh`, errors);
  if (node.skin !== undefined) validateIndex(node.skin, document.skins.length, `${label}.skin`, errors);
  if (node.weights !== undefined) validateWeights(node.weights, `${label}.weights`, errors);
  if (node.children !== undefined) {
    if (!Array.isArray(node.children)) errors.push(`${label}.children must be an array`);
    else node.children.forEach((child: Untrusted, childIndex: number) => {
      validateIndex(child, document.nodes.length, `${label}.children[${childIndex}]`, errors);
      if (Number.isInteger(child) && child >= 0 && child < parentCount.length) parentCount[child] += 1;
    });
  }
}

function validateNodeCycles(nodes: Untrusted[], errors: string[]) {
  const visiting = new Set<number>();
  const visited = new Set<number>();
  const visit = (index: number) => {
    if (visited.has(index)) return;
    if (visiting.has(index)) {
      errors.push(`nodes[${index}] participates in a hierarchy cycle`);
      return;
    }
    visiting.add(index);
    for (const child of nodes[index]?.children || []) if (Number.isInteger(child) && child >= 0 && child < nodes.length) visit(child);
    visiting.delete(index);
    visited.add(index);
  };
  nodes.forEach((_, index) => visit(index));
}

function validateSkin(
  skin: Untrusted,
  index: number,
  document: Untrusted,
  errors: string[],
  capabilities: SceneCapabilities,
) {
  const label = `skins[${index}]`;
  if (!skin || typeof skin !== 'object' || !Array.isArray(skin.joints) || skin.joints.length === 0) {
    errors.push(`${label}.joints must be a non-empty array`);
    return;
  }
  capabilities.skins = true;
  capabilities.maxSkinJoints = Math.max(capabilities.maxSkinJoints, skin.joints.length);
  skin.joints.forEach((joint: Untrusted, jointIndex: number) => validateIndex(joint, document.nodes.length, `${label}.joints[${jointIndex}]`, errors));
  if (skin.inverseBindMatrices !== undefined) {
    validateIndex(skin.inverseBindMatrices, document.accessors.length, `${label}.inverseBindMatrices`, errors);
    const accessor = document.accessors[skin.inverseBindMatrices];
    if (accessor && (accessor.components !== 16 || accessor.count !== skin.joints.length)) errors.push(`${label}.inverseBindMatrices must contain one mat4 per joint`);
  }
  if (skin.skeleton !== undefined) validateIndex(skin.skeleton, document.nodes.length, `${label}.skeleton`, errors);
}

function validateAnimation(
  animation: Untrusted,
  index: number,
  document: Untrusted,
  errors: string[],
  warnings: string[],
  capabilities: SceneCapabilities,
) {
  const label = `animations[${index}]`;
  if (!animation || typeof animation !== 'object' || !Array.isArray(animation.samplers) || !Array.isArray(animation.channels)) {
    errors.push(`${label} must contain samplers and channels arrays`);
    return;
  }
  capabilities.animations = capabilities.animations || animation.channels.length > 0;
  validateFiniteNumber(animation.duration, `${label}.duration`, errors, 0);
  animation.samplers.forEach((sampler: Untrusted, samplerIndex: number) => {
    const samplerLabel = `${label}.samplers[${samplerIndex}]`;
    if (!sampler || typeof sampler !== 'object') return errors.push(`${samplerLabel} must be an object`);
    validateIndex(sampler.input, document.accessors.length, `${samplerLabel}.input`, errors);
    validateIndex(sampler.output, document.accessors.length, `${samplerLabel}.output`, errors);
    const interpolation = sampler.interpolation || 'LINEAR';
    if (!INTERPOLATIONS.has(interpolation)) errors.push(`${samplerLabel}.interpolation is invalid`);
    if (interpolation === 'CUBICSPLINE') capabilities.cubicAnimation = true;
    const input = document.accessors[sampler.input];
    const output = document.accessors[sampler.output];
    if (input && (input.componentType !== 5126 || input.components !== 1)) errors.push(`${samplerLabel}.input must be float seconds`);
    if (output && output.componentType !== 5126) errors.push(`${samplerLabel}.output must be float values`);
    if (input && output) {
      const multiplier = interpolation === 'CUBICSPLINE' ? 3 : 1;
      if (output.count !== input.count * multiplier) errors.push(`${samplerLabel}.output count does not match key count/interpolation`);
    }
  });
  animation.channels.forEach((channel: Untrusted, channelIndex: number) => {
    const channelLabel = `${label}.channels[${channelIndex}]`;
    if (!channel || typeof channel !== 'object') return errors.push(`${channelLabel} must be an object`);
    validateIndex(channel.sampler, animation.samplers.length, `${channelLabel}.sampler`, errors);
    validateIndex(channel.node, document.nodes.length, `${channelLabel}.node`, errors);
    if (!ANIMATION_PATHS.has(channel.path)) errors.push(`${channelLabel}.path is invalid`);
    const sampler = animation.samplers[channel.sampler];
    const output = sampler && document.accessors[sampler.output];
    const expected = channel.path === 'rotation' ? 4 : channel.path === 'weights' ? undefined : 3;
    if (expected && output && output.components !== expected) errors.push(`${channelLabel} output must use ${expected} components`);
    if (channel.path === 'weights' && output && output.components < 1) errors.push(`${channelLabel} weight output must have components`);
    if (document.nodes[channel.node]?.matrix) warnings.push(`${channelLabel} targets a matrix node and requires TRS baking for GLB export`);
  });
}

function validateIndex(value: Untrusted, length: number, label: string, errors: string[]) {
  if (!Number.isInteger(value) || value < 0 || value >= length) errors.push(`${label} must reference an existing index`);
}

function validateFiniteNumber(value: Untrusted, label: string, errors: string[], defaultValue?: number) {
  if (value === undefined && defaultValue !== undefined) return;
  if (!Number.isFinite(value)) errors.push(`${label} must be finite`);
}

function validateNumberArray(
  values: Untrusted,
  expectedLength: number,
  label: string,
  errors: string[],
  defaultValue?: number[],
) {
  if (values === undefined && defaultValue !== undefined) return;
  if (!Array.isArray(values) || values.length !== expectedLength || values.some((value) => !Number.isFinite(value))) {
    errors.push(`${label} must contain ${expectedLength} finite numbers`);
  }
}

function validateQuaternion(values: Untrusted, label: string, errors: string[]) {
  if (values === undefined) return;
  validateNumberArray(values, 4, label, errors);
  if (Array.isArray(values) && values.length === 4 && values.every(Number.isFinite) && Math.hypot(...values) < 1e-8) errors.push(`${label} must not be zero-length`);
}

function validateWeights(values: Untrusted, label: string, errors: string[]) {
  if (!Array.isArray(values) || values.some((value) => !Number.isFinite(value))) errors.push(`${label} must be an array of finite numbers`);
}

function isBytes(value: Untrusted): value is Uint8Array {
  return value instanceof Uint8Array;
}
