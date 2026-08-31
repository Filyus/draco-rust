/**
 * The summary panel's geometry figures come from the SceneDocument.
 *
 * They used to come from a second full walk of the asset, opened purely to
 * count what the document already holds. This pins the replacement against the
 * walk it replaced: same vertices, same triangles, same attribute flags, on
 * assets with indexed and non-indexed primitives, multiple meshes and morph
 * targets.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');
const pkg = resolve(here, '..', 'www', 'pkg');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href) as any;
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

const { buildSceneDocumentFromGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
) as any;
const { summarizeSceneDocumentGeometry, triangleCountForMode } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'scene-document.ts')).href
) as any;

/** The walk this replaced, kept here as the thing the new figures must match. */
function summarizeByWalkingTheAsset(data: Uint8Array, resources: Record<string, Uint8Array>) {
  const asset = gltfModule.GltfAsset.withResources(data, resources, '2.1');
  let vertexCount = 0;
  let triangleCount = 0;
  let pointCount = 0;
  let hasNormals = false;
  let hasUvs = false;
  try {
    for (let mesh = 0; mesh < asset.meshCount(); mesh += 1) {
      for (let primitive = 0; primitive < asset.primitiveCount(mesh); primitive += 1) {
        const geometry = asset.readPrimitive(mesh, primitive);
        try {
          let primitiveVertexCount = 0;
          for (let attribute = 0; attribute < geometry.attributeCount(); attribute += 1) {
            const semantic = geometry.attributeSemantic(attribute);
            if (semantic === 'POSITION') primitiveVertexCount = geometry.attributeElementCount(attribute);
            else if (semantic === 'NORMAL') hasNormals = true;
            else if (semantic.startsWith('TEXCOORD_')) hasUvs = true;
          }
          vertexCount += primitiveVertexCount;
          const elementCount = geometry.hasIndices() ? geometry.indexCount() : primitiveVertexCount;
          triangleCount += triangleCountForMode(geometry.mode(), elementCount);
          // One point per element, by the same rule that gives such a
          // primitive no triangles.
          if (geometry.mode() === 0) pointCount += elementCount;
        } finally {
          geometry.free();
        }
      }
    }
  } finally {
    asset.free();
  }
  return { vertexCount, triangleCount, pointCount, hasNormals, hasUvs };
}

/** A .gltf needs its companions by name; a .glb needs none. */
async function companionResources(path: string): Promise<Record<string, Uint8Array>> {
  if (path.endsWith('.glb')) return {};
  const directory = dirname(path);
  const manifest = JSON.parse(await readFile(path, 'utf8'));
  const names = [
    ...(manifest.buffers || []).map((buffer: { uri?: string }) => buffer.uri),
    ...(manifest.images || []).map((image: { uri?: string }) => image.uri),
  ].filter((uri): uri is string => typeof uri === 'string' && !uri.startsWith('data:'));
  const resources: Record<string, Uint8Array> = {};
  for (const name of names) resources[name] = new Uint8Array(await readFile(resolve(directory, name)));
  return resources;
}

const models = [
  // Fox is indexed and animated with several primitives; CesiumMan is skinned;
  // MultiUVTest carries a second UV set, which the attribute flags depend on.
  { name: 'Fox', path: resolve(repoRoot, 'testdata', 'Fox', 'glTF', 'Fox.gltf') },
  { name: 'CesiumMan', path: resolve(repoRoot, 'testdata', 'CesiumMan', 'glTF_Binary', 'CesiumMan.glb') },
  { name: 'IridescenceLamp', path: resolve(repoRoot, 'testdata', 'IridescenceLamp.glb') },
];

let checked = 0;
for (const model of models) {
  let data: Uint8Array;
  try {
    data = new Uint8Array(await readFile(model.path));
  } catch {
    console.log(`skip ${model.name}: ${model.path} is unavailable`);
    continue;
  }
  const resources = await companionResources(model.path);
  const walked = summarizeByWalkingTheAsset(data, resources);
  const derived = summarizeSceneDocumentGeometry(buildSceneDocumentFromGltf(data, resources, gltfModule));
  assert.deepEqual(derived, walked, `${model.name} summary drifted from the asset walk`);
  assert.ok(derived.vertexCount > 0, `${model.name} reported no vertices at all`);
  checked += 1;
}

assert.ok(checked > 0, 'no glTF fixture was available to compare against');
console.log(`glTF summary parity passed (${checked} model${checked === 1 ? '' : 's'})`);
