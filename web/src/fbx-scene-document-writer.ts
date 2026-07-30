/**
 * Portable SceneDocument -> typed FBX SceneInput adapter.
 *
 * This is the FBX export boundary: the document contract stays format-neutral,
 * while axis/unit conversion, Euler lowering, skin-cluster construction, and
 * optional source provenance are kept here. The returned object is consumed by
 * fbx-wasm.create_fbx_scene; no flattened mesh writer is involved.
 */

import { DEFAULT_FBX_EXPORT_SPACE, FBX_EXPORT_SPACES, fbxSpace } from './fbx-space.ts';
import type { FbxExportSpaceName, FbxSpace } from './fbx-space.ts';
import { assertValidSceneDocument } from './scene-document.ts';
import { hasMaterialExtensionValues } from './material-extensions.ts';
import type { SceneDocument, ScenePrimitive } from './scene-document.ts';
import { componentByteWidth, normalizeComponent, readComponent } from './component-values.ts';
import { invertMat4, multiplyMat4 } from './mat4.ts';
import {
  convertGltfMatrixToFbx,
  convertGltfVectorArrayToFbx,
  lowerPbrToFbxPhong,
  quaternionKeysToFbxEuler,
} from './fbx-scene-adapter.ts';
import { assertFbxProvenance } from './fbx-scene-provenance.ts';
import type { FbxSceneProvenance } from './fbx-scene-provenance.ts';

/**
 * The typed-writer SceneInput and the semantic source scene it may reuse.
 * Both are fbx-wasm's own shapes, assembled field by field here.
 */
type FbxJson = any;

export interface FbxWriteOptions {
  provenance?: FbxSceneProvenance | null;
  /**
   * Which space to write, when there is no provenance to follow.
   *
   * A re-export ignores this: it goes back into the space its source declared,
   * which is what makes it a round trip rather than a conversion.
   */
  space?: FbxExportSpaceName;
}

/** Unit handling depends on whether a source FBX scene is being reused. */
type SourceUnits = FbxJson;

/**
 * Source meshes indexed both ways: FBX geometry need not be named, and the
 * positional list is the only way to reach the unnamed ones.
 */
interface SourceMeshIndex {
  byName: Map<string, FbxJson>;
  ordered: FbxJson[];
}

/** Build a typed-writer SceneInput from a validated portable document. */
export function buildFbxSceneFromDocument(document: SceneDocument, options: FbxWriteOptions = {}) {
  assertValidSceneDocument(document);
  const provenance = options.provenance || null;
  if (provenance) assertFbxProvenance(provenance);
  const sourceScene = provenance?.sourceScene || null;
  const sourceNodes = indexSourceNodes(sourceScene?.rootNodes || []);
  const sourceMeshes = indexSourceMeshes(sourceScene?.rootNodes || []);
  const sourceUnits = Boolean(provenance);
  // Where the geometry is written. A re-export goes back into the space its
  // source declared -- that is what makes it a round trip -- and everything else
  // into the space the caller asked for.
  const space = fbxSpace(provenance
    ? sourceScene?.globalSettings
    : FBX_EXPORT_SPACES[options.space || DEFAULT_FBX_EXPORT_SPACE]);
  const worlds = buildDocumentWorlds(document);
  const warnings = [...document.warnings];
  // FBX has light nodes of its own, but this writer emits geometry and
  // materials only, so a lit scene arrives dark and has to say so.
  if (document.lights?.length) {
    warnings.push(`${document.lights.length} punctual lights were not written: the FBX writer emits geometry and materials only`);
  }

  const scene = {
    // Declared rather than left to the writer's defaults: the file has to say
    // which space it is in, and this is the only place that knows.
    globalSettings: { ...space.settings },
    rootNodes: document.rootNodes.map((index) => buildNode(
      document,
      index,
      sourceNodes,
      sourceMeshes,
      worlds,
      sourceUnits,
      space,
      warnings,
    )),
    materials: buildMaterials(document, warnings),
    textures: buildTextures(document),
    animations: sourceScene?.animations?.length
      ? structuredClone(sourceScene.animations)
      : buildAnimations(document, space, warnings),
    warnings,
  };
  return scene;
}

