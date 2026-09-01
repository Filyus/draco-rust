/**
 * Semantic FBX -> SceneDocument import boundary.
 *
 * The existing FBX bind-pose, root-motion, and cubic compatibility policies
 * stay here (via fbx-animation-adapter.js). The output is source-neutral: it
 * contains matrices/bytes/TRS clips and no FBX parser or browser objects.
 */

import { cloneTrs, decomposeMat4, identityMat4, invertMat4, multiplyMat4 } from './mat4.ts';
import { fbxSpace, multiplyQuaternion } from './fbx-space.ts';
import type { FbxSpace } from './fbx-space.ts';
import { adaptFbxAnimation } from './fbx-animation-adapter.ts';
import { adaptFbxMaterial } from './fbx-material-adapter.ts';
import { assertValidSceneDocument, createSceneDocument } from './scene-document.ts';
import type {
  AlphaMode, AnimationChannel, AnimationSampler, AttributeMap, SceneDocument, SceneMaterial,
  SceneNode, ScenePrimitive,
} from './scene-document.ts';
import { createFbxSceneProvenance } from './fbx-scene-provenance.ts';
import type { FbxSceneProvenance } from './fbx-scene-provenance.ts';
import {
  appendAccessor, basename, bytesFromF32, bytesFromU16, bytesFromU32,
  mimeFromUri, resolveResource, sniffMime,
} from './scene-resources.ts';
import type { ResourceMap } from './scene-resources.ts';
import type { AnimationTarget, Trs } from './viewer-scene.ts';

/** The semantic FBX tree, walked node by node rather than trusted wholesale. */
type FbxJson = any;

/**
 * Per-node bookkeeping threaded through the whole conversion: which document
 * node it produced, plus the rest/animation bases the shared FBX animation
 * adapter reads. It has no world matrix — nothing renders these.
 */
interface FbxNodeState extends AnimationTarget {
  id: number | null;
  name: string;
  documentNode: number;
  trs: Trs;
  restTrs: Trs;
  bindTrs: Trs;
  animationTrs: Trs;
  hasComplexTransformStack: boolean;
  usesAuthoredModelTrs: boolean;
  localMatrix: Float32Array;
  weights: Float32Array;
  source: FbxJson;
}

/** The node lookups the animation pass resolves channel targets through. */
interface FbxImportState {
  stateById: Map<unknown, FbxNodeState>;
  stateByName: Map<string, FbxNodeState>;
  bindPoseByNodeId: Map<number, number[]>;
  nodes: FbxJson[];
}

/** Rotate a flat `[x, y, z, ...]` stream into glTF axes, and scale it. */
function rebaseVector3(values: ArrayLike<number>, space: FbxSpace, scale = 1): Float32Array {
  const output = new Float32Array(values.length);
  const matrix = space.basis;
  for (let index = 0; index + 2 < values.length; index += 3) {
    const [x, y, z] = [values[index], values[index + 1], values[index + 2]];
    output[index] = (matrix[0] * x + matrix[4] * y + matrix[8] * z) * scale;
    output[index + 1] = (matrix[1] * x + matrix[5] * y + matrix[9] * z) * scale;
    output[index + 2] = (matrix[2] * x + matrix[6] * y + matrix[10] * z) * scale;
  }
  return output;
}

/**
 * The same transform expressed in glTF axes: `B · M · B⁻¹`.
 *
 * A space change on a transform is a conjugation, not a multiplication — the
 * matrix has to receive glTF-space points and hand back glTF-space points, so it
 * is entered and left through the space. Uniform scale conjugates to itself, so
 * the unit factor stays a plain multiply on the translation column.
 */
function rebaseMatrix(matrix: ArrayLike<number>, space: FbxSpace, scale: number): Float32Array {
  const source = Array.from(matrix, Number);
  const rebased = space.identity
    ? source
    : multiplyMat4(multiplyMat4(space.basis, source), space.inverse) || source;
  const output = Float32Array.from(rebased);
  output[12] *= scale;
  output[13] *= scale;
  output[14] *= scale;
  return output;
}

/**
 * A rotation in glTF axes: `b ⊗ q ⊗ b⁻¹`.
 *
 * Conjugation is linear in `q`, which is what lets a cubic sampler's tangents go
 * through the same call as its key values — they live in the same space and are
 * the same shape.
 */
