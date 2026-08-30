/**
 * Format-neutral flat mesh adapter for the viewer.
 *
 * Semantic FBX import lives in fbx-import-scene.js. glTF has its own document
 * adapter in gltf-loader.js; both converge only on viewer's shared Scene.
 */

import { buildSceneFromFbx as buildSemanticFbxScene } from './fbx-import-scene.ts';
import { basename, mimeFromUri, resolveResource } from './scene-resources.ts';
import type { ResourceMap } from './scene-resources.ts';
import type {
  Aabb, Renderable, RuntimeAccessor, Trs, ViewerMesh, ViewerNode, ViewerPrimitive, ViewerTexture,
} from './viewer-scene.ts';

/**
 * One FBX layer element as the reader hands it over: UVs, normals or colours
 * addressed by the file's own mapping and reference modes. Carried through the
 * export path untouched, which is why nothing here is interpreted.
 */
export interface LoadedLayerSet {
  name?: string;
  mapping?: string;
  reference?: string;
  values?: ArrayLike<number>;
  indices?: ArrayLike<number>;
}

/**
 * An attribute a reader carried but did not interpret.
 *
 * Draco puts no limit on how many attributes of a type a payload holds, and the
 * flat mesh has one slot per named type and none for generics. Rather than
 * decode those and drop them, the reader hands them over whole — type,
 * component count, component type and the id a consumer addresses them by —
 * and the writer that understands the format puts them back unchanged. Nothing
 * between the two reads their meaning, which is what makes it safe: a generic
 * attribute holding skin weights travels the same way as a second UV set.
 */
export interface OpaqueAttribute {
  type: string;
  components: number;
  dataType: string;
  uniqueId: number;
  normalized: boolean;
  /** One tuple per vertex, `components` long. */
  values: ArrayLike<number>;
}

/**
 * One mesh as the flat readers hand it over.
 *
 * Numeric fields cross the wasm boundary as plain or typed arrays depending on
 * the reader, so they are read as `ArrayLike` and copied wherever a real array
 * is needed. Everything is optional: PLY has no material name, OBJ has no layer
 * elements, and a mesh may legitimately be non-indexed.
 */
export interface LoadedMesh {
  name?: string;
  positions?: ArrayLike<number>;
  indices?: ArrayLike<number> | null;
  normals?: ArrayLike<number> | null;
  uvs?: ArrayLike<number> | null;
  colors?: ArrayLike<number> | null;
  /** Skin influences; the preview uses the first set and warns about a second. */
  joints0?: ArrayLike<number> | null;
  weights0?: ArrayLike<number> | null;
  joints1?: ArrayLike<number> | null;
  weights1?: ArrayLike<number> | null;
  /** OBJ only: the `usemtl` name to look up in the companion library. */
  material?: string;
  /** FBX only, and passed straight back out to the FBX writer. */
  controlPoints?: ArrayLike<number> | null;
  polygonVertexIndices?: ArrayLike<number> | null;
  materialIndices?: number[];
  skin?: unknown;
  morphTargets?: unknown[];
  uvSets?: LoadedLayerSet[];
  normalSets?: LoadedLayerSet[];
  colorSets?: LoadedLayerSet[];
  /** Draco only: whatever the payload carried that no slot above names. */
  extras?: OpaqueAttribute[];
}

/** A material as the OBJ companion-library reader builds it. */
export interface LoadedObjMaterial {
  diffuse?: number[];
  alpha?: number;
  baseColorTextureUri?: string;
}

/** Flat mesh output from the OBJ, PLY and legacy FBX readers. */
export interface ParsedMeshes {
  meshes?: LoadedMesh[];
  materials?: Record<string, LoadedObjMaterial>;
  warnings?: string[];
}

/** A viewer mesh plus the two fields only this flat path attaches to it. */
interface FlatSceneMesh extends ViewerMesh {
  morphTargets: unknown[];
  _defaultMaterial: FlatMaterial;
}

/** Diagnostics sink shared with the rest of the import path. */
interface ImportHooks {
  onLog?: (message: string, level: string) => void;
}

/**
 * The viewer material as these flat formats build it, before the texture pass
 * resolves the URI into an uploaded index.
 */
interface FlatMaterial {
  baseColorFactor: number[];
  doubleSided: boolean;
  alphaMode: string;
  unlit: boolean;
  baseColorTextureUri?: string;
  baseColorTexture?: number;
}

