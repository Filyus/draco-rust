/**
 * FBX scene export boundary for the web application.
 *
 * This module deliberately has no dependency on glTF loader code. It owns the
 * FBX representation and format-specific material/texture mapping — but it
 * reads glTF material and texture JSON through the one shared interpreter, so
 * this export route resolves extensions and defaults exactly as the preview
 * and the portable document do.
 */

import { identityMat4, invertMat4 } from './mat4.ts';
import { hasMaterialExtensionValues } from './material-extensions.ts';
import { readGltfMaterial, resolveSampler, resolveTextureSource } from './gltf-interpretation.ts';
import type { InterpretedTexture } from './gltf-interpretation.ts';
import { multiplyQuaternion } from './fbx-space.ts';
import type { FbxSpace } from './fbx-space.ts';
import type { ResourceMap } from './scene-resources.ts';
import type { GltfAsset, NumericArray } from './wasm-modules.ts';

/** glTF manifest fragments, inspected field by field rather than trusted. */
type GltfJson = any;

/** Composes a glTF TRS triple into a column-major matrix. */
type ComposeTrs = (
  translation: ArrayLike<number>,
  rotation: ArrayLike<number>,
  scale: ArrayLike<number>,
) => ArrayLike<number>;

/** Reads one glTF accessor into a typed array plus its declared shape. */
type ReadAccessorAsTyped = (asset: GltfAsset, index: number) => {
  componentType: number;
  components: number;
  count: number;
  data: NumericArray;
};

/** A row-major FBX matrix, in the writer's own convention. */
type FbxMatrix = number[];

/** Key payloads arrive either as typed accessor data or as plain JSON arrays. */
type Numbers = Float32Array | number[];

interface FbxJoint {
  nodeId: number;
  bind: FbxMatrix;
}

export interface FbxSkin {
  joints: FbxJoint[];
  armatureBindTransform: FbxMatrix | null;
}

interface FbxCluster {
  jointNodeId: number;
  controlPointIndices: number[];
  weights: number[];
  meshBindTransform: FbxMatrix;
  jointBindTransform: FbxMatrix;
  armatureBindTransform: FbxMatrix | null;
}

/** The mesh geometry the FBX writer consumes, in glTF component order. */
interface FbxSourceMesh {
  positions: ArrayLike<number>;
  joints0?: ArrayLike<number>;
  weights0?: ArrayLike<number>;
  joints1?: ArrayLike<number>;
  weights1?: ArrayLike<number>;
}

// The bases used to be a pair of constants here, for the one Z-up convention
// this writer emitted. They come from the target space now, because the space is
// a choice -- and because the importer resolves the same seven GlobalSettings
// fields, so having one derivation is what keeps the two directions inverses.
//
// `FbxSpace` stores its basis column-major for column vectors; FBX uses row
// vectors, and for a signed permutation the two storage orders differ by a
// transpose, which is also what changes a column-vector matrix into a row-vector
// one. The two cancel, so the same sixteen numbers serve both readings.

function multiplyMat4(a: ArrayLike<number>, b: ArrayLike<number>): FbxMatrix {
  return Array.from({ length: 16 }, (_, index) => {
    const row = Math.floor(index / 4); const column = index % 4;
    return a[row * 4] * b[column] + a[row * 4 + 1] * b[column + 4]
      + a[row * 4 + 2] * b[column + 8] + a[row * 4 + 3] * b[column + 12];
  });
}

/** glTF vectors in the target space, and in its units. */
export function convertGltfVectorArrayToFbx(
  values: ArrayLike<number>,
  space: FbxSpace,
): number[] {
  const scale = 1 / space.metersPerUnit;
  if (space.identity && scale === 1) return Array.from(values);
  const basis = space.inverse;
  const converted = Array.from(values);
  for (let offset = 0; offset + 2 < converted.length; offset += 3) {
    const [x, y, z] = [converted[offset], converted[offset + 1], converted[offset + 2]];
    converted[offset] = (x * basis[0] + y * basis[4] + z * basis[8]) * scale;
    converted[offset + 1] = (x * basis[1] + y * basis[5] + z * basis[9]) * scale;
    converted[offset + 2] = (x * basis[2] + y * basis[6] + z * basis[10]) * scale;
  }
  return converted;
}