function buildNode(
  document: SceneDocument,
  index: number,
  sourceNodes: Map<string, FbxJson>,
  sourceMeshes: SourceMeshIndex,
  worlds: number[][],
  sourceUnits: SourceUnits,
  space: FbxSpace,
  warnings: string[],
): FbxJson {
  const node = document.nodes[index] || {};
  const source = node.name === undefined ? undefined : sourceNodes.get(node.name);
  const matrix = source?.matrix?.length === 16
    ? Array.from(source.matrix)
    : convertNodeMatrix(node, space);
  const meshes = node.mesh === undefined ? [] : buildNodeMeshes(
    document,
    node,
    node.mesh,
    index,
    sourceMeshes,
    worlds,
    sourceUnits,
    space,
    warnings,
  );
  return {
    id: index + 1,
    name: node.name || `node_${index}`,
    matrix,
    ...(source?.transformStack ? { transformStack: structuredClone(source.transformStack) } : {}),
    meshes,
    children: (node.children || []).map((child) => buildNode(
      document,
      child,
      sourceNodes,
      sourceMeshes,
      worlds,
      sourceUnits,
      space,
      warnings,
    )),
  };
}

function buildNodeMeshes(
  document: SceneDocument,
  node: FbxJson,
  meshIndex: number,
  nodeIndex: number,
  sourceMeshes: SourceMeshIndex,
  worlds: number[][],
  sourceUnits: SourceUnits,
  space: FbxSpace,
  warnings: string[],
): FbxJson[] {
  const mesh = document.meshes[meshIndex];
  if (!mesh) return [];
  return mesh.primitives.map((primitive, primitiveIndex) => {
    const positions = readAccessorValues(document, primitive.attributes.POSITION);
    const indices = primitive.indices === undefined
      ? Array.from({ length: positions.length / 3 }, (_, value) => value)
      : readAccessorValues(document, primitive.indices).map((value) => Math.max(0, Math.trunc(value)));
    const sourceMesh = findSourceMesh(sourceMeshes, document.meshes, mesh.name, meshIndex, primitiveIndex);
    // With provenance present, an unmatched source mesh is the one case
    // where FBX-only data is genuinely lost: extra UV/normal/colour layers,
    // tangents, binormals, hard edges and creases all come from it. Without
    // provenance there is nothing to lose, so this is the only reachable
    // point at which the loss can be reported.
    if (sourceUnits && !sourceMesh) {
      warnings.push(`FBX mesh ${mesh.name ?? meshIndex} could not be matched to its source geometry, so FBX-only layers were not re-exported`);
    }
    const meshInput: FbxJson = {
      name: mesh.name ? `${mesh.name}_${primitiveIndex}` : `mesh_${meshIndex}_${primitiveIndex}`,
      positions: convertGltfVectorArrayToFbx(positions, space),
      indices,
      normals: optionalAttribute(document, primitive, 'NORMAL', null),
      uvs: optionalAttribute(document, primitive, 'TEXCOORD_0', null),
      materialIndices: Array.from({ length: Math.floor(indices.length / 3) }, () => primitive.material ?? -1),
      morphTargets: buildMorphTargets(document, primitive, space),
    };
    // Normals turn with the space but take none of its unit factor, so they go
    // through a space that keeps the axes and drops the scale.
    if (meshInput.normals) meshInput.normals = convertGltfVectorArrayToFbx(meshInput.normals, unitlessSpace(space));
    if (meshInput.uvs && !sourceUnits) meshInput.uvs = flipUvV(meshInput.uvs);
    if (!sourceUnits) {
      const uvSets = [];
      for (let set = 1; set < 8; set += 1) {
        const values = optionalAttribute(document, primitive, `TEXCOORD_${set}`, flipUvV);
        if (!values) break;
        uvSets.push({ name: `UVSet${set}`, mapping: 'ByPolygonVertex', reference: 'Direct', values, indices: [] });
      }
      if (uvSets.length > 0) meshInput.uvSets = uvSets;
    }
    if (sourceUnits && sourceMesh?.uvSets?.length) meshInput.uvSets = structuredClone(sourceMesh.uvSets);
    if (sourceUnits && sourceMesh?.normalSets?.length) meshInput.normalSets = structuredClone(sourceMesh.normalSets);
    if (sourceUnits && sourceMesh?.colorSets?.length) {
      meshInput.colorSets = structuredClone(sourceMesh.colorSets);
    } else {
      // COLOR_0 is corner-domain RGBA, which is exactly what
      // LayerElementColor stores as ByPolygonVertex/Direct.
      const colors = optionalAttribute(document, primitive, 'COLOR_0', null);
      if (colors?.length) {
        const vertexCount = positions.length / 3;
        // glTF allows COLOR_0 as VEC3 or VEC4; FBX stores RGBA.
        const rgba = colors.length === vertexCount * 4
          ? Array.from(colors)
          : Array.from({ length: vertexCount * 4 }, (_, index) => (
            index % 4 === 3 ? 1 : colors[Math.floor(index / 4) * 3 + (index % 4)]
          ));
        meshInput.colorSets = [{
          name: 'Col', mapping: 'ByPolygonVertex', reference: 'Direct', values: rgba, indices: [],
        }];
      }
    }
    // Smoothing flags and crease weights address edges, polygons or control
    // points, none of which the portable document represents, so they only
    // travel on the FBX provenance path.
    if (sourceUnits && sourceMesh?.smoothingLayers?.length) {
      meshInput.smoothingLayers = structuredClone(sourceMesh.smoothingLayers);
    }
    if (sourceUnits && sourceMesh?.creaseLayers?.length) {
      meshInput.creaseLayers = structuredClone(sourceMesh.creaseLayers);
    }
    if (sourceUnits && sourceMesh?.tangentSets?.length) {
      meshInput.tangentSets = structuredClone(sourceMesh.tangentSets);
      if (sourceMesh.binormalSets?.length) meshInput.binormalSets = structuredClone(sourceMesh.binormalSets);
    } else {
      // glTF TANGENT is xyzw with the handedness sign in w, which is what
      // the writer splits back into Tangents and TangentsW.
      const tangents = optionalAttribute(document, primitive, 'TANGENT', null);
      if (tangents?.length === (positions.length / 3) * 4) {
        // Same Y-up to Z-up swap the normals get, applied to xyz only.
        // It is a rotation, so the handedness in w is unaffected.
        const values = Array.from(tangents);
        if (!sourceUnits) {
          for (let offset = 0; offset + 3 < values.length; offset += 4) {
            const y = values[offset + 1];
            values[offset + 1] = values[offset + 2];
            values[offset + 2] = -y;
          }
        }
        meshInput.tangentSets = [{
          name: '', mapping: 'ByPolygonVertex', reference: 'Direct',
          // The sign came from glTF, so it is real data rather than
          // the reader's default and is written out as TangentsW.
          values, indices: [], hasHandedness: true,
        }];
      }
    }
    const skinIndex = node.skin;
    if (Number.isInteger(skinIndex) && document.skins[skinIndex]) {
      meshInput.skin = sourceUnits && sourceMesh?.skin
        ? structuredClone(sourceMesh.skin)
        : buildSkin(document, primitive, skinIndex, nodeIndex, worlds, space, warnings);
    }
    if (sourceUnits && sourceMesh?.controlPoints?.length) {
      meshInput.controlPoints = sourceMesh.controlPoints.flat();
      meshInput.polygonVertexIndices = sourceMesh.polygonVertexIndices?.slice() || [];
    }
    return meshInput;
  });
}