function pushAabb(box: Aabb, x: number, y: number, z: number) {
  if (x < box.min[0]) box.min[0] = x;
  if (y < box.min[1]) box.min[1] = y;
  if (z < box.min[2]) box.min[2] = z;
  if (x > box.max[0]) box.max[0] = x;
  if (y > box.max[1]) box.max[1] = y;
  if (z > box.max[2]) box.max[2] = z;
}

/** Build a Scene from flat mesh primitives emitted by OBJ/PLY/legacy FBX. */
export async function buildSceneFromMeshes(
  parsed: ParsedMeshes,
  resources: ResourceMap = Object.create(null),
  hooks: ImportHooks = {},
) {
  const meshes = parsed?.meshes || [];
  if (meshes.length === 0) throw new Error('No meshes were decoded from this file');
  const sceneMeshes: FlatSceneMesh[] = [];
  const box: Aabb = { min: [Infinity, Infinity, Infinity], max: [-Infinity, -Infinity, -Infinity] };

  for (const mesh of meshes) {
    const positions = Float32Array.from(mesh.positions || []);
    const vertexCount = positions.length / 3;
    const indices = mesh.indices ? Uint32Array.from(mesh.indices) : null;
    const normals = mesh.normals?.length === positions.length ? Float32Array.from(mesh.normals) : null;
    const uvs = mesh.uvs?.length === vertexCount * 2 ? Float32Array.from(mesh.uvs) : null;
    // Two domains reach here. PLY and DRC hand over bytes in 0..255; FBX hands
    // over the format's own floats in 0..1. Reading the second as the first
    // truncates every channel to 0 or 1 and then normalizes it by 255, so the
    // vertex colour a base colour is multiplied by becomes 0.004 and the model
    // renders black -- which is what a fully textured character did.
    const colorSource = mesh.colors && mesh.colors.length > 0 ? mesh.colors : null;
    const colorsAreFloat = colorSource instanceof Float32Array;
    const colors = colorSource
      ? (colorsAreFloat ? Float32Array.from(colorSource) : Uint8Array.from(colorSource))
      : null;
    const joints = mesh.joints0?.length === vertexCount * 4 ? Uint16Array.from(mesh.joints0) : null;
    const weights = mesh.weights0?.length === vertexCount * 4 ? Float32Array.from(mesh.weights0) : null;
    if (mesh.joints1?.length === vertexCount * 4 || mesh.weights1?.length === vertexCount * 4) {
      parsed.warnings ||= [];
      parsed.warnings.push(`Mesh ${mesh.name || sceneMeshes.length} has additional skin influences; preview uses the first four while document/export paths retain the extra set`);
    }
    if (vertexCount === 0) continue;

    const localAabb: Aabb = { min: [Infinity, Infinity, Infinity], max: [-Infinity, -Infinity, -Infinity] };
    for (let index = 0; index < positions.length; index += 3) {
      pushAabb(box, positions[index], positions[index + 1], positions[index + 2]);
      pushAabb(localAabb, positions[index], positions[index + 1], positions[index + 2]);
    }
    const attributes: Record<string, RuntimeAccessor> = {
      POSITION: { bytes: positions, componentType: 5126, components: 3, normalized: false, count: vertexCount },
    };
    if (normals) attributes.NORMAL = { bytes: normals, componentType: 5126, components: 3, normalized: false, count: vertexCount };
    if (uvs) attributes.TEXCOORD_0 = { bytes: uvs, componentType: 5126, components: 2, normalized: false, count: vertexCount };
    if (colors) {
      attributes.COLOR_0 = {
        bytes: colors,
        componentType: colorsAreFloat ? 5126 : 5121,
        components: colors.length === vertexCount * 4 ? 4 : 3,
        normalized: !colorsAreFloat,
        count: vertexCount,
      };
    }
    if (joints && weights) {
      attributes.JOINTS_0 = { bytes: joints, componentType: 5123, components: 4, normalized: false, count: vertexCount };
      attributes.WEIGHTS_0 = { bytes: weights, componentType: 5126, components: 4, normalized: false, count: vertexCount };
    }
    const primitive: ViewerPrimitive = { attributes, mode: 4, materialIndex: 0 };
    if (indices) {
      primitive.indices = {
        bytes: indices, componentType: 5125, components: 1, normalized: false, count: indices.length,
      };
    }

    const sourceMaterial = mesh.material === undefined ? undefined : parsed.materials?.[mesh.material];
    const material: FlatMaterial = {
      baseColorFactor: colors ? [1, 1, 1, 1] : [...(sourceMaterial?.diffuse || [1, 1, 1]), sourceMaterial?.alpha ?? 1],
      doubleSided: true,
      alphaMode: 'OPAQUE',
      unlit: false,
    };
    if (sourceMaterial?.baseColorTextureUri) {
      if (uvs) material.baseColorTextureUri = sourceMaterial.baseColorTextureUri;
      else {
        parsed.warnings ||= [];
        parsed.warnings.push(`OBJ texture ${sourceMaterial.baseColorTextureUri} ignored for ${mesh.material}: mesh has no texture coordinates`);
      }
    }
    sceneMeshes.push({
      name: mesh.name || `mesh_${sceneMeshes.length}`,
      primitives: [primitive],
      aabb: localAabb,
      // Retain source sparse targets for FBX export; the FBX semantic
      // adapter creates its render-space expansion separately.
      morphTargets: mesh.morphTargets || [],
      _defaultMaterial: material,
    });
  }
  if (!isFinite(box.min[0])) {
    box.min = [-0.5, -0.5, -0.5];
    box.max = [0.5, 0.5, 0.5];
  }

  const nodes: ViewerNode[] = sceneMeshes.map((mesh, index) => ({
    name: mesh.name,
    trs: restTrs(),
    children: [],
    meshIndex: index,
    skinIndex: -1,
    world: new Float32Array(16),
  }));
  const materials = sceneMeshes.map((mesh) => mesh._defaultMaterial);
  sceneMeshes.forEach((mesh, meshIndex) => mesh.primitives.forEach((primitive) => { primitive.materialIndex = meshIndex; }));
  const renderables: Renderable[] = nodes.map((node, meshIndex) => ({ node, meshIndex, skinIndex: -1 }));
  const textures = await buildObjTextures(materials, resources, parsed.warnings || (parsed.warnings = []), hooks);
  return {
    nodes,
    rootIndices: nodes.map((_, index) => index),
    meshes: sceneMeshes,
    skins: [],
    materials,
    textures,
    animations: [],
    renderables,
    aabb: box,
    warnings: parsed?.warnings || [],
  };
}