function rebaseQuaternions(values: ArrayLike<number>, space: FbxSpace): Float32Array {
  const output = new Float32Array(values.length);
  const inverse = [-space.rotation[0], -space.rotation[1], -space.rotation[2], space.rotation[3]];
  for (let index = 0; index + 3 < values.length; index += 4) {
    const rotated = multiplyQuaternion(
      multiplyQuaternion(space.rotation, [
        values[index], values[index + 1], values[index + 2], values[index + 3],
      ]),
      inverse,
    );
    output.set(rotated, index);
  }
  return output;
}

/** `xyzw` tangents: the direction turns, the handedness in `w` stays. */
function rebaseTangents(values: ArrayLike<number>, space: FbxSpace): Float32Array {
  const output = new Float32Array(values.length);
  for (let index = 0; index + 3 < values.length; index += 4) {
    output.set(rebaseVector3([values[index], values[index + 1], values[index + 2]], space), index);
    output[index + 3] = values[index + 3];
  }
  return output;
}

/** Per-axis scale follows the axes themselves; the signs do not apply to it. */
function rebaseScales(values: ArrayLike<number>, space: FbxSpace): Float32Array {
  const output = new Float32Array(values.length);
  for (let index = 0; index + 2 < values.length; index += 3) {
    for (let row = 0; row < 3; row += 1) output[index + row] = values[index + space.axes[row]];
  }
  return output;
}

/** Adapt a semantic `fbx-wasm` parse result into a portable SceneDocument. */
export function buildSceneDocumentFromFbx(
  parsed: FbxJson,
  resources: ResourceMap = Object.create(null),
): SceneDocument {
  const roots = parsed?.scene?.rootNodes;
  if (!Array.isArray(roots) || roots.length === 0) {
    throw new Error('FBX SceneDocument import requires semantic binary FBX scene data');
  }
  const space = fbxSpace(parsed?.scene?.globalSettings || parsed?.globalSettings);
  const document = createSceneDocument({ warnings: [
    ...(parsed.warnings || []),
    `FBX geometry and transforms were read in the space the file declares: ${space.metersPerUnit} m per unit, and its own up and front axes`,
    'FBX source unit/axis settings remain in the optional FBX provenance sidecar; SceneDocument uses canonical glTF meter/Y-up space',
  ] });
  const scale = space.metersPerUnit;
  const { materialMap } = collectFbxMaterials(parsed, resources, document);
  const state = buildFbxNodeState(roots, document, scale, space);
  attachMeshesAndSkins(roots, state, materialMap, document, scale, space);
  appendFbxAnimations(parsed?.scene?.animations || parsed?.animations || [], state, document, scale, space);
  assertValidSceneDocument(document);
  return document;
}

/**
 * Build the normal portable document plus an optional FBX-only sidecar.
 *
 * The sidecar is intentionally returned separately: viewer and GLB paths
 * receive only SceneDocument, while a future FBX export boundary can retain
 * raw unit/axis/curve provenance without contaminating shared scene data.
 */
export function buildSceneDocumentWithFbxProvenance(
  parsed: FbxJson,
  resources: ResourceMap = Object.create(null),
): { document: SceneDocument; provenance: FbxSceneProvenance } {
  return {
    document: buildSceneDocumentFromFbx(parsed, resources),
    provenance: createFbxSceneProvenance(parsed),
  };
}