function optionalAttribute(
  document: SceneDocument,
  primitive: ScenePrimitive,
  semantic: string,
  transform: ((values: number[]) => number[]) | null,
) {
  const index = primitive.attributes?.[semantic];
  if (!Number.isInteger(index)) return null;
  const values = readAccessorValues(document, index);
  return transform ? transform(values) : values;
}

function buildSkin(
  document: SceneDocument,
  primitive: ScenePrimitive,
  skinIndex: number,
  nodeIndex: number,
  worlds: number[][],
  space: FbxSpace,
  warnings: string[],
): FbxJson {
  const skin = document.skins[skinIndex];
  const joints = skin.joints || [];
  const influenceSets: FbxJson[] = [];
  for (let set = 0; set < 8; set += 1) {
    const jointValues = optionalAttribute(document, primitive, `JOINTS_${set}`, null);
    const weightValues = optionalAttribute(document, primitive, `WEIGHTS_${set}`, null);
    if (jointValues === null && weightValues === null) break;
    if (!jointValues || !weightValues || jointValues.length !== weightValues.length || jointValues.length % 4 !== 0) {
      warnings.push(`FBX export skin ${skinIndex} has unaligned JOINTS_${set}/WEIGHTS_${set} attributes`);
      continue;
    }
    influenceSets.push({ jointValues, weightValues });
  }
  if (influenceSets.length === 0) {
    warnings.push(`FBX export skin ${skinIndex} lacks aligned joint/weight attributes`);
    return null;
  }
  const inverseBinds = skin.inverseBindMatrices === undefined
    ? [] : readAccessorMatrices(document, skin.inverseBindMatrices);
  const meshWorld = worlds[nodeIndex] || identityMatrix();
  const clusters = joints.map((jointIndex, jointSlot) => {
    const jointWorld = inverseBinds[jointSlot] ? (invertMat4(inverseBinds[jointSlot]) || identityMatrix()) : worlds[jointIndex] || identityMatrix();
    const convertedJoint = convertGltfMatrixToFbx(jointWorld, space);
    const convertedMesh = convertGltfMatrixToFbx(meshWorld, space);
    const meshBind = multiplyMat4(convertedMesh, invertMat4(convertedJoint) || identityMatrix());
    const controlPointIndices = [];
    const weights = [];
    const vertexCount = influenceSets[0].jointValues.length / 4;
    for (let vertex = 0; vertex < vertexCount; vertex += 1) {
      for (const { jointValues, weightValues } of influenceSets) {
        for (let component = 0; component < 4; component += 1) {
          if (Math.trunc(jointValues[vertex * 4 + component]) !== jointSlot) continue;
          const weight = Number(weightValues[vertex * 4 + component]) || 0;
          if (weight <= 0) continue;
          controlPointIndices.push(vertex);
          weights.push(weight);
        }
      }
    }
    return {
      jointNodeId: jointIndex + 1,
      controlPointIndices,
      weights,
      meshBindTransform: meshBind,
      jointBindTransform: convertedJoint,
    };
  }).filter((cluster) => cluster.weights.length > 0);
  const bindPose = [
    { nodeId: nodeIndex + 1, matrix: convertGltfMatrixToFbx(meshWorld, space) },
    ...joints.map((jointIndex, slot) => ({
      nodeId: jointIndex + 1,
      matrix: convertGltfMatrixToFbx(
        inverseBinds[slot] ? (invertMat4(inverseBinds[slot]) || identityMatrix()) : worlds[jointIndex] || identityMatrix(),
        space,
      ),
    })),
  ];
  return { clusters, bindPose };
}

