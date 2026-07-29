/**
 * SceneDocument -> viewer runtime adapter.
 *
 * This is intentionally format-neutral. Image decoding and GPU upload remain
 * the viewer's concern; textures retain their source byte resource here so a
 * browser-specific hydration step can be added without leaking DOM handles
 * into SceneDocument.
 */

import {
  componentByteSize, morphDeltaAccessor, normalizeComponent, readComponent,
} from './component-values.ts';
import { cloneTrs, composeTrs, decomposeMat4 } from './mat4.ts';
import {
  MAX_ACTIVE_MORPH_TARGETS, VIEWER_LIMIT_WARNINGS, peakActiveMorphWeights,
} from './viewer-scene.ts';
import type { RuntimeAccessor, ViewerInstancing, ViewerLight } from './viewer-scene.ts';
import { assertValidSceneDocument } from './scene-document.ts';
import {
  MATERIAL_EXTENSION_TEXTURE_SLOTS, materialExtensionFactors,
} from './material-extensions.ts';
import type {
  SceneAccessor,
  SceneAnimation,
  SceneDocument,
  SceneLight,
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
/**
 * Which material a primitive shows under the chosen variant.
 *
 * The document carries every alternative and no selection, because a variant
 * is a choice the viewer makes. A primitive the variant says nothing about
 * keeps its own material, which is what the extension means by an absent
 * mapping.
 */
function selectedMaterial(primitive: ScenePrimitive, variant: number | null): number {
  if (variant !== null && primitive.variantMaterials?.[variant] !== undefined) {
    return primitive.variantMaterials[variant];
  }
  return primitive.material ?? -1;
}

export function buildViewerSceneFromDocument(document: SceneDocument, variant: number | null = null) {
  assertValidSceneDocument(document);
  // Only what the renderer cannot show. The document's own warnings — what the
  // portable form cost the asset — are a different question with a different
  // answer, and the scene report already presents them under their own source.
  // Repeating them here would tell someone looking at a frame that something
  // was "omitted from SceneDocument", which is true of the export and not of
  // what they are looking at.
  const viewerWarnings: string[] = [];
  const accessors = document.accessors.map(toRuntimeAccessor);
  const meshes = document.meshes.map((mesh, meshIndex) => ({
    name: mesh.name || `mesh_${meshIndex}`,
    primitives: mesh.primitives.map((primitive, primitiveIndex) => adaptPrimitive(
      primitive, accessors, viewerWarnings, meshIndex, primitiveIndex, variant,
    )),
    aabb: meshAabb(mesh, accessors),
  }));
  const nodes = document.nodes.map((node) => ({
    ...adaptNode(node, document),
    ...(adaptInstancing(node, accessors) ? { instancing: adaptInstancing(node, accessors)! } : {}),
  }));
  const materials = document.materials.map(adaptMaterial);
  const textures = document.textures.map((texture, textureIndex) => adaptTexture(texture, document.resources, textureIndex));
  const skins = document.skins.map((skin, skinIndex) => adaptSkin(skin, nodes, accessors, skinIndex));

  nodes.forEach((node, index) => {
    const source = document.nodes[index];
    if (source.skin !== undefined) node.skinIndex = source.skin;
  });
  const { renderables, aabb } = collectRenderables(nodes, meshes, document.rootNodes);

  const animations = document.animations.map(
    (clip, clipIndex) => adaptAnimation(clip, nodes, accessors, clipIndex, viewerWarnings),
  );
  reportMorphWeightLimits(document, meshes, nodes, viewerWarnings);
  for (const mesh of document.meshes) for (const primitive of mesh.primitives) {
    if (primitive.attributes.JOINTS_1 !== undefined || primitive.attributes.WEIGHTS_1 !== undefined) {
      viewerWarnings.push('Preview skinning uses the first four influences; additional influence sets remain available to exporters.');
      break;
    }
  }
  // Only lights a node places: an unplaced one has no position to shine from,
  // and the document said so when it read them.
  const lights = document.nodes.flatMap((source, index) => {
    if (source.light === undefined) return [];
    const light = document.lights?.[source.light];
    return light ? [adaptLight(light, nodes[index])] : [];
  });

  return {
    nodes,
    lights,
    rootIndices: [...document.rootNodes],
    meshes,
    skins,
    materials,
    textures,
    animations,
    renderables,
    aabb,
    variants: [...(document.variants ?? [])],
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

function adaptPrimitive(
  primitive: ScenePrimitive,
  accessors: RuntimeAccessor[],
  warnings: string[],
  meshIndex: number,
  primitiveIndex: number,
  variant: number | null,
) {
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
    // Matches the glTF loader's convention: a primitive without a material
    // resolves to nothing, and the renderer falls back to its own defaults.
    // Defaulting to 0 would silently paint it with the first material instead.
    materialIndex: selectedMaterial(primitive, variant),
  };
  if (primitive.indices !== undefined) runtime.indices = accessors[primitive.indices];
  if (primitive.targets?.length) {
    // A SceneDocument keeps morph deltas in the component type they arrived in,
    // because it has to stay a lossless record of the source. The morph texture
    // the preview blends through is float only, so the expansion belongs here —
    // without it a quantized asset silently renders at its rest pose.
    const vertexCount = attributes.POSITION?.count ?? 0;
    const deltas = (
      semantic: 'POSITION' | 'NORMAL',
      report: (target: number, mesh: number, primitive: number) => string,
    ) => primitive.targets!.map((target, index) => {
      const accessorIndex = target[semantic];
      if (accessorIndex === undefined) return null;
      const expanded = morphDeltaAccessor(accessors[accessorIndex], vertexCount);
      if (!expanded) warnings.push(report(index, meshIndex, primitiveIndex));
      return expanded;
    });
    runtime.morphPositions = deltas('POSITION', VIEWER_LIMIT_WARNINGS.morphTarget);
    runtime.morphNormals = deltas('NORMAL', VIEWER_LIMIT_WARNINGS.morphNormal);
    // The document keeps TANGENT deltas because an exporter can still write
    // them; the preview derives its tangent frame from deformed geometry and
    // UVs instead, so it says so rather than dropping them silently.
    if (primitive.targets.some((target) => target.TANGENT !== undefined)) {
      warnings.push(VIEWER_LIMIT_WARNINGS.morphTangents(meshIndex, primitiveIndex));
    }
  }
  return runtime;
}

/**
 * The instance transforms of a node, composed into matrices.
 *
 * Composed once here rather than per frame: EXT_mesh_gpu_instancing has no
 * animation, so the matrices never change after the scene is built. An
 * attribute the node left out takes the identity value for its part, which is
 * what "any subset" means in the extension.
 */
function adaptInstancing(node: SceneNode, accessors: RuntimeAccessor[]): ViewerInstancing | undefined {
  const source = node.instancing;
  if (!source) return undefined;
  const read = (semantic: string, components: number) => {
    const index = source.attributes[semantic];
    void components;
    return index === undefined ? null : floatAccessor(accessors[index]);
  };
  const translations = read('TRANSLATION', 3);
  const rotations = read('ROTATION', 4);
  const scales = read('SCALE', 3);
  const matrices = new Float32Array(source.count * 16);
  for (let index = 0; index < source.count; index += 1) {
    const trs = {
      translation: translations ? [...translations.subarray(index * 3, index * 3 + 3)] : [0, 0, 0],
      rotation: rotations ? [...rotations.subarray(index * 4, index * 4 + 4)] : [0, 0, 0, 1],
      scale: scales ? [...scales.subarray(index * 3, index * 3 + 3)] : [1, 1, 1],
    };
    matrices.set(composeTrs(trs), index * 16);
  }
  return { matrices, count: source.count };
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
  warnings: string[],
) {
  const name = clip.name || `animation_${clipIndex}`;
  return {
    name,
    duration: clip.duration,
    channels: clip.channels.map((channel) => {
      const sampler = clip.samplers[channel.sampler];
      const input = floatAccessor(accessors[sampler.input]);
      const output = floatAccessor(accessors[sampler.output]);
      const targetCount = channel.path === 'weights' ? accessors[sampler.output].components : 3;
      const runtime = { input, output, interpolation: sampler.interpolation || 'LINEAR' };
      if (channel.path === 'weights') {
        const active = peakActiveMorphWeights(runtime, targetCount);
        if (active > MAX_ACTIVE_MORPH_TARGETS) {
          warnings.push(VIEWER_LIMIT_WARNINGS.morphKeyframeWeights(name, active));
        }
      }
      return {
        node: nodes[channel.node],
        path: channel.path,
        targetCount,
        sampler: runtime,
      };
    }),
  };
}

/**
 * Report meshes whose rest pose already drives more targets than one frame
 * blends.
 *
 * The document sizes `node.weights` to the target count on import, so unlike
 * the glTF loader this only has to count them — there is no pose to
 * reconstruct first.
 */
function reportMorphWeightLimits(
  document: SceneDocument,
  meshes: { name: string }[],
  nodes: ReturnType<typeof adaptNode>[],
  warnings: string[],
) {
  nodes.forEach((node, index) => {
    const mesh = meshes[node.meshIndex];
    if (!mesh || document.nodes[index] === undefined) return;
    let active = 0;
    for (const weight of node.weights) if (weight) active += 1;
    if (active > MAX_ACTIVE_MORPH_TARGETS) {
      warnings.push(VIEWER_LIMIT_WARNINGS.morphMeshWeights(mesh.name, active));
    }
  });
}

/**
 * Project a portable material into the flat record the renderer binds.
 *
 * Field for field this must match what the glTF loader produces: the two are
 * the only producers of `ViewerScene.materials`, they feed the same
 * `applyMaterial`, and a field present in one and missing in the other shades
 * the same asset differently depending on which path loaded it.
 */
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
    emissiveStrength: material.emissiveStrength ?? 1,
    emissiveTexture: textureInfo(material.emissiveTexture),
    normalTexture: textureInfo(material.normalTexture),
    occlusionTexture: textureInfo(material.occlusionTexture),
    doubleSided: Boolean(material.doubleSided),
    alphaMode: material.alphaMode || 'OPAQUE',
    alphaCutoff: material.alphaCutoff ?? 0.5,
    // The portable form omits anything the core model already implies, so the
    // defaults come from the table that decided to omit them.
    ...materialExtensionFactors(material),
    ...Object.fromEntries(MATERIAL_EXTENSION_TEXTURE_SLOTS.map((slot) => [
      slot, textureInfo(material[slot as keyof SceneMaterial] as TextureInfo | undefined),
    ])),
  };
}