/**
 * A glTF transform in the target space.
 *
 * `G⁻¹ · M · G` with row vectors, which is the same conjugation the importer
 * performs in the other direction: a transform has to receive and return points
 * that are already in the space it is written into.
 */
export function convertGltfMatrixToFbx(matrix: ArrayLike<number>, space: FbxSpace): FbxMatrix {
  const conjugated = space.identity
    ? Array.from(matrix, Number)
    : multiplyMat4(multiplyMat4(space.basis, matrix), space.inverse);
  const scale = 1 / space.metersPerUnit;
  conjugated[12] *= scale;
  conjugated[13] *= scale;
  conjugated[14] *= scale;
  return conjugated;
}

/** The PBR values an FBX Phong material is lowered from, whatever read them. */
export interface PhongSource {
  name: string;
  baseColorFactor: number[];
  emissiveFactor: number[];
  metallicFactor: number;
  roughnessFactor: number;
  textures: { slot: string; textureIndex: number }[];
}

/**
 * Lower a PBR material to FBX Phong properties.
 *
 * Shared by both FBX export routes — from a glTF document and from a
 * SceneDocument — because the mapping is a property of FBX, not of whichever
 * model happened to be read.
 */
export function lowerPbrToFbxPhong(source: PhongSource) {
  return {
    name: source.name,
    shadingModel: 'Phong',
    diffuse: source.baseColorFactor.slice(0, 3),
    diffuseFactor: 1,
    emissive: [...source.emissiveFactor],
    emissiveFactor: 1,
    reflectionFactor: source.metallicFactor,
    shininess: Math.max(0, Math.min(1, source.roughnessFactor)) * -100 + 100,
    opacity: source.baseColorFactor[3] ?? 1,
    textures: source.textures,
  };
}

/**
 * Convert glTF materials to FBX Phong, reporting what FBX cannot carry.
 *
 * Read through the shared interpreter rather than off the JSON, so this route
 * resolves extensions and defaults the same way the preview and the portable
 * document do. FBX's material model is Phong plus texture slots, so the
 * layered extensions and per-slot texture transforms have nowhere to go; the
 * warnings say so once per kind rather than once per material.
 */
export function buildFbxMaterials(definitions: GltfJson[], warnings: string[] = []) {
  let droppedLayers = false;
  let droppedTransforms = false;
  const materials = definitions.map((definition, index) => {
    const source = readGltfMaterial(definition, index);
    const textures: { slot: string; textureIndex: number }[] = [];
    const add = (binding: InterpretedTexture | null, slot: string) => {
      if (!binding) return;
      textures.push({ slot, textureIndex: binding.index });
      if (binding.transform) droppedTransforms = true;
    };
    add(source.baseColorTexture, 'diffuse');
    add(source.normalTexture, 'normal');
    add(source.emissiveTexture, 'emissive');
    add(source.metallicRoughnessTexture, 'roughness');
    // One question with one answer: what the material states beyond the core
    // metallic-roughness model is exactly what a Phong material cannot carry.
    if (hasMaterialExtensionValues(source)) droppedLayers = true;
    return lowerPbrToFbxPhong({
      name: source.name,
      baseColorFactor: source.baseColorFactor,
      emissiveFactor: source.emissiveFactor,
      metallicFactor: source.metallicFactor,
      roughnessFactor: source.roughnessFactor,
      textures,
    });
  });
  if (droppedLayers) {
    warnings.push('FBX materials are Phong: clearcoat, specular, index of refraction, emissive strength and unlit were not written');
  }
  if (droppedTransforms) {
    warnings.push('FBX textures carry no UV transform: KHR_texture_transform offsets and scales were not written');
  }
  return materials;
}

/**
 * Preserve embedded or external glTF image data for an FBX Texture/Video.
 *
 * Never drops an entry: materials address this array by position, so a texture
 * that could not be resolved has to stay as an empty slot rather than shift
 * every later index by one.
 */
