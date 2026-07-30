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

import {
  componentByteSize, isNormalizedIntegerType, morphDeltaAccessor, normalizeComponent, readComponent,
} from './component-values.ts';
import {
  GLTF_READER_RESOLVED_EXTENSIONS,
  gltfExtensionWarnings,
  isInterpretedGltfExtension,
  readGltfMaterial,
  resolveSampler,
  resolveTextureSource,
} from './gltf-interpretation.ts';
import { mimeFromUri, resolveResource, sniffMime } from './scene-resources.ts';
import type { ResourceMap } from './scene-resources.ts';
import {
  buildFbxMeshSkin,
  buildFbxMaterials,
  buildFbxMorphTargets,
  buildFbxSkins,
  buildFbxTextures,
  buildFbxWorldMatrices,
  convertGltfScaleKeysToFbx,
  convertGltfVectorArrayToFbx,
  fbxRowMajorMatrix,
  extractGltfCubicSegment,
  quaternionKeysToFbxEuler,
} from './fbx-scene-adapter.ts';
import { DEFAULT_FBX_EXPORT_SPACE, FBX_EXPORT_SPACES, fbxSpace } from './fbx-space.ts';
import type { FbxExportSpaceName, FbxSpace } from './fbx-space.ts';
import { invertMat4, multiplyMat4 } from './mat4.ts';
import { MATERIAL_EXTENSION_SLOTS, materialExtensionFactors } from './material-extensions.ts';
import { assertConverterProfile } from './wasm-modules.ts';
import type { GltfAsset, GltfModule, PackedAccessor, PackedGeometry } from './wasm-modules.ts';
import {
  MAX_ACTIVE_MORPH_TARGETS, VIEWER_LIMIT_WARNINGS, peakActiveMorphWeights,
} from './viewer-scene.ts';
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

/**
 * Extensions the FBX export route can act on.
 *
 * Only the ones the Rust reader resolves before JS sees them: their payload
 * arrives as ordinary geometry, so nothing is lost by writing FBX from it.
 * Every material and texture extension is genuinely dropped here, which is why
 * this set is narrower than either the preview's or the document's.
 */