function buildMorphTargets(document: SceneDocument, primitive: ScenePrimitive, space: FbxSpace): FbxJson[] {
  return (primitive.targets || []).flatMap((target, index) => {
    if (target.POSITION === undefined) return [];
    const position = readAccessorValues(document, target.POSITION);
    const normal = target.NORMAL === undefined ? null : readAccessorValues(document, target.NORMAL);
    return [{
      name: `target_${index}`,
      controlPointIndices: Array.from({ length: position.length / 3 }, (_, value) => value),
      positionDeltas: convertGltfVectorArrayToFbx(position, space),
      ...(normal ? { normalDeltas: convertGltfVectorArrayToFbx(normal, unitlessSpace(space)) } : {}),
      defaultWeight: 0,
      fullWeight: 100,
    }];
  });
}

function buildMaterials(document: SceneDocument, warnings: string[]): FbxJson[] {
  let droppedLayers = false;
  const materials = document.materials.map((material, index) => {
    const textures: { slot: string; textureIndex: number }[] = [];
    const add = (info: { texture?: number } | undefined, slot: string) => {
      if (Number.isInteger(info?.texture)) textures.push({ slot, textureIndex: info!.texture! });
    };
    add(material.baseColorTexture, 'diffuse');
    add(material.normalTexture, 'normal');
    add(material.emissiveTexture, 'emissive');
    add(material.metallicRoughnessTexture, 'roughness');
    if (hasMaterialExtensionValues(material)) droppedLayers = true;
    return lowerPbrToFbxPhong({
      name: material.name || `material_${index}`,
      baseColorFactor: material.baseColorFactor || [1, 1, 1, 1],
      emissiveFactor: material.emissiveFactor || [0, 0, 0],
      metallicFactor: material.metallicFactor ?? 0,
      roughnessFactor: material.roughnessFactor ?? 1,
      textures,
    });
  });
  if (droppedLayers) {
    warnings.push('FBX materials are Phong: clearcoat, specular, index of refraction, emissive strength and unlit were not written');
  }
  return materials;
}