export function buildFbxTextures(
  asset: GltfAsset,
  document: GltfJson,
  resources: ResourceMap,
  resolveUriBytes: (uri: string, resources: ResourceMap) => Uint8Array | null,
  warnings: string[] = [],
) {
  const images: GltfJson[] = document.images || [];
  const samplers: GltfJson[] = document.samplers || [];
  let alternateSource = false;
  let droppedSamplers = false;
  const textures = (document.textures || []).map((texture: GltfJson, index: number) => {
    const { source, extension } = resolveTextureSource(texture);
    if (extension) alternateSource = true;
    const sampler = resolveSampler(Number.isInteger(texture.sampler) ? samplers[texture.sampler] : null);
    // 10497 is REPEAT, which is also what FBX assumes; anything else is a
    // wrap mode the format has no field for.
    if (sampler.wrapS !== 10497 || sampler.wrapT !== 10497) droppedSamplers = true;
    const image = images[source] || {};
    const content = typeof image.bufferView === 'number'
      ? Array.from(new Uint8Array(asset.bufferViewBytes(image.bufferView)))
      : (image.uri ? Array.from(resolveUriBytes(image.uri, resources) || []) : null);
    return { name: texture.name || image.name || `texture_${index}`, content, filename: image.uri || null };
  });
  if (alternateSource) {
    warnings.push('Textures encoded as WebP, AVIF or KTX2 were written to FBX unchanged; most importers cannot decode them');
  }
  if (droppedSamplers) {
    warnings.push('FBX textures carry no wrap mode: clamp and mirror settings were not written');
  }
  return textures;
}

/**
 * Convert glTF quaternion key values to FBX XYZ Euler key values.
 *
 * The basis change comes first, and it comes from the target space. It used to
 * be written out by hand as `(x, z, -y)` — the one Z-up conversion this writer
 * emitted — which was right for exactly that space and silently wrong for any
 * other. Once the space became a choice, a Y-up export was turning its animated
 * rotations by ninety degrees while leaving everything else alone.
 *
 * Still a quaternion product rather than a matrix per frame, which is what the
 * hand-written form was for.
 */
export function quaternionKeysToFbxEuler(values: Numbers, space: FbxSpace): number[] {
  const result: number[] = [];
  // glTF -> FBX, so the rotation is conjugated by the inverse of the basis the
  // importer reads: q' = b* q b.
  const [bx, by, bz, bw] = space.rotation;
  const conjugate = [-bx, -by, -bz, bw];
  for (let index = 0; index + 3 < values.length; index += 4) {
    const key = Array.from(values.slice(index, index + 4));
    const [qx, qy, qz, w] = space.identity
      ? key
      : multiplyQuaternion(multiplyQuaternion(conjugate, key), space.rotation);
    const euler = [
      Math.atan2(2 * (w * qx + qy * qz), 1 - 2 * (qx * qx + qy * qy)),
      Math.asin(Math.max(-1, Math.min(1, 2 * (w * qy - qz * qx)))),
      Math.atan2(2 * (w * qz + qx * qy), 1 - 2 * (qy * qy + qz * qz)),
    ];
    // Legacy FBX's Euler evaluator does not pick an equivalent branch.
    // Unwrap each component so consecutive keys remain continuous.
    for (let component = 0; component < 3; component += 1) {
      const previous = result.length >= 3 ? result[result.length - 3 + component] : euler[component];
      while (euler[component] - previous > Math.PI) euler[component] -= Math.PI * 2;
      while (euler[component] - previous < -Math.PI) euler[component] += Math.PI * 2;
    }
    result.push(...euler);
  }
  return result;
}

/**
 * Animated per-axis scale, in the target space.
 *
 * Scale follows the axes and not their signs, so this is a permutation rather
 * than a rotation: glTF component `r` lands on FBX axis `axes[r]`. It used to be
 * a hand-written Y/Z swap in the glTF route and nothing at all in the document
 * route -- the same split the rotation keys had, found by looking for the rest
 * of it.
 */