const FBX_HONORED_EXTENSIONS: ReadonlySet<string> = GLTF_READER_RESOLVED_EXTENSIONS;


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
    assertConverterProfile(asset);
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
          const color = attributes.get('COLOR_0');
          const meshName = definitions[meshIndex]?.name || `mesh_${meshIndex}`;
          meshes.push({
            name: primitiveCount === 1 ? meshName : `${meshName}_${primitiveIndex}`,
            // Where this came from. A primitive with no positions or no
            // triangles is skipped above, so the list is not one entry per
            // declared primitive and cannot be walked by counting.
            meshIndex,
            primitiveIndex,
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
            colors: color && color.count === position.count
              ? rgbaBytes(packedAttributeNumbers(color), color.components)
              : null,
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

/** The roots of the document's scene, or every parentless node. */
function sceneRootNodes(document: GltfJson): number[] {
  const nodes: GltfJson[] = document.nodes || [];
  const sceneIndex = typeof document.scene === 'number' ? document.scene : 0;
  return document.scenes?.[sceneIndex]?.nodes
    || document.scenes?.[0]?.nodes
    || nodes
      .map((_: GltfJson, index: number) => index)
      .filter((index: number) => !nodes.some((node: GltfJson) => node.children?.includes(index)));
}

/** Every node's world matrix, column-major, in the document's own space. */
function gltfWorldMatrices(document: GltfJson): (number[] | null)[] {
  const nodes: GltfJson[] = document.nodes || [];
  const worlds: (number[] | null)[] = nodes.map(() => null);
  const visit = (index: number, parent: number[] | null) => {
    const node = nodes[index];
    if (!node || worlds[index]) return;
    const local = Array.isArray(node.matrix) && node.matrix.length === 16
      ? Array.from<number>(node.matrix)
      : composeTrs(
        node.translation || [0, 0, 0],
        node.rotation || [0, 0, 0, 1],
        node.scale || [1, 1, 1],
      );
    worlds[index] = parent ? multiplyMat4(parent, local) : local;
    for (const child of node.children || []) visit(child as number, worlds[index]);
  };
  for (const root of sceneRootNodes(document)) visit(root as number, null);
  return worlds;
}

/** Apply a column-major matrix to a flat `[x, y, z, ...]` array. */
function transformPositions(values: ArrayLike<number>, matrix: number[]): number[] {
  const out: number[] = new Array(values.length);
  for (let index = 0; index + 2 < values.length; index += 3) {
    const [x, y, z] = [values[index], values[index + 1], values[index + 2]];
    out[index] = matrix[0] * x + matrix[4] * y + matrix[8] * z + matrix[12];
    out[index + 1] = matrix[1] * x + matrix[5] * y + matrix[9] * z + matrix[13];
    out[index + 2] = matrix[2] * x + matrix[6] * y + matrix[10] * z + matrix[14];
  }
  return out;
}

/**
 * Apply a matrix to normals: the inverse transpose, renormalized.
 *
 * The matrix itself is only correct for normals when it carries no non-uniform
 * scale, and a node scaled on one axis is ordinary. A singular matrix has no
 * inverse to use, and there the rotation part is the best available answer.
 */
function transformNormals(values: ArrayLike<number>, matrix: number[]): number[] {
  const inverse = invertMat4(matrix);
  const out: number[] = new Array(values.length);
  for (let index = 0; index + 2 < values.length; index += 3) {
    const [x, y, z] = [values[index], values[index + 1], values[index + 2]];
    const transformed = inverse
      ? [
        inverse[0] * x + inverse[1] * y + inverse[2] * z,
        inverse[4] * x + inverse[5] * y + inverse[6] * z,
        inverse[8] * x + inverse[9] * y + inverse[10] * z,
      ]
      : [
        matrix[0] * x + matrix[4] * y + matrix[8] * z,
        matrix[1] * x + matrix[5] * y + matrix[9] * z,
        matrix[2] * x + matrix[6] * y + matrix[10] * z,
      ];
    const length = Math.hypot(...transformed) || 1;
    out[index] = transformed[0] / length;
    out[index + 1] = transformed[1] / length;
    out[index + 2] = transformed[2] / length;
  }
  return out;
}

/**
 * The scene's triangles, placed where the scene puts them.
 *
 * `buildFlatMeshesFromGltf` returns each mesh definition once, in the space its
 * accessors are written in. That is what the FBX route wants, because it
 * rebuilds the hierarchy and puts the transforms back. Every other flat target
 * — OBJ, PLY, STL, `.drc` — has nowhere to put a hierarchy, so the placement
 * has to be baked into the coordinates or the whole scene collapses onto the
 * origin: two objects a metre apart come out inside one another.
 *
 * A mesh several nodes instance is returned once per node, since that is what
 * the scene contains. A document with no scene graph at all is returned
 * untouched rather than emptied.
 */
export function buildFlatSceneMeshesFromGltf(
  sourceData: Uint8Array,
  resources: ResourceMap,
  gltfModule: GltfModule,
): GltfJson[] {
  const flat = buildFlatMeshesFromGltf(sourceData, resources, gltfModule);
  const asset = gltfModule.GltfAsset.withResources(sourceData, resources, '2.1');
  let document: GltfJson;
  try {
    document = JSON.parse(new TextDecoder().decode(asset.json()));
  } finally {
    asset.free();
  }

  const nodes: GltfJson[] = document.nodes || [];
  if (!nodes.some((node) => typeof node.mesh === 'number')) return flat;

  const byMesh = new Map<number, GltfJson[]>();
  for (const mesh of flat) {
    const list = byMesh.get(mesh.meshIndex) || [];
    list.push(mesh);
    byMesh.set(mesh.meshIndex, list);
  }

  const worlds = gltfWorldMatrices(document);
  const placed: GltfJson[] = [];
  nodes.forEach((node, index) => {
    if (typeof node.mesh !== 'number') return;
    const world = worlds[index];
    // Not reachable from the scene, so not part of what is being exported.
    if (!world) return;
    for (const mesh of byMesh.get(node.mesh) || []) {
      placed.push({
        ...mesh,
        name: node.name ? `${node.name}_${mesh.name}` : mesh.name,
        positions: transformPositions(mesh.positions, world),
        normals: mesh.normals ? transformNormals(mesh.normals, world) : null,
      });
    }
  });
  return placed;
}

/** Build the hierarchy and local transforms representable by FBX export. */
export function buildFbxSceneFromGltf(
  sourceData: Uint8Array,
  resources: ResourceMap,
  gltfModule: GltfModule,
  options: { legacyCompatibility?: boolean; space?: FbxExportSpaceName } = {},
) {
  const legacyCompatibility = options.legacyCompatibility === true;
  const space = fbxSpace(FBX_EXPORT_SPACES[options.space || DEFAULT_FBX_EXPORT_SPACE]);
  const asset = gltfModule.GltfAsset.withResources(sourceData, resources, '2.1');
  try {
    assertConverterProfile(asset);
    const document = JSON.parse(new TextDecoder().decode(asset.json()));
    const definitions = document.meshes || [];
    const flatMeshes = buildFlatMeshesFromGltf(sourceData, resources, gltfModule);
    const meshesByDefinition = definitions.map((definition: GltfJson, meshIndex: number) => {
      const primitives = definition.primitives || [];
      return flatMeshes.splice(0, primitives.length).map((mesh: GltfJson, primitiveIndex: number) => {
        const material = primitives[primitiveIndex]?.material;
        return {
          ...mesh,
          positions: convertGltfVectorArrayToFbx(mesh.positions, space),
          // Normals turn with the space and take none of its unit factor.
          normals: mesh.normals
            && convertGltfVectorArrayToFbx(mesh.normals, { ...space, metersPerUnit: 1 }),
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
            space,
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
    const worlds = buildFbxWorldMatrices(nodes, roots, composeTrs, space);
    const skins = buildFbxSkins(
      asset, document.skins || [], worlds, warnings, readAccessorAsTyped, composeTrs, space,
    );
    const buildNode = (index: number, isRoot = false): GltfJson => {
      const node = nodes[index] || {};
      return {
        id: index + 1,
        name: node.name || `node_${index}`,
        matrix: scaleFbxRootMatrix(
          fbxRowMajorMatrix(node, composeTrs, space),
        ),
        meshes: typeof node.mesh === 'number'
          ? (meshesByDefinition[node.mesh] || []).map((mesh: GltfJson) => ({
            ...mesh,
            skin: typeof node.skin === 'number'
              ? buildFbxMeshSkin(
                mesh, skins[node.skin], index + 1, worlds[index], composeTrs, space,
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
    // FBX honors a far narrower set than the preview or the portable document:
    // it has no unlit, no clearcoat, no specular or ior, and no UV transform.
    // The predicate is a parameter precisely so each consumer can state its
    // own reach rather than inherit someone else's.
    warnings.push(...gltfExtensionWarnings(
      document,
      (extension) => FBX_HONORED_EXTENSIONS.has(extension),
      {
        ignored: (names) => `glTF extensions the FBX writer cannot express: ${names}`,
        required: 'This model requires glTF extensions that FBX cannot express; the exported scene is incomplete',
      },
    ));
    return {
      // The file has to say which space it is in, and this is the only place
      // that knows: the writer's defaults are a guess for callers that pass
      // nothing, not a description of what was written here.
      globalSettings: { ...space.settings },
      rootNodes: roots.map((root: number) => buildNode(root, true)),
      materials: buildFbxMaterials(document.materials || [], warnings),
      textures: buildFbxTextures(asset, document, resources, resolveUriBytes, warnings),
      animations: buildFbxAnimations(
        asset, document.animations || [], nodeRecords, warnings, space, legacyCompatibility,
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
  space: FbxSpace,
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
        ? quaternionKeysToFbxEuler(keyValues, space)
        : channel.path === 'translation'
          ? convertGltfVectorArrayToFbx(keyValues, space)
          : convertGltfScaleKeysToFbx(keyValues, space);
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

/**
 * `COLOR_0` as the RGBA bytes every flat writer wants.
 *
 * glTF states colours as floats in 0..1, or as normalized integers that
 * `packedAttributeNumbers` has already brought to the same range, so one scale
 * covers both. A three-component set is opaque: glTF says so, and PLY and Draco
 * have no way to spell "no alpha stated".
 */
function rgbaBytes(values: number[], components: number): number[] {
  const byte = (value: number) => Math.round(Math.min(Math.max(value, 0), 1) * 255);
  const out: number[] = [];
  for (let index = 0; index + components <= values.length; index += components) {
    out.push(
      byte(values[index]),
      byte(values[index + 1]),
      byte(values[index + 2]),
      components >= 4 ? byte(values[index + 3]) : 255,
    );
  }
  return out;
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
export function extensionWarnings(document: GltfJson, honoredSources: Map<unknown, unknown>): string[] {
  return gltfExtensionWarnings(
    document,
    // An alternate-source extension counts as honored only when every texture
    // that read through it produced a bitmap: the codec belongs to the host.
    (extension) => isInterpretedGltfExtension(extension) || honoredSources.get(extension) === true,
    {
      ignored: (names) => `Unsupported glTF extensions ignored: ${names}`,
      required: 'Model requires extensions that this viewer ignores; rendering may be incomplete',
    },
  );
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
          const target = morphDeltaAccessor(
            readAccessorAsTyped(asset, accessorIndex), attributes.POSITION.count,
          );
          if (!target) {
            warnings.push(VIEWER_LIMIT_WARNINGS.morphTarget(targetIndex, meshIndex, p));
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
          const target = morphDeltaAccessor(
            readAccessorAsTyped(asset, accessorIndex), attributes.POSITION.count,
          );
          if (!target) {
            warnings.push(VIEWER_LIMIT_WARNINGS.morphNormal(targetIndex, meshIndex, p));
            primitive.morphNormals.push(null);
            continue;
          }
          primitive.morphNormals.push(target);
        }
        if (targets.some((target: GltfJson) => typeof target.TANGENT === 'number')) {
          warnings.push(VIEWER_LIMIT_WARNINGS.morphTangents(meshIndex, p));
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
      warnings.push(VIEWER_LIMIT_WARNINGS.morphMeshWeights(mesh.name, activeTargets));
    }
  }
}

/**
 * Read animation sampler values as unit-range floats.
 *
 * glTF stores rotations and morph weights as normalized integers as well, and
 * every consumer downstream — interpolation, weight sorting, FBX export —
 * assumes the float spelling.
 */
function samplerValues(accessor: ReturnType<typeof readAccessorAsTyped>) {
  if (!accessor.normalized || !isNormalizedIntegerType(accessor.componentType)) return accessor.data;
  const values = new Float32Array(accessor.data.length);
  for (let i = 0; i < values.length; i++) {
    values[i] = normalizeComponent(accessor.data[i], accessor.componentType);
  }
  return values;
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

/**
 * Flatten glTF material definitions into the flat records the renderer binds.
 *
 * Exported for the material smoke test: extension defaults are the difference
 * between a coated surface and a flat one, and they are easier to pin here than
 * through a rendered frame.
 */
export function buildMaterials(defs: GltfJson[]): GltfJson[] {
  const fallback = {
    baseColorFactor: [1, 1, 1, 1],
    doubleSided: false,
    alphaMode: 'OPAQUE',
    ...materialExtensionFactors(null),
    ...Object.fromEntries(MATERIAL_EXTENSION_SLOTS.map(({ property }) => [property, null])),
  };
  const list: GltfJson[] = defs.map((def, idx) => {
    const material = readGltfMaterial(def, idx);
    return {
      name: material.name,
      baseColorFactor: material.baseColorFactor,
      // The renderer addresses base color through three separate uniforms, so
      // this one slot is flattened while the rest pass through as bindings.
      baseColorTexture: material.baseColorTexture?.index ?? null,
      baseColorTexCoord: material.baseColorTexture?.texCoord ?? 0,
      baseColorTextureTransform: material.baseColorTexture?.transform
        ?? { offset: [0, 0], scale: [1, 1], rotation: 0 },
      metallic: material.metallicFactor,
      roughness: material.roughnessFactor,
      metallicRoughnessTexture: material.metallicRoughnessTexture,
      emissiveFactor: material.emissiveFactor,
      emissiveStrength: material.emissiveStrength,
      emissiveTexture: material.emissiveTexture,
      normalTexture: material.normalTexture,
      occlusionTexture: material.occlusionTexture,
      doubleSided: material.doubleSided,
      alphaMode: material.alphaMode,
      alphaCutoff: material.alphaCutoff,
      // The layered extensions pass through by the names the table gives them,
      // so this producer and the SceneDocument adapter cannot end up carrying
      // different sets - which is what `gltf-material-parity` compares.
      ...materialExtensionFactors(material),
      ...Object.fromEntries(MATERIAL_EXTENSION_SLOTS.map(({ property }) => [property, material[property]])),
    };
  });
  list.push(fallback);
  return list;
}

async function buildTextures(asset: GltfAsset, manifest: GltfJson, resources: ResourceMap, hooks: ImportHooks) {
  const images = await decodeImages(asset, manifest.images || [], resources, hooks);
  const samplers = (manifest.samplers || []).map(resolveSampler);
  const defaultSampler = resolveSampler(null);

  // An alternate-source extension is honored only while every texture that
  // reads through it produced a bitmap: the codec belongs to the host, so
  // this is an observation rather than a support claim.
  const honoredSources = new Map();
  const textures = (manifest.textures || []).map((tex: GltfJson, idx: number) => {
    const samplerIndex = typeof tex.sampler === 'number' ? tex.sampler : -1;
    const sampler = samplerIndex >= 0 ? samplers[samplerIndex] : defaultSampler;
    const { source, extension } = resolveTextureSource(tex);
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

export function buildAnimations(asset: GltfAsset, defs: GltfJson[], nodes: ViewerNode[], warnings: string[]): GltfJson[] {
  return defs.map((def, animIndex) => {
    const name = def.name || `animation_${animIndex}`;
    const samplers = (def.samplers || []).map((s: GltfJson) => {
      const input = readAccessorAsTyped(asset, s.input);
      const output = readAccessorAsTyped(asset, s.output);
      return {
        input: input.data,
        output: samplerValues(output),
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
          warnings.push(VIEWER_LIMIT_WARNINGS.morphKeyframeWeights(name, active));
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
    // KHR_mesh_quantization stores POSITION as a normalized integer as
    // readily as a float, and the GPU reads it through the same flag, so the
    // box has to agree with what is drawn or the camera frames a model
    // 32767 times its size.
    const scale = pos.normalized && isNormalizedIntegerType(pos.componentType)
      ? (value: number) => normalizeComponent(value, pos.componentType)
      : (value: number) => value;
    for (let i = 0; i < pos.count; i++) {
      const x = scale(view[i * components]);
      const y = scale(view[i * components + 1]);
      const z = scale(view[i * components + 2]);
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