function buildTextures(document: SceneDocument): FbxJson[] {
  return document.textures.map((texture, index) => {
    const resource = document.resources[texture.resource];
    return {
      name: texture.name || resource?.name || `texture_${index}`,
      content: resource?.bytes ? Array.from(resource.bytes) : null,
      filename: resource?.name || null,
    };
  });
}

function buildAnimations(document: SceneDocument, space: FbxSpace, warnings: string[]): FbxJson[] {
  return document.animations.map((clip) => ({
    name: clip.name,
    duration: clip.duration,
    channels: clip.channels.flatMap((channel): FbxJson[] => {
      const sampler = clip.samplers[channel.sampler];
      if (!sampler || !document.nodes[channel.node]) return [];
      const input = readAccessorValues(document, sampler.input);
      const cubic = sampler.interpolation === 'CUBICSPLINE';
      const components = channel.path === 'rotation' ? 4 : channel.path === 'weights'
        ? document.accessors[sampler.output].components : 3;
      const values = readAccessorValues(document, sampler.output);
      const keyValues = cubic ? extractCubic(values, components, 1) : values;
      if (channel.path === 'weights') {
        // One glTF sampler interleaves every target's weight; FBX wants one
        // curve per target, so this fans out rather than returning a channel.
        const targetCount = components;
        if (cubic) {
          warnings.push(`Animation ${clip.name}: morph weight tangents were flattened for FBX export`);
        }
        return Array.from({ length: targetCount }, (_, target) => ({
          nodeName: document.nodes[channel.node].name,
          nodeId: channel.node + 1,
          morphTargetIndex: target,
          path: 'morphweight',
          sampler: {
            input,
            // Weights are normalized in SceneDocument; FBX samples percentages.
            output: keyValues.filter((_, index) => index % targetCount === target).map((value) => value * 100),
            // Linear regardless of the source: extractCubic has already dropped
            // the tangents by keeping only the value segment, unlike the direct
            // glTF-to-FBX path, which still holds them when it converts.
            interpolation: 'linear',
          },
        }));
      }
      let output;
      if (channel.path === 'rotation') output = quaternionKeysToFbxEuler(keyValues, space);
      else if (channel.path === 'translation') output = convertGltfVectorArrayToFbx(keyValues, space);
      else output = keyValues;
      if (output.length !== input.length * 3) {
        warnings.push(`Animation ${clip.name}: ${channel.path} sampler was omitted from FBX export`);
        return [];
      }
      return [{
        nodeName: document.nodes[channel.node].name,
        nodeId: channel.node + 1,
        path: channel.path,
        sampler: {
          input,
          output,
          interpolation: cubic && channel.path !== 'rotation' ? 'cubic' : 'linear',
          inTangents: null,
          outTangents: null,
        },
      }];
    }),
  }));
}

function extractCubic(values: number[], components: number, segment: number): number[] {
  const stride = components * 3;
  const output = [];
  for (let offset = 0; offset + stride <= values.length; offset += stride) output.push(...values.slice(offset + components * segment, offset + components * (segment + 1)));
  return output;
}

function convertNodeMatrix(node: FbxJson, space: FbxSpace): number[] {
  const matrix = node.matrix || composeMatrix(node.translation, node.rotation, node.scale);
  return convertGltfMatrixToFbx(matrix, space);
}

/** The same space with its unit factor removed, for the streams that only turn. */
function unitlessSpace(space: FbxSpace): FbxSpace {
  return space.metersPerUnit === 1 ? space : { ...space, metersPerUnit: 1 };
}

function buildDocumentWorlds(document: SceneDocument): number[][] {
  const worlds: (number[] | null)[] = Array.from({ length: document.nodes.length }, () => null);
  const visit = (index: number, parent: number[] | null) => {
    if (worlds[index]) return;
    const node = document.nodes[index] || {};
    const local = node.matrix || composeMatrix(node.translation, node.rotation, node.scale);
    worlds[index] = parent ? multiplyMat4(parent, local) : Array.from(local);
    for (const child of node.children || []) visit(child, worlds[index]);
  };
  for (const root of document.rootNodes) visit(root, null);
  document.nodes.forEach((_, index) => { if (!worlds[index]) visit(index, null); });
  return worlds as number[][];
}