export function convertGltfScaleKeysToFbx(values: ArrayLike<number>, space: FbxSpace): number[] {
  const converted = Array.from(values);
  if (space.identity) return converted;
  for (let offset = 0; offset + 2 < converted.length; offset += 3) {
    for (let component = 0; component < 3; component += 1) {
      converted[offset + space.axes[component]] = values[offset + component];
    }
  }
  return converted;
}

/** Split glTF CUBICSPLINE [in, value, out] key payloads. */
export function extractGltfCubicSegment(values: Numbers, components: number, segment: number): number[] {
  const result: number[] = [];
  const stride = components * 3;
  for (let offset = 0; offset + stride <= values.length; offset += stride) {
    result.push(...values.slice(offset + components * segment, offset + components * (segment + 1)));
  }
  return result;
}

export function fbxRowMajorMatrix(
  node: GltfJson,
  composeTrs: ComposeTrs,
  space: FbxSpace,
): FbxMatrix {
  const gltfMatrix = Array.from<number>(Array.isArray(node.matrix) && node.matrix.length === 16
    ? node.matrix
    : composeTrs(node.translation || [0, 0, 0], node.rotation || [0, 0, 0, 1], node.scale || [1, 1, 1]));
  return convertGltfMatrixToFbx(gltfMatrix, space);
}

export function buildFbxWorldMatrices(
  nodes: GltfJson[],
  roots: number[],
  composeTrs: ComposeTrs,
  space: FbxSpace,
): (FbxMatrix | null)[] {
  const worlds: (FbxMatrix | null)[] = Array.from({ length: nodes.length }, () => null);
  // Scale is carried entirely by `UnitScaleFactor = 100.0` in the writer's
  // GlobalSettings (Blender reads it as the centimeters->meters factor).
  // Scaling coordinates here as well makes the imported scene 100× too
  // large, which was the original "legacy FBX" workaround that is no
  // longer needed.
  const visit = (index: number, parent: FbxMatrix | null) => {
    if (worlds[index]) return;
    const local = fbxRowMajorMatrix(nodes[index] || {}, composeTrs, space);
    // FBX uses row vectors. The local transform therefore precedes its
    // parent's transform in the composed world matrix.
    worlds[index] = parent ? multiplyMat4(local, parent) : local;
    for (const child of nodes[index]?.children || []) visit(child as number, worlds[index]);
  };
  for (const root of roots) visit(root, null);
  nodes.forEach((_, index) => { if (!worlds[index]) visit(index, null); });
  return worlds;
}

/** Turn glTF skin accessors into FBX clusters without truncating influences. */
export function buildFbxSkins(
  asset: GltfAsset,
  definitions: GltfJson[],
  worlds: (FbxMatrix | null)[],
  warnings: string[],
  readAccessorAsTyped: ReadAccessorAsTyped,
  composeTrs: ComposeTrs,
  space: FbxSpace,
): FbxSkin[] {
  return definitions.map((definition, index) => {
    const joints: FbxJoint[] = (definition.joints || []).map((nodeIndex: number) => ({
      nodeId: nodeIndex + 1,
      bind: worlds[nodeIndex] || fbxRowMajorMatrix({}, composeTrs, space),
    }));
    if (typeof definition.inverseBindMatrices === 'number') {
      const accessor = readAccessorAsTyped(asset, definition.inverseBindMatrices);
      if (accessor.componentType !== 5126 || accessor.components !== 16) {
        warnings.push(`Skin ${index} has unsupported inverse bind matrices`);
      } else {
        for (let jointIndex = 0; jointIndex < joints.length && jointIndex < accessor.count; jointIndex += 1) {
          const inverse = Array.from(accessor.data.subarray(jointIndex * 16, jointIndex * 16 + 16));
          const bind = invertMat4(convertGltfMatrixToFbx(inverse, space)) || joints[jointIndex].bind;
          joints[jointIndex].bind = bind;
        }
      }
    }
    return {
      joints,
      // glTF has joints but no Armature object. Blender's legacy FBX
      // importer interprets TransformAssociateModel as the Armature
      // object's world matrix, not the root joint's matrix. The latter
      // makes the legacy importer apply the root transform twice.
      // Identity is correct in both legacy and standard paths now that
      // scale is carried by UnitScaleFactor instead of by the matrix.
      armatureBindTransform: null,
    };
  });
}

