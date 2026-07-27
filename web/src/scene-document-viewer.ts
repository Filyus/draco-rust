/**
 * SceneDocument -> viewer runtime adapter.
 *
 * This is intentionally format-neutral. Image decoding and GPU upload remain
 * the viewer's concern; textures retain their source byte resource here so a
 * browser-specific hydration step can be added without leaking DOM handles
 * into SceneDocument.
 */

import { componentByteSize, morphDeltaAccessor, readComponent } from './component-values.ts';
import { cloneTrs, decomposeMat4 } from './mat4.ts';
import {
  MAX_ACTIVE_MORPH_TARGETS, VIEWER_LIMIT_WARNINGS, peakActiveMorphWeights,
} from './viewer-scene.ts';
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
  const viewerWarnings = [...document.warnings, ...validation.warnings];
  const accessors = document.accessors.map(toRuntimeAccessor);
  const meshes = document.meshes.map((mesh, meshIndex) => ({
    name: mesh.name || `mesh_${meshIndex}`,
    primitives: mesh.primitives.map((primitive, primitiveIndex) => adaptPrimitive(
      primitive, accessors, viewerWarnings, meshIndex, primitiveIndex,
    )),
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
  return {
    nodes,
    rootIndices: [...document.rootNodes],
    meshes,
    skins,
    materials,
    textures,
    animations,
    renderables,
    aabb,
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
    materialIndex: primitive.material ?? -1,
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
    ior: material.ior ?? 1.5,
    specularFactor: material.specularFactor ?? 1,
    specularColorFactor: [...(material.specularColorFactor || [1, 1, 1])],
    specularTexture: textureInfo(material.specularTexture),
    specularColorTexture: textureInfo(material.specularColorTexture),
    clearcoatFactor: material.clearcoatFactor ?? 0,
    clearcoatRoughnessFactor: material.clearcoatRoughnessFactor ?? 0,
    clearcoatTexture: textureInfo(material.clearcoatTexture),
    clearcoatRoughnessTexture: textureInfo(material.clearcoatRoughnessTexture),
    clearcoatNormalTexture: textureInfo(material.clearcoatNormalTexture),
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

function readAccessor(accessor: RuntimeAccessor): number[] {
  const bytes = componentByteSize(accessor.componentType);
  const view = new DataView(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength);
  const values = new Array(accessor.count * accessor.components);
  for (let index = 0; index < values.length; index += 1) {
    values[index] = readComponent(view, index * bytes, accessor.componentType);
  }
  return values;
}