function collectFbxMaterials(parsed: FbxJson, resources: ResourceMap, document: SceneDocument) {
  const sourceTextures = parsed?.scene?.textures || parsed?.textures || [];
  const textureMap: number[] = sourceTextures.map((texture: FbxJson, sourceIndex: number) => {
    const bytes = texture.content?.length ? new Uint8Array(texture.content) : resolveResource(texture.filename, resources);
    if (!bytes) {
      if (texture.filename) document.warnings.push(`FBX texture not selected: ${texture.filename}`);
      return -1;
    }
    const resource = document.resources.length;
    document.resources.push({
      name: texture.name || basename(texture.filename || `texture_${sourceIndex}`),
      mimeType: mimeFromUri(texture.filename) || sniffMime(bytes) || 'application/octet-stream',
      bytes: new Uint8Array(bytes),
    });
    const index = document.textures.length;
    document.textures.push({ name: texture.name || `texture_${sourceIndex}`, resource, sampler: defaultSampler() });
    return index;
  });
  const sourceMaterials = parsed?.scene?.materials || parsed?.materials || [];
  const materialMap: number[] = sourceMaterials.map((source: FbxJson, materialIndex: number) => {
    const converted = adaptFbxMaterial(source, materialIndex);
    const material: SceneMaterial = {
      name: converted.name,
      baseColorFactor: converted.baseColorFactor,
      metallicFactor: converted.metallic,
      roughnessFactor: converted.roughness,
      emissiveFactor: converted.emissiveFactor,
      doubleSided: converted.doubleSided,
      alphaMode: converted.alphaMode as AlphaMode,
      alphaCutoff: converted.alphaCutoff,
      unlit: converted.unlit,
    };
    for (const binding of source.textures || []) {
      const texture = textureMap[binding.textureIndex];
      if (!Number.isInteger(texture) || texture < 0) continue;
      if (binding.slot === 'diffuse') material.baseColorTexture = { texture, texCoord: 0 };
      else if (binding.slot === 'normal') material.normalTexture = { texture, texCoord: 0 };
      else if (binding.slot === 'emissive') material.emissiveTexture = { texture, texCoord: 0 };
      else if (binding.slot === 'roughness' || binding.slot === 'metallic') material.metallicRoughnessTexture = { texture, texCoord: 0 };
      else document.warnings.push(`FBX material ${source.name || materialIndex} ${binding.slot} texture is outside the portable PBR subset`);
    }
    const index = document.materials.length;
    document.materials.push(material);
    return index;
  });
  return { materialMap };
}

function buildFbxNodeState(roots: FbxJson[], document: SceneDocument, scale: number, space: FbxSpace): FbxImportState {
  const bindPoseByNodeId = new Map<number, number[]>();
  const collectBindPoses = (source: FbxJson) => {
    for (const mesh of source.meshes || []) {
      for (const entry of mesh.skin?.bindPose || []) {
        if (Number.isInteger(entry?.nodeId) && validMatrix(entry.matrix) && !bindPoseByNodeId.has(entry.nodeId)) bindPoseByNodeId.set(entry.nodeId, entry.matrix);
      }
    }
    for (const child of source.children || []) collectBindPoses(child);
  };
  roots.forEach(collectBindPoses);

  const nodes: SceneNode[] = [];
  const stateById = new Map<unknown, FbxNodeState>();
  const stateByName = new Map<string, FbxNodeState>();
  const rootsOut: number[] = [];
  const append = (source: FbxJson, parentBind: number[] | null = null): number => {
    const bind = Number.isInteger(source.id) ? bindPoseByNodeId.get(source.id) : null;
    const sourceMatrix = validMatrix(source.matrix) ? source.matrix : null;
    // Conjugated here rather than at the end, so that everything derived from
    // it -- the document's matrix, the rest pose, the animation base the shared
    // FBX adapter reads -- is in one space instead of two.
    const rawLocalMatrix = rebaseMatrix(bind
      ? (parentBind ? (multiplyMat4(invertMat4(parentBind), bind) || bind) : bind)
      : sourceMatrix || identityMat4(), space, 1);
    const localMatrix = scaleMatrixTranslation(rawLocalMatrix, scale);
    const bindTrs = decomposeMat4(rawLocalMatrix);
    const animationTrs = sourceMatrix && !source.hasComplexTransformStack
      ? decomposeMat4(rebaseMatrix(sourceMatrix, space, 1))
      : cloneTrs(bindTrs);
    const index = nodes.length;
    const sceneNode: SceneNode = {
      name: source.name || `node_${index}`,
      matrix: Array.from(localMatrix),
      children: [],
    };
    nodes.push(sceneNode);
    const state: FbxNodeState = {
      id: Number.isInteger(source.id) ? source.id : null,
      name: sceneNode.name!,
      documentNode: index,
      // Keep the complete animation-adapter node contract here. The
      // adapter is shared with the established direct FBX path, whose
      // rest/animation bases distinguish skin placement from authored
      // local animation values.
      trs: cloneTrs(bindTrs),
      restTrs: cloneTrs(bindTrs),
      bindTrs,
      animationTrs,
      hasComplexTransformStack: Boolean(source.hasComplexTransformStack),
      usesAuthoredModelTrs: Boolean(sourceMatrix && !source.hasComplexTransformStack),
      localMatrix: rawLocalMatrix,
      weights: Float32Array.from(morphWeights(source.meshes?.[0])),
      source,
    };
    if (state.id !== null) stateById.set(state.id, state);
    if (source.name) stateByName.set(source.name, state);
    sceneNode.children!.push(...(source.children || []).map((child: FbxJson) => append(child, bind || parentBind)));
    return index;
  };
  for (const root of roots) rootsOut.push(append(root));
  document.nodes.push(...nodes);
  document.rootNodes.push(...rootsOut);
  return { stateById, stateByName, bindPoseByNodeId, nodes };
}