/** Attach all JOINTS_0/1 and WEIGHTS_0/1 influences to FBX skin clusters. */
export function buildFbxMeshSkin(
  mesh: FbxSourceMesh,
  skin: FbxSkin | null,
  meshNodeId: number,
  meshBindTransform: FbxMatrix | null,
  composeTrs: ComposeTrs,
  space: FbxSpace,
) {
  if (!skin || !mesh.joints0 || !mesh.weights0) return null;
  const meshBind = meshBindTransform || fbxRowMajorMatrix({}, composeTrs, space);
  const clusters: FbxCluster[] = skin.joints.map((joint) => ({
    jointNodeId: joint.nodeId,
    controlPointIndices: [],
    weights: [],
    // Blender's importer reconstructs the mesh bind matrix as
    // `TransformLink @ Transform`. Solving for `Transform` gives
    // `TransformLink⁻¹ @ MeshWorldBind`; `multiplyMat4(a, b)` is row-major
    // `a · b`, whose column-major equivalent is `b @ a`, so
    // `meshBind · inverse(joint.bind)` yields the required
    // `inverse(TransformLink) @ MeshWorldBind`.
    meshBindTransform: multiplyMat4(meshBind, invertMat4(joint.bind) || identityMat4()),
    jointBindTransform: joint.bind,
    armatureBindTransform: skin.armatureBindTransform || null,
  }));
  const vertexCount = mesh.positions.length / 3;
  for (const [joints, weights] of [[mesh.joints0, mesh.weights0], [mesh.joints1, mesh.weights1]]) {
    if (!joints || !weights) continue;
    for (let vertex = 0; vertex < vertexCount; vertex += 1) {
      for (let component = 0; component < 4; component += 1) {
        const weight = Number(weights[vertex * 4 + component]) || 0;
        const joint = Number(joints[vertex * 4 + component]);
        if (weight > 0 && Number.isInteger(joint) && clusters[joint]) {
          clusters[joint].controlPointIndices.push(vertex);
          clusters[joint].weights.push(weight);
        }
      }
    }
  }
  return {
    clusters: clusters.filter((cluster) => cluster.weights.length > 0),
    // Native Blender's importer needs the mesh Model as well as every
    // joint in BindPose to construct an armature modifier.
    bindPose: [
      { nodeId: meshNodeId, matrix: meshBind },
      ...skin.joints.map((joint) => ({ nodeId: joint.nodeId, matrix: joint.bind })),
    ],
  };
}

/** Decode glTF morph deltas into the FBX shape contract. */
export function buildFbxMorphTargets(
  asset: GltfAsset,
  targetDefinitions: GltfJson[],
  weights: ArrayLike<number>,
  readAccessorAsTyped: ReadAccessorAsTyped,
  space: FbxSpace,
) {
  return targetDefinitions.flatMap((target, index) => {
    if (typeof target.POSITION !== 'number') return [];
    const accessor = readAccessorAsTyped(asset, target.POSITION);
    if (accessor.componentType !== 5126 || accessor.components !== 3) return [];
    let normalDeltas: number[] | null = null;
    if (typeof target.NORMAL === 'number') {
      const normal = readAccessorAsTyped(asset, target.NORMAL);
      if (normal.componentType === 5126 && normal.components === 3 && normal.count === accessor.count) normalDeltas = Array.from(normal.data);
    }
    return [{
      name: `target_${index}`,
      controlPointIndices: Array.from({ length: accessor.count }, (_, point) => point),
      positionDeltas: convertGltfVectorArrayToFbx(accessor.data, space),
      // Normals turn with the space and take none of its unit factor.
      normalDeltas: normalDeltas
        && convertGltfVectorArrayToFbx(normalDeltas, { ...space, metersPerUnit: 1 }),
      defaultWeight: Number(weights[index]) || 0,
      fullWeight: 100,
    }];
  });
}
