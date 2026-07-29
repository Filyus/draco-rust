/**
 * Flat triangle meshes → SceneDocument.
 *
 * The formats that hand over a bare mesh list — OBJ, PLY, STL and the
 * standalone Draco container — had no route to glTF at all: the document
 * exports read a source document, and a mesh list is not one. This builds the
 * portable document those exports already know how to write, so the same
 * SceneDocument → GLB path that serves FBX serves them too.
 *
 * Source-neutral by construction, like every other importer here: it takes
 * arrays and returns structured data, and knows nothing about which reader
 * produced them.
 */

import { componentByteSize, normalizeComponent, readComponent } from './component-values.ts';
import { composeTrs, invertMat4, multiplyMat4 } from './mat4.ts';
import { createSceneDocument } from './scene-document.ts';
import type {
  AttributeMap, ComponentType, SceneDocument, SceneMaterial, ScenePrimitive,
} from './scene-document.ts';
import type { LoadedMesh, LoadedObjMaterial, OpaqueAttribute } from './mesh-loader.ts';
import {
  appendAccessor, basename, bytesFromF32, bytesFromU16, bytesFromU32, mimeFromUri, resolveResource,
  sniffMime,
} from './scene-resources.ts';
import type { ResourceMap } from './scene-resources.ts';

export interface MeshDocumentOptions {
  /** OBJ material library, keyed by the name a mesh's `material` names. */
  materials?: Record<string, LoadedObjMaterial>;
  /** Companion files, for embedding the textures a material points at. */
  resources?: ResourceMap;
}

/** Build a portable document from meshes that arrived without a scene. */
export function buildSceneDocumentFromMeshes(
  meshes: LoadedMesh[],
  options: MeshDocumentOptions = {},
): SceneDocument {
  const document = createSceneDocument();
  const materialIndices = new Map<string, number>();

  meshes.forEach((mesh, index) => {
    const positions = mesh.positions || [];
    const vertexCount = Math.floor(positions.length / 3);
    if (vertexCount === 0) return;

    const attributes: AttributeMap = {
      POSITION: appendFloatAccessor(document, positions, 3),
    };
    if (mesh.normals?.length === vertexCount * 3) {
      attributes.NORMAL = appendFloatAccessor(document, mesh.normals, 3);
    }
    if (mesh.uvs?.length === vertexCount * 2) {
      attributes.TEXCOORD_0 = appendFloatAccessor(document, mesh.uvs, 2);
    }
    if (mesh.colors?.length === vertexCount * 4) {
      // The flat readers hand colours over as RGBA bytes, which glTF spells as
      // a normalized unsigned byte vector rather than as floats.
      attributes.COLOR_0 = appendAccessor(document, {
        bytes: Uint8Array.from(mesh.colors, (value) => Math.min(Math.max(value, 0), 255)),
        componentType: 5121,
        components: 4,
        count: vertexCount,
        normalized: true,
      });
    }
    appendOpaqueAttributes(document, attributes, mesh.extras || [], vertexCount);

    const primitive: ScenePrimitive = { attributes };
    const indices = mesh.indices;
    if (indices?.length) primitive.indices = appendIndexAccessor(document, indices);
    const material = materialIndexFor(document, materialIndices, mesh, options);
    if (material !== null) primitive.material = material;

    const name = mesh.name || `mesh_${index}`;
    document.meshes.push({ name, primitives: [primitive] });
    document.rootNodes.push(document.nodes.length);
    document.nodes.push({ name, mesh: document.meshes.length - 1 });
  });

  return document;
}

/**
 * The document's triangles, placed where the scene puts them.
 *
 * The inverse direction, and the one every flat writer needs: OBJ, PLY, STL and
 * `.drc` have nowhere to put a hierarchy, so a node's transform has to be baked
 * into its coordinates or the scene collapses onto the origin — two objects a
 * metre apart come out inside one another. An FBX exported to any of those did
 * exactly that, because the flat meshes its reader hands over are in each
 * node's own space and the hierarchy that places them was left behind.
 *
 * A mesh several nodes instance is returned once per node, since that is what
 * the scene contains.
 */