function attachMeshesAndSkins(roots: FbxJson[], state: FbxImportState, materialMap: number[], document: SceneDocument, scale: number, space: FbxSpace) {
  const append = (source: FbxJson, ownerState: FbxNodeState) => {
    const ownerNode = document.nodes[ownerState.documentNode];
    const meshBindings = [];
    for (const sourceMesh of source.meshes || []) {
      const meshIndex = appendMesh(sourceMesh, materialMap, document, scale, space);
      if (meshIndex < 0) continue;
      const skinIndex = appendSkin(sourceMesh, ownerState, state, document, scale, space);
      meshBindings.push({ source: sourceMesh, meshIndex, skinIndex, geometric: geometricMatrix(sourceMesh, space, scale) });
    }
    // A geometric offset places the geometry relative to its node and is not
    // inherited by the node's children, which no glTF-shaped document can say
    // about a node that also has children. The offset therefore becomes a child
    // node of its own -- the same shape the multi-mesh case already uses.
    const needsOwnNode = meshBindings.length > 1 || meshBindings.some((binding) => binding.geometric);
    if (!needsOwnNode && meshBindings.length === 1) {
      ownerNode.mesh = meshBindings[0].meshIndex;
      if (meshBindings[0].skinIndex >= 0) ownerNode.skin = meshBindings[0].skinIndex;
      // The bound geometry's own weights, not the node's first: a geometry with
      // nothing to draw is left out, so the two need not be the same one.
      ownerNode.weights = morphWeights(meshBindings[0].source);
    } else if (meshBindings.length > 0) {
      for (const binding of meshBindings) {
        const childIndex = document.nodes.length;
        document.nodes.push({
          name: `${ownerNode.name}_mesh_${childIndex}`,
          ...(binding.geometric
            ? { matrix: Array.from(binding.geometric) }
            : { translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1] }),
          mesh: binding.meshIndex,
          ...(binding.skinIndex >= 0 ? { skin: binding.skinIndex } : {}),
        });
        ownerNode.children!.push(childIndex);
      }
    }
    for (const child of source.children || []) {
      const childState = state.stateById.get(child.id);
      if (childState) append(child, childState);
    }
  };
  for (const root of roots) {
    const rootState = state.stateById.get(root.id);
    if (rootState) append(root, rootState);
  }
}

/**
 * The mesh's `Geometric*` offset in document space, or null when it has none.
 *
 * Conjugated through the same space and unit scale as a node's own matrix,
 * because that is what it becomes.
 */
function geometricMatrix(source: FbxJson, space: FbxSpace, scale: number): Float32Array | null {
  const matrix = source?.geometricTransform?.matrix;
  if (!validMatrix(Array.isArray(matrix) ? matrix : Array.from(matrix || []))) return null;
  return rebaseMatrix(matrix, space, scale);
}

/**
 * Append one FBX geometry as a document mesh, or -1 when it holds no polygons.
 *
 * FBX carries a curve as a Geometry object like any other: control points, no
 * `PolygonVertexIndex`. Nothing renders from one, and a document mesh must have
 * at least one primitive, so a file that contains any -- a chain along a spline,
 * a lattice -- would otherwise fail validation as a whole and refuse to open.
 * The node keeps its transform and its children; only the geometry is dropped.
 */