/** Facade preserving the public FBX import entry point. */
export async function buildSceneFromFbx(
  parsed: ParsedMeshes,
  resources: ResourceMap = Object.create(null),
  hooks: ImportHooks = {},
) {
  return buildSemanticFbxScene(parsed, resources, hooks, buildSceneFromMeshes);
}

async function buildObjTextures(
  materials: FlatMaterial[],
  resources: ResourceMap,
  warnings: string[],
  hooks: ImportHooks,
) {
  const textures: ViewerTexture[] = [];
  const byUri = new Map<string, number>();
  for (const material of materials) {
    const uri = material.baseColorTextureUri;
    if (!uri) continue;
    let index = byUri.get(uri);
    if (index === undefined) {
      const bytes = resolveResource(uri, resources);
      if (!bytes) {
        warnings.push(`OBJ texture not selected: ${uri}`);
        continue;
      }
      try {
        // BlobPart excludes SharedArrayBuffer-backed views; these never are.
        const bitmap = await createImageBitmap(new Blob([bytes as BlobPart], { type: mimeFromUri(uri) || 'application/octet-stream' }));
        if (!bitmap) throw new Error('browser could not decode the image');
        index = textures.length;
        textures.push({
          name: basename(uri), image: bitmap, flipY: true,
          wrapS: WebGL2RenderingContext.REPEAT,
          wrapT: WebGL2RenderingContext.REPEAT,
          minFilter: WebGL2RenderingContext.LINEAR_MIPMAP_LINEAR,
          magFilter: WebGL2RenderingContext.LINEAR,
        });
        byUri.set(uri, index);
      } catch (error) {
        const message = `Failed to decode OBJ texture ${uri}: ${(error as Error).message}`;
        warnings.push(message);
        hooks.onLog?.(message, 'warning');
        continue;
      }
    }
    material.baseColorTexture = index;
    delete material.baseColorTextureUri;
  }
  return textures;
}

function restTrs(): Trs {
  return { translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1] };
}
