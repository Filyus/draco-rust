/**
 * Textures that want the same filtering share one sampler.
 *
 * glTF attaches no identity to a sampler — it is a bag of four enums — so a
 * writer that emits one per texture produces N identical records. Nothing
 * misreads such a file, but every exported asset carries the copies and no
 * two exports of the same scene diff cleanly.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { stripeNormalMapPng } from './smoke-fixtures.ts';
import type { createSceneDocument as CreateSceneDocument } from '../src/scene-document.ts';
import type { lowerSceneDocumentToGltf as LowerSceneDocumentToGltf } from '../src/scene-document-gltf.ts';
import type { buildSceneDocumentFromGltf as BuildSceneDocumentFromGltf } from '../src/gltf-scene-document.ts';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');
const pkg = resolve(here, '..', 'www', 'pkg');

const { createSceneDocument } = await import(pathToFileURL(resolve(here, '..', 'src', 'scene-document.ts')).href) as {
  createSceneDocument: typeof CreateSceneDocument;
};
const { lowerSceneDocumentToGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'scene-document-gltf.ts')).href
) as { lowerSceneDocumentToGltf: typeof LowerSceneDocumentToGltf };

const png = new Uint8Array(stripeNormalMapPng());
const positions = new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]);
const uvs = new Float32Array([0, 0, 1, 0, 0, 1]);
const bytes = (view: Float32Array): Uint8Array => new Uint8Array(view.buffer.slice(view.byteOffset, view.byteOffset + view.byteLength));
const repeat = { wrapS: 10497, wrapT: 10497, minFilter: 9987, magFilter: 9729 };

const document = createSceneDocument({
  resources: [{ name: 'stripes.png', mimeType: 'image/png', bytes: png }],
  textures: [
    { name: 'a', resource: 0, sampler: { ...repeat } },
    { name: 'b', resource: 0, sampler: { ...repeat } },
    { name: 'c', resource: 0, sampler: { ...repeat } },
    // One genuinely different setting must still get its own record.
    { name: 'clamped', resource: 0, sampler: { ...repeat, wrapS: 33071 } },
  ],
  materials: [{
    baseColorTexture: { texture: 0 },
    metallicRoughnessTexture: { texture: 1 },
    normalTexture: { texture: 2 },
    emissiveTexture: { texture: 3 },
  }],
  accessors: [
    { bytes: bytes(positions), componentType: 5126, components: 3, count: 3 },
    { bytes: bytes(uvs), componentType: 5126, components: 2, count: 3 },
  ],
  meshes: [{ primitives: [{ attributes: { POSITION: 0, TEXCOORD_0: 1 }, material: 0 }] }],
  nodes: [{ name: 'quad', mesh: 0 }],
  rootNodes: [0],
});

const manifest = JSON.parse(new TextDecoder().decode(lowerSceneDocumentToGltf(document).json));
assert.equal(manifest.samplers.length, 2, 'four textures over two distinct settings need two samplers');
assert.deepEqual(manifest.textures.map((texture: { sampler: number }) => texture.sampler), [0, 0, 0, 1]);
// One image behind four textures was already shared; assert it stays that way.
assert.equal(manifest.images.length, 1);

// A real asset: DuplicateMeshes declares no samplers at all, so every texture
// resolves to the same glTF defaults.
const model = resolve(repoRoot, 'testdata', 'DuplicateMeshes', 'duplicate_meshes.gltf');
const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });
const { buildSceneDocumentFromGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
) as { buildSceneDocumentFromGltf: typeof BuildSceneDocumentFromGltf };
const source = JSON.parse(await readFile(model, 'utf8'));
const resources: Record<string, Uint8Array> = {};
for (const uri of [...(source.buffers || []), ...(source.images || [])].map((entry: { uri?: string }) => entry.uri)) {
  if (typeof uri === 'string' && !uri.startsWith('data:')) {
    resources[uri] = new Uint8Array(await readFile(resolve(dirname(model), uri)));
  }
}
const imported = buildSceneDocumentFromGltf(new Uint8Array(await readFile(model)), resources, gltfModule);
const exported = JSON.parse(new TextDecoder().decode(lowerSceneDocumentToGltf(imported).json));
assert.equal(imported.textures.length, 4);
assert.equal(exported.samplers.length, 1, 'four textures on identical default settings need one sampler');

console.log('SceneDocument glTF sampler sharing passed');