/**
 * One glTF image is routinely read by many textures — the same map through
 * different sampler settings, or simply the same map on many materials. The
 * document records that faithfully: several textures, one resource. Copying the
 * bytes per texture would undo it, and on a real asset that is tens of
 * megabytes of identical buffers, so the scene points at the document's bytes.
 * Nothing on this path writes to them.
 */
/**
 * A document light bound to the node that places it.
 *
 * Every optional field is resolved here rather than in the shader: the
 * renderer sends fixed-size arrays, and "absent" has no representation in
 * them. A range of zero is the extension's infinite - the light never stops.
 */
function adaptLight(light: SceneLight, node: ReturnType<typeof adaptNode>): ViewerLight {
  return {
    type: light.type,
    node,
    color: [...(light.color || [1, 1, 1])],
    intensity: light.intensity ?? 1,
    range: light.range ?? 0,
    innerConeAngle: light.innerConeAngle ?? 0,
    outerConeAngle: light.outerConeAngle ?? Math.PI / 4,
  };
}

function adaptTexture(texture: SceneTexture, resources: SceneResource[], index: number) {
  const resource = resources[texture.resource];
  return {
    name: texture.name || resource.name || `texture_${index}`,
    resource: texture.resource,
    mimeType: resource.mimeType,
    bytes: resource.bytes,
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

/**
 * An accessor as floats, whatever it was stored as.
 *
 * Reading the bytes as float32 outright was wrong for every accessor the
 * format allows to be smaller: EXT_mesh_gpu_instancing states ROTATION as
 * normalized BYTE or SHORT as readily as FLOAT, and a skin's inverse bind
 * matrices go through here too. The stored width decides the stride, and
 * `normalized` decides whether an integer means itself or a fraction of its
 * range.
 */
function floatAccessor(accessor: RuntimeAccessor): Float32Array {
  const view = new DataView(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength);
  const width = componentByteSize(accessor.componentType);
  const values = new Float32Array(accessor.count * accessor.components);
  for (let index = 0; index < values.length; index += 1) {
    const value = readComponent(view, index * width, accessor.componentType);
    values[index] = accessor.normalized ? normalizeComponent(value, accessor.componentType) : value;
  }
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
    // Through the same reader the rest of the file uses, because
    // KHR_mesh_quantization stores POSITION as a normalized SHORT just as
    // readily as a float. Read raw, ShaderBall.glb measured 32767 units
    // across instead of two, and the camera framed the box it was given: the
    // model drew at its true size, a speck at the origin of an empty
    // viewport.
    const values = floatAccessor(accessor);
    for (let index = 0; index < values.length; index += 3) {
      for (let component = 0; component < 3; component += 1) {
        aabb.min[component] = Math.min(aabb.min[component], values[index + component]);
        aabb.max[component] = Math.max(aabb.max[component], values[index + component]);
      }
    }
  }
  return Number.isFinite(aabb.min[0]) ? aabb : { min: [-0.5, -0.5, -0.5], max: [0.5, 0.5, 0.5] };
}

/**
 * Walk the scene from its roots, collecting what is drawn and how big it is.
 *
 * From the roots rather than over every node, because a document may hold
 * nodes no scene reaches — a mesh left behind by an authoring tool draws
 * nothing in glTF and must not widen the frame here either. The glTF loader
 * has always walked this way; the two rules agreed on every asset in the
 * corpus, which is exactly why the difference could sit here unnoticed.
 *
 * `visited` guards against a cycle in `children`. A document that reaches the
 * viewer has passed validation, but this walk is cheap insurance against
 * hanging the tab over a malformed one.
 */
function collectRenderables(
  nodes: ReturnType<typeof adaptNode>[],
  meshes: { aabb: { min: number[]; max: number[] } }[],
  rootNodes: number[],
) {
  const aabb = { min: [Infinity, Infinity, Infinity], max: [-Infinity, -Infinity, -Infinity] };
  const renderables: { node: typeof nodes[number]; meshIndex: number; skinIndex: number }[] = [];
  const visited = new Set<number>();

  const walk = (nodeIndex: number) => {
    if (visited.has(nodeIndex)) return;
    visited.add(nodeIndex);
    const node = nodes[nodeIndex];
    if (!node) return;
    const mesh = meshes[node.meshIndex];
    if (node.meshIndex >= 0 && mesh) {
      renderables.push({ node, meshIndex: node.meshIndex, skinIndex: node.skinIndex });
      for (let component = 0; component < 3; component += 1) {
        aabb.min[component] = Math.min(aabb.min[component], mesh.aabb.min[component]);
        aabb.max[component] = Math.max(aabb.max[component], mesh.aabb.max[component]);
      }
    }
    for (const child of node.children) walk(child);
  };
  for (const root of rootNodes) walk(root);

  return {
    renderables,
    aabb: Number.isFinite(aabb.min[0]) ? aabb : { min: [-0.5, -0.5, -0.5], max: [0.5, 0.5, 0.5] },
  };
}