function appendMesh(source: FbxJson, materialMap: number[], document: SceneDocument, scale: number, space: FbxSpace) {
  if ((source.indices?.length || 0) < 3) {
    document.warnings.push(`FBX geometry ${source.name || 'geometry'} has no polygons and is not part of the scene`);
    return -1;
  }
  const vertexCount = (source.positions?.length || 0) / 3;
  const attributes: AttributeMap = {
    POSITION: appendFloatAccessor(document, rebaseVector3(source.positions || [], space, scale), 3),
  };
  // Normals rotate but do not scale, and the space is a signed permutation, so
  // its inverse transpose is itself -- the same call serves both.
  if (source.normals?.length === vertexCount * 3) {
    attributes.NORMAL = appendFloatAccessor(document, rebaseVector3(source.normals, space), 3);
  }
  // FBX puts V's origin at the opposite end of the image from glTF, which is why
  // the writer flips it on the way out. The way in did not, so the pair was
  // asymmetric: a glTF exported to FBX and read back had its textures upside
  // down, and an FBX exported to GLB carried the flip into the result.
  if (source.uvs?.length === vertexCount * 2) {
    attributes.TEXCOORD_0 = appendFloatAccessor(document, flipUvV(source.uvs), 2);
  }
  // Extra FBX UV layers become TEXCOORD_1.. so a second set -- a lightmap,
  // typically -- survives into glTF instead of being dropped at import.
  for (let set = 1; set < Math.min(source.uvLayers?.length ?? 0, 8); set += 1) {
    const values = source.uvLayers[set];
    if (values?.length === vertexCount * 2) attributes[`TEXCOORD_${set}`] = appendFloatAccessor(document, flipUvV(values), 2);
  }
  // FBX LayerElementColor is linear RGBA on the polygon-corner domain, which
  // is already what the render mesh hands us.
  if (source.colors?.length === vertexCount * 4) attributes.COLOR_0 = appendFloatAccessor(document, source.colors, 4);
  // FBX splits tangents across Tangents and TangentsW; the reader merges them
  // into xyzw, which is already glTF's TANGENT layout. Files older than 7500
  // have no handedness array, so w was defaulted to +1 there.
  if (source.tangents?.length === vertexCount * 4) {
    attributes.TANGENT = appendFloatAccessor(document, rebaseTangents(source.tangents, space), 4);
  }
  for (let set = 1; set < (source.uvSets?.length || 0) && set < 8; set += 1) {
    const expanded = expandFbxLayer(source, source.uvSets[set], 2, vertexCount);
    if (expanded) attributes[`TEXCOORD_${set}`] = appendFloatAccessor(document, expanded, 2);
  }
  const influences = expandFbxInfluences(source, vertexCount);
  const influenceSets = influences || [
    source.joints0,
    source.joints1,
  ].map((joints, set) => joints?.length === vertexCount * 4 && source[`weights${set}`]?.length === vertexCount * 4
    ? { joints, weights: source[`weights${set}`] } : null).filter(Boolean);
  for (let set = 0; set < influenceSets.length && set < 2; set += 1) {
    const influence = influenceSets[set];
    attributes[`JOINTS_${set}`] = appendAccessor(document, { bytes: bytesFromU16(influence!.joints), componentType: 5123, components: 4, count: vertexCount, normalized: false });
    attributes[`WEIGHTS_${set}`] = appendFloatAccessor(document, influence!.weights, 4);
  }
  const targets = appendMorphTargets(source, vertexCount, document, scale, space);
  const indices = source.indices || [];
  const groups = materialGroups(indices, source.materialIndices, source.material, materialMap, document, source.name);
  const primitives: ScenePrimitive[] = groups.map((group) => ({
    attributes,
    indices: appendAccessor(document, { bytes: bytesFromU32(group.indices), componentType: 5125, components: 1, count: group.indices.length, normalized: false }),
    mode: 4,
    ...(group.material >= 0 ? { material: group.material } : {}),
    ...(targets.length > 0 ? { targets } : {}),
  }));
  const index = document.meshes.length;
  document.meshes.push({ name: source.name || `mesh_${index}`, weights: morphWeights(source), primitives });
  if (source.skin?.clusters?.some((cluster: FbxJson) => cluster.controlPointIndices?.length > 0) && !attributes.JOINTS_0) {
    document.warnings.push(`FBX mesh ${source.name || index} lacks render JOINTS_0/WEIGHTS_0 expansion and cannot be skinned by the portable runtime`);
  }
  return index;
}

