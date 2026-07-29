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