export function flattenSceneDocument(document: SceneDocument): LoadedMesh[] {
  const worlds = documentWorldMatrices(document);
  const meshes: LoadedMesh[] = [];
  document.nodes.forEach((node, index) => {
    const world = worlds[index];
    if (node.mesh === undefined || !world) return;
    const source = document.meshes[node.mesh];
    if (!source) return;
    source.primitives.forEach((primitive, primitiveIndex) => {
      // The flat writers take triangles and nothing else; a line or point set
      // has no representation to lose data into.
      if (primitive.mode !== undefined && primitive.mode !== 4) return;
      const positions = attributeNumbers(document, primitive.attributes.POSITION);
      if (!positions?.length) return;
      const normals = attributeNumbers(document, primitive.attributes.NORMAL);
      const name = [node.name || source.name, source.primitives.length > 1 ? primitiveIndex : null]
        .filter((part) => part !== null && part !== undefined)
        .join('_') || `mesh_${meshes.length}`;
      meshes.push({
        name,
        positions: transformPositions(positions, world),
        indices: primitive.indices === undefined
          ? null
          : attributeNumbers(document, primitive.indices),
        normals: normals?.length ? transformNormals(normals, world) : null,
        uvs: attributeNumbers(document, primitive.attributes.TEXCOORD_0),
        colors: colorBytes(document, primitive.attributes.COLOR_0),
      });
    });
  });
  return meshes;
}

/** Every node's world matrix, column-major, or null when unreachable. */
function documentWorldMatrices(document: SceneDocument): (number[] | null)[] {
  const worlds: (number[] | null)[] = document.nodes.map(() => null);
  const visit = (index: number, parent: number[] | null) => {
    const node = document.nodes[index];
    if (!node || worlds[index]) return;
    const local = node.matrix?.length === 16
      ? [...node.matrix]
      : composeTrs({
        translation: node.translation ?? [0, 0, 0],
        rotation: node.rotation ?? [0, 0, 0, 1],
        scale: node.scale ?? [1, 1, 1],
      });
    worlds[index] = parent ? multiplyMat4(parent, local) : local;
    for (const child of node.children || []) visit(child, worlds[index]);
  };
  for (const root of document.rootNodes) visit(root, null);
  return worlds;
}

/** One accessor as plain numbers, whatever component type it was stored in. */
function attributeNumbers(document: SceneDocument, index: number | undefined): number[] | null {
  if (index === undefined) return null;
  const accessor = document.accessors[index];
  if (!accessor) return null;
  const width = componentByteSize(accessor.componentType);
  const view = new DataView(
    accessor.bytes.buffer,
    accessor.bytes.byteOffset,
    accessor.bytes.byteLength,
  );
  const total = accessor.count * accessor.components;
  const values: number[] = new Array(total);
  for (let element = 0; element < total; element += 1) {
    const value = readComponent(view, element * width, accessor.componentType);
    values[element] = accessor.normalized
      ? normalizeComponent(value, accessor.componentType)
      : value;
  }
  return values;
}

/** `COLOR_0` as the RGBA bytes the flat writers take, or nothing. */
function colorBytes(document: SceneDocument, index: number | undefined): number[] | null {
  const accessor = index === undefined ? null : document.accessors[index];
  const values = attributeNumbers(document, index);
  if (!accessor || !values) return null;
  const components = accessor.components;
  const scale = accessor.normalized || accessor.componentType === 5126 ? 255 : 1;
  const out: number[] = [];
  for (let element = 0; element + components <= values.length; element += components) {
    const byte = (value: number) => Math.round(Math.min(Math.max(value * scale, 0), 255));
    out.push(
      byte(values[element]),
      byte(values[element + 1]),
      byte(values[element + 2]),
      components >= 4 ? byte(values[element + 3]) : 255,
    );
  }
  return out;
}

function transformPositions(values: number[], matrix: number[]): number[] {
  const out: number[] = new Array(values.length);
  for (let index = 0; index + 2 < values.length; index += 3) {
    const [x, y, z] = [values[index], values[index + 1], values[index + 2]];
    out[index] = matrix[0] * x + matrix[4] * y + matrix[8] * z + matrix[12];
    out[index + 1] = matrix[1] * x + matrix[5] * y + matrix[9] * z + matrix[13];
    out[index + 2] = matrix[2] * x + matrix[6] * y + matrix[10] * z + matrix[14];
  }
  return out;
}