function expandFbxInfluences(source: FbxJson, vertexCount: number) {
  const clusters = source.skin?.clusters || [];
  if (!clusters.some((cluster: FbxJson) => cluster.controlPointIndices?.length || cluster.renderPointIndices?.length)) return null;
  const perVertex: { joint: number; weight: number }[][] = Array.from({ length: vertexCount }, () => []);
  const renderByControl = buildRenderPointsByControl(source);
  clusters.forEach((cluster: FbxJson, joint: number) => {
    const controlPoints = cluster.controlPointIndices || [];
    for (let index = 0; index < controlPoints.length; index += 1) {
      const weight = Number(cluster.weights?.[index]) || 0;
      if (weight <= 0) continue;
      const points = renderByControl.get(controlPoints[index]) || (cluster.renderPointIndices?.length === cluster.weights?.length ? [cluster.renderPointIndices[index]] : []);
      for (const point of points) if (point >= 0 && point < vertexCount) perVertex[point].push({ joint, weight });
    }
  });
  if (!perVertex.some((entries) => entries.length > 0)) return null;
  const sets = [
    { joints: new Uint16Array(vertexCount * 4), weights: new Float32Array(vertexCount * 4) },
    { joints: new Uint16Array(vertexCount * 4), weights: new Float32Array(vertexCount * 4) },
  ];
  perVertex.forEach((entries: { joint: number; weight: number }[], vertex: number) => {
    entries.sort((left, right) => right.weight - left.weight);
    const selected = entries.slice(0, 8);
    const sum = selected.reduce((total, entry) => total + entry.weight, 0);
    selected.forEach((entry, slot) => {
      const set = slot < 4 ? 0 : 1;
      const offset = vertex * 4 + (slot % 4);
      sets[set].joints[offset] = entry.joint;
      sets[set].weights[offset] = sum > 0 ? entry.weight / sum : 0;
    });
  });
  if (sets[1].weights.every((weight: number) => weight === 0)) sets.pop();
  return sets;
}

function buildRenderPointsByControl(source: FbxJson) {
  const byControl = new Map<number, number[]>();
  if (!source.polygonVertexIndices?.length) return byControl;
  let polygon: number[] = [];
  let render = 0;
  const emit = (controlPoint: number) => {
    if (!byControl.has(controlPoint)) byControl.set(controlPoint, []);
    byControl.get(controlPoint)!.push(render);
    render += 1;
  };
  for (const encoded of source.polygonVertexIndices) {
    polygon.push(encoded < 0 ? ~encoded : encoded);
    if (encoded < 0) {
      for (let index = 1; index < polygon.length - 1; index += 1) {
        emit(polygon[0]); emit(polygon[index]); emit(polygon[index + 1]);
      }
      polygon = [];
    }
  }
  return byControl;
}

function expandFbxLayer(source: FbxJson, layer: FbxJson, components: number, vertexCount: number) {
  if (!layer || !source.controlPoints?.length || !source.polygonVertexIndices?.length) return null;
  const output: number[] = [];
  // Hoisted out of emit: these do not vary per corner, and emit runs once for
  // every polygon vertex in the mesh.
  const mapping = layer.mapping || 'ByControlPoint';
  const indexToDirect = layer.reference === 'IndexToDirect';
  const values: ArrayLike<number> = layer.values || [];
  const emit = (controlPoint: number, corner: number) => {
    const logical = mapping === 'ByPolygonVertex' ? corner : mapping === 'AllSame' ? 0 : controlPoint;
    const valueIndex = indexToDirect ? (layer.indices?.[logical] ?? logical) : logical;
    const start = Math.max(0, valueIndex) * components;
    // Copied element by element: slicing allocated a throwaway array per
    // corner, and spreading it passed the copy as arguments.
    const end = Math.min(start + components, values.length);
    for (let index = start; index < end; index += 1) output.push(values[index]);
  };
  let polygon: { controlPoint: number; corner: number }[] = [];
  let corner = 0;
  for (const encoded of source.polygonVertexIndices) {
    const controlPoint = encoded < 0 ? ~encoded : encoded;
    polygon.push({ controlPoint, corner });
    corner += 1;
    if (encoded < 0) {
      for (let index = 1; index < polygon.length - 1; index += 1) {
        for (const entry of [polygon[0], polygon[index], polygon[index + 1]]) emit(entry.controlPoint, entry.corner);
      }
      polygon = [];
    }
  }
  return output.length === vertexCount * components ? output : null;
}