function composeMatrix(
  translation: ArrayLike<number> = [0, 0, 0],
  rotation: ArrayLike<number> = [0, 0, 0, 1],
  scale: ArrayLike<number> = [1, 1, 1],
): number[] {
  const [x, y, z, w] = Array.from(rotation);
  const [sx, sy, sz] = Array.from(scale);
  return [
    (1 - 2 * (y * y + z * z)) * sx, (2 * (x * y + z * w)) * sx, (2 * (x * z - y * w)) * sx, 0,
    (2 * (x * y - z * w)) * sy, (1 - 2 * (x * x + z * z)) * sy, (2 * (y * z + x * w)) * sy, 0,
    (2 * (x * z + y * w)) * sz, (2 * (y * z - x * w)) * sz, (1 - 2 * (x * x + y * y)) * sz, 0,
    translation[0], translation[1], translation[2], 1,
  ];
}

function readAccessorValues(document: SceneDocument, index: number): number[] {
  const accessor = document.accessors[index];
  if (!accessor) return [];
  const bytes = componentByteWidth(accessor.componentType);
  if (!bytes) throw new Error(`Unsupported SceneDocument accessor component type ${accessor.componentType}`);
  const view = new DataView(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength);
  const values = [];
  for (let item = 0; item < accessor.count * accessor.components; item += 1) {
    const value = readComponent(view, item * bytes, accessor.componentType);
    values.push(accessor.normalized ? normalizeComponent(value, accessor.componentType) : value);
  }
  return values;
}

function readAccessorMatrices(document: SceneDocument, index: number): number[][] {
  const accessor = document.accessors[index];
  if (!accessor || accessor.components !== 16) return [];
  const values = readAccessorValues(document, index);
  return Array.from({ length: accessor.count }, (_, item) => values.slice(item * 16, item * 16 + 16));
}

function scaleValues(values: number[], factor: number): number[] {
  return Array.from(values, (value) => value * factor);
}

function flipUvV(values: number[]): number[] {
  const output = Array.from(values);
  for (let index = 1; index < output.length; index += 2) output[index] = 1 - output[index];
  return output;
}

function identityMatrix(): number[] {
  return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
}

function indexSourceNodes(roots: FbxJson[]): Map<string, FbxJson> {
  const map = new Map<string, FbxJson>();
  const visit = (node: FbxJson) => {
    if (node.name) map.set(node.name, node);
    for (const child of node.children || []) visit(child);
  };
  roots.forEach(visit);
  return map;
}

/**
 * Index the source meshes provenance can restore from, by name and by position.
 *
 * Name alone is not enough: an FBX Geometry need not be named, and the document
 * then synthesizes `mesh_<position>` for it. Keying only by name silently
 * skipped every unnamed geometry -- 229 of 655 meshes across the ufbx corpus --
 * so their UV, normal, colour and tangent layers, skin and control points all
 * fell back to the lossy portable path.
 *
 * The positional list is only usable when both traversals agree on how many
 * meshes there are; the document builder drops nodes it has no state for, which
 * would otherwise shift the pairing and restore the wrong mesh's layers.
 */
function indexSourceMeshes(roots: FbxJson[]): SourceMeshIndex {
  const byName = new Map<string, FbxJson>();
  const ordered: FbxJson[] = [];
  const visit = (node: FbxJson) => {
    for (const mesh of node.meshes || []) {
      if (mesh.name) byName.set(mesh.name, mesh);
      ordered.push(mesh);
    }
    for (const child of node.children || []) visit(child);
  };
  roots.forEach(visit);
  return { byName, ordered };
}

/** Resolve the source mesh a document mesh came from, or undefined. */
function findSourceMesh(
  sourceMeshes: SourceMeshIndex,
  documentMeshes: SceneDocument['meshes'],
  name: string | undefined,
  meshIndex: number,
  primitiveIndex: number,
): FbxJson {
  const byName = (name === undefined ? undefined : sourceMeshes.byName.get(name))
    || sourceMeshes.byName.get(`${name}_${primitiveIndex}`);
  if (byName) return byName;
  if (sourceMeshes.ordered.length !== documentMeshes.length) return undefined;
  const positional = sourceMeshes.ordered[meshIndex];
  // A named source mesh would already have matched above, so a name here
  // means the two orderings disagree and the pairing cannot be trusted.
  return positional && !positional.name ? positional : undefined;
}