/** Normals go through the inverse transpose, so non-uniform scale is honoured. */
function transformNormals(values: number[], matrix: number[]): number[] {
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
 * The attributes a reader carried without interpreting, given glTF names.
 *
 * A second texture-coordinate or colour set is exactly what `TEXCOORD_1` and
 * `COLOR_1` are for, so those survive the crossing. A generic attribute does
 * not: glTF's only home for one is an application-specific `_NAME`, and the
 * name would be this converter's invention rather than anything the payload
 * stated. It is reported instead — the document carries the warning, and the
 * export panel shows it.
 */
function appendOpaqueAttributes(
  document: SceneDocument,
  attributes: AttributeMap,
  extras: OpaqueAttribute[],
  vertexCount: number,
) {
  const nextIndex = { TEXCOORD: 1, COLOR: 1 };
  for (const extra of extras) {
    const semantic = extra.type === 'TEX_COORD' ? 'TEXCOORD'
      : extra.type === 'COLOR' ? 'COLOR' : null;
    if (!semantic || extra.values.length < vertexCount * extra.components) {
      document.warnings.push(
        `glTF has no place for a ${extra.type} attribute (${extra.components} components, `
        + `id ${extra.uniqueId}); it was not written`,
      );
      continue;
    }
    attributes[`${semantic}_${nextIndex[semantic]}`] = appendOpaqueAccessor(
      document, extra, vertexCount,
    );
    nextIndex[semantic] += 1;
  }
}

/**
 * One carried attribute as an accessor, in the component type it arrived in.
 *
 * glTF accepts floats for these semantics, and unsigned bytes or shorts only
 * when they are normalized. Anything else is written as floats — the values
 * are already exact as doubles, so widening loses nothing and keeps the
 * document valid, which a normalized flag invented here would not.
 */
function appendOpaqueAccessor(
  document: SceneDocument,
  extra: OpaqueAttribute,
  vertexCount: number,
): number {
  const count = vertexCount * extra.components;
  const values = Array.from({ length: count }, (_, index) => extra.values[index]);
  if (extra.normalized && (extra.dataType === 'uint8' || extra.dataType === 'uint16')) {
    const componentType: ComponentType = extra.dataType === 'uint8' ? 5121 : 5123;
    return appendAccessor(document, {
      bytes: extra.dataType === 'uint8'
        ? Uint8Array.from(values, (value) => Math.min(Math.max(value, 0), 255))
        : bytesFromU16(values.map((value) => Math.min(Math.max(value, 0), 65535))),
      componentType,
      components: extra.components,
      count: vertexCount,
      normalized: true,
    });
  }
  return appendFloatAccessor(document, values, extra.components);
}

/** The material a mesh names, created once and shared by every mesh naming it. */
function materialIndexFor(
  document: SceneDocument,
  indices: Map<string, number>,
  mesh: LoadedMesh,
  options: MeshDocumentOptions,
): number | null {
  const name = mesh.material;
  const source = name ? options.materials?.[name] : undefined;
  if (!name || !source) return null;
  const existing = indices.get(name);
  if (existing !== undefined) return existing;

  const alpha = typeof source.alpha === 'number' ? source.alpha : 1;
  const diffuse = source.diffuse?.length === 3 ? source.diffuse : [1, 1, 1];
  const material: SceneMaterial = {
    name,
    baseColorFactor: [...diffuse, alpha],
    // OBJ states no metallic-roughness, and its diffuse colour is what a
    // dielectric surface shows. Leaving the glTF defaults would render an OBJ
    // as polished metal.
    metallicFactor: 0,
    roughnessFactor: 1,
    ...(alpha < 1 ? { alphaMode: 'BLEND' as const } : {}),
  };
  const texture = embedTexture(document, source.baseColorTextureUri, options.resources);
  if (texture !== null) material.baseColorTexture = { texture, texCoord: 0 };

  const index = document.materials.length;
  document.materials.push(material);
  indices.set(name, index);
  return index;
}

/** Put a companion image into the document, so the GLB is self-contained. */
function embedTexture(
  document: SceneDocument,
  uri: string | null | undefined,
  resources: ResourceMap | undefined,
): number | null {
  if (!uri || !resources) return null;
  const bytes = resolveResource(uri, resources);
  if (!bytes) {
    document.warnings.push(`Texture not in the selection, so it was not written: ${uri}`);
    return null;
  }
  const mimeType = sniffMime(bytes) || mimeFromUri(uri);
  if (!mimeType) {
    document.warnings.push(`Texture format not recognised, so it was not written: ${uri}`);
    return null;
  }
  const resource = document.resources.length;
  document.resources.push({ mimeType, bytes, name: basename(uri) });
  const texture = document.textures.length;
  document.textures.push({ resource, name: basename(uri) });
  return texture;
}

function appendFloatAccessor(
  document: SceneDocument,
  values: ArrayLike<number>,
  components: number,
): number {
  return appendAccessor(document, {
    bytes: bytesFromF32(values),
    componentType: 5126,
    components,
    count: Math.floor(values.length / components),
    normalized: false,
  });
}

/**
 * Indices, in the narrowest width that holds them.
 *
 * Halving the index stream on a mesh under 65536 vertices is most of what
 * separates a written GLB from a needlessly large one, and every consumer
 * reads both.
 */
function appendIndexAccessor(document: SceneDocument, indices: ArrayLike<number>): number {
  let highest = 0;
  for (let index = 0; index < indices.length; index += 1) {
    if (indices[index] > highest) highest = indices[index];
  }
  const narrow = highest < 65536;
  return appendAccessor(document, {
    bytes: narrow ? bytesFromU16(indices) : bytesFromU32(indices),
    componentType: narrow ? 5123 : 5125,
    components: 1,
    count: indices.length,
    normalized: false,
  });
}