function appendSkin(sourceMesh: FbxJson, ownerState: FbxNodeState, state: FbxImportState, document: SceneDocument, scale: number, space: FbxSpace) {
  const clusters = sourceMesh.skin?.clusters || [];
  if (clusters.length === 0) return -1;
  const bindPose = new Map<unknown, number[]>((sourceMesh.skin.bindPose || []).filter((entry: FbxJson) => validMatrix(entry.matrix)).map((entry: FbxJson) => [entry.nodeId, entry.matrix]));
  const joints: number[] = [];
  const matrices: number[] = [];
  for (const cluster of clusters) {
    const jointState = state.stateById.get(cluster.jointNodeId);
    if (!jointState) {
      document.warnings.push(`FBX skin cluster targets missing joint ${cluster.jointNodeId} and was omitted`);
      continue;
    }
    const meshBind = rebaseMatrix(bindPose.get(ownerState.id) || cluster.meshBindTransform || identityMat4(), space, scale);
    const jointBind = rebaseMatrix(jointState.hasComplexTransformStack
      ? (bindPose.get(cluster.jointNodeId) || cluster.jointBindTransform || identityMat4())
      : (cluster.jointBindTransform || bindPose.get(cluster.jointNodeId) || identityMat4()), space, scale);
    const inverse = invertMat4(jointBind) || identityMat4();
    joints.push(jointState.documentNode);
    matrices.push(...(multiplyMat4(inverse, meshBind) || inverse));
  }
  if (joints.length === 0) return -1;
  const inverseBindMatrices = appendFloatAccessor(document, matrices, 16);
  const index = document.skins.length;
  document.skins.push({ name: `${sourceMesh.name || 'mesh'}_skin`, joints, inverseBindMatrices });
  return index;
}

function appendMorphTargets(source: FbxJson, vertexCount: number, document: SceneDocument, scale: number, space: FbxSpace) {
  return (source.morphTargets || []).map((target: FbxJson) => {
    const position = new Float32Array(vertexCount * 3);
    const renderIndices = target.renderPointIndices || [];
    const renderDeltas = target.renderPositionDeltas || [];
    for (let entry = 0; entry < renderIndices.length; entry += 1) {
      const render = renderIndices[entry] * 3;
      const delta = entry * 3;
      if (render + 2 < position.length && delta + 2 < renderDeltas.length) position.set(rebaseVector3(renderDeltas.slice(delta, delta + 3), space, scale), render);
    }
    const output: AttributeMap = { POSITION: appendFloatAccessor(document, position, 3) };
    if (target.renderNormalDeltas?.length) {
      const normal = new Float32Array(vertexCount * 3);
      for (let entry = 0; entry < renderIndices.length; entry += 1) {
        const render = renderIndices[entry] * 3;
        const delta = entry * 3;
        if (render + 2 < normal.length && delta + 2 < target.renderNormalDeltas.length) normal.set(target.renderNormalDeltas.slice(delta, delta + 3), render);
      }
      output.NORMAL = appendFloatAccessor(document, normal, 3);
    }
    return output;
  });
}

function appendFbxAnimations(clips: FbxJson[], state: FbxImportState, document: SceneDocument, scale: number, space: FbxSpace) {
  for (const clip of clips) {
    const adapted = adaptFbxAnimation(clip, state.stateById, state.stateByName);
    if (!adapted) continue;
    const byNode = new Map();
    for (const channel of adapted.channels) {
      if (!byNode.has(channel.node)) byNode.set(channel.node, []);
      byNode.get(channel.node).push(channel);
    }
    const samplers: AnimationSampler[] = [];
    const channels: AnimationChannel[] = [];
    for (const [node, nodeChannels] of byNode) {
      const time = nodeChannels[0].sampler.input;
      for (const path of ['translation', 'rotation', 'scale'] as const) {
        const sourceChannel = nodeChannels.find((channel: FbxJson) => channel.path === path);
        const channel = sourceChannel
          ? rebaseFbxAnimationChannel(sourceChannel, scale, space)
          : constantFbxChannel(node, path, time, scale);
        // Only TRS paths reach here; weight channels are appended by
        // the loop below, which is where a target count is meaningful.
        const sampler = appendAnimationSampler(document, channel.sampler);
        const samplerIndex = samplers.length;
        samplers.push(sampler);
        channels.push({ sampler: samplerIndex, node: node.documentNode, path });
      }
      for (const channel of nodeChannels.filter((candidate: FbxJson) => candidate.path === 'weights')) {
        const samplerIndex = samplers.length;
        samplers.push(appendAnimationSampler(document, channel.sampler, channel.targetCount));
        channels.push({ sampler: samplerIndex, node: node.documentNode, path: 'weights' });
      }
    }
    if (channels.length > 0) document.animations.push({ name: adapted.name, duration: adapted.duration, samplers, channels });
  }
}

function constantFbxChannel(node: FbxNodeState, path: 'translation' | 'rotation' | 'scale', input: Float32Array, scale: number) {
  const values = path === 'translation'
    ? node.animationTrs.translation.map((value) => value * scale)
    : path === 'rotation' ? node.animationTrs.rotation : node.animationTrs.scale;
  const output = new Float32Array(input.length * values.length);
  for (let index = 0; index < input.length; index += 1) output.set(values, index * values.length);
  return { path, sampler: { input, output, interpolation: 'LINEAR' } };
}

/**
 * One animated channel in glTF axes and metres.
 *
 * Every re-basing here is a linear map on the value, which is what makes a cubic
 * sampler safe to run through unchanged: its tangents sit in the same space and
 * have the same shape as its keys, so the whole output stream is one call.
 */
function rebaseFbxAnimationChannel(channel: FbxJson, scale: number, space: FbxSpace) {
  const output = channel.path === 'translation'
    ? rebaseVector3(channel.sampler.output, space, scale)
    : space.identity ? null
      : channel.path === 'rotation' ? rebaseQuaternions(channel.sampler.output, space)
        : channel.path === 'scale' ? rebaseScales(channel.sampler.output, space) : null;
  if (!output) return channel;
  return { ...channel, sampler: { ...channel.sampler, output } };
}

function appendAnimationSampler(document: SceneDocument, sampler: FbxJson, targetCount?: number) {
  const input = appendFloatAccessor(document, sampler.input, 1);
  const components = targetCount || (sampler.output.length / (sampler.input.length * ((sampler.interpolation || 'LINEAR') === 'CUBICSPLINE' ? 3 : 1)));
  const output = appendFloatAccessor(document, sampler.output, components);
  return { input, output, interpolation: sampler.interpolation || 'LINEAR' };
}

function materialGroups(indices: FbxJson, materialIndices: FbxJson, fallback: number, materialMap: number[], document: SceneDocument, name: string) {
  const source = Array.from(indices || []);
  const groups = new Map();
  for (let offset = 0; offset + 2 < source.length; offset += 3) {
    const sourceMaterial = materialIndices?.[offset / 3] ?? fallback;
    const material = Number.isInteger(sourceMaterial) ? (materialMap[sourceMaterial] ?? -1) : -1;
    if (!groups.has(material)) groups.set(material, []);
    groups.get(material).push(source[offset], source[offset + 1], source[offset + 2]);
  }
  if (groups.size > 1) document.warnings.push(`FBX mesh ${name || 'mesh'} material assignments were split into portable primitives`);
  return [...groups.entries()].map(([material, groupIndices]) => ({ material, indices: groupIndices }));
}

function morphWeights(mesh: FbxJson): number[] {
  return Array.from<FbxJson, number>(mesh?.morphTargets || [], (target) => (Number(target.defaultWeight) || 0) / 100);
}

/** V runs the other way in FBX; the same one line serves both directions. */
function flipUvV(values: ArrayLike<number>): Float32Array {
  const output = new Float32Array(values.length);
  for (let index = 0; index < values.length; index += 1) {
    output[index] = index % 2 === 1 ? 1 - values[index] : values[index];
  }
  return output;
}

function appendFloatAccessor(document: SceneDocument, values: ArrayLike<number>, components: number): number {
  return appendAccessor(document, { bytes: bytesFromF32(values), componentType: 5126, components, count: values.length / components, normalized: false });
}

function scaleMatrixTranslation(matrix: Float32Array, scale: number): Float32Array {
  const output = Float32Array.from(matrix);
  output[12] *= scale;
  output[13] *= scale;
  output[14] *= scale;
  return output;
}

function validMatrix(matrix: unknown): boolean {
  return Array.isArray(matrix) && matrix.length === 16 && matrix.every(Number.isFinite);
}

function defaultSampler() {
  return { wrapS: 10497, wrapT: 10497, minFilter: 9987, magFilter: 9729 };
}

