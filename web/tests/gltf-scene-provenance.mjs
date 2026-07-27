/**
 * The extension claims a glTF makes about itself travel beside the document,
 * not inside it.
 *
 * SceneDocument records what a scene *is*; `extensionsUsed` is what the file
 * said about itself, which is a different kind of fact and belongs to the
 * source rather than to the portable form. But a consumer that has to report
 * what it could not act on needs those claims, and the whole point of building
 * the document once is that nobody re-opens the asset to read two arrays.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

const { buildSceneDocumentFromGltf, buildSceneDocumentWithGltfProvenance } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
);
const { GLTF_SCENE_PROVENANCE_VERSION } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-provenance.ts')).href
);

const positions = new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]);
const binary = new Uint8Array(positions.buffer);

// Deliberately mixed: one extension both readers interpret, one the preview
// ignores, and a required list that is not the same list as the used one.
const manifest = {
  asset: { version: '2.0' },
  extensionsUsed: ['KHR_materials_unlit', 'KHR_materials_sheen', 'KHR_texture_transform'],
  extensionsRequired: ['KHR_materials_sheen'],
  scene: 0,
  scenes: [{ nodes: [0] }],
  nodes: [{ mesh: 0 }],
  meshes: [{ primitives: [{ attributes: { POSITION: 0 }, material: 0 }] }],
  materials: [{ extensions: { KHR_materials_unlit: {} } }],
  accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] }],
  bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: binary.length }],
  buffers: [{
    byteLength: binary.length,
    uri: `data:application/octet-stream;base64,${Buffer.from(binary).toString('base64')}`,
  }],
};
const source = new Uint8Array(new TextEncoder().encode(JSON.stringify(manifest)));

const { document, provenance } = buildSceneDocumentWithGltfProvenance(source, {}, gltfModule);

assert.equal(provenance.version, GLTF_SCENE_PROVENANCE_VERSION);
assert.equal(provenance.format, 'gltf');
assert.deepEqual(
  provenance.extensionsUsed,
  ['KHR_materials_unlit', 'KHR_materials_sheen', 'KHR_texture_transform'],
  'extensionsUsed must arrive verbatim and in document order',
);
assert.deepEqual(
  provenance.extensionsRequired,
  ['KHR_materials_sheen'],
  'extensionsRequired is its own list and must not be conflated with extensionsUsed',
);

// The pairing must not change what the document is: the plain builder is now
// the paired one with the provenance dropped, and anything else would mean the
// export path and the preview path disagree about the same file again.
assert.deepEqual(
  buildSceneDocumentFromGltf(source, {}, gltfModule),
  document,
  'adding provenance must leave the document untouched',
);

// The claims stay out of the document itself; what the portable form lost is
// already stated there in its own words.
assert.equal(
  JSON.stringify(document).includes('extensionsUsed'),
  false,
  'the document must not start carrying the file claims it deliberately omits',
);
assert.ok(
  document.warnings.some((warning) => warning.includes('KHR_materials_sheen')),
  'the document still reports what the portable subset could not take',
);

// A file that claims nothing yields empty lists rather than absent ones, so a
// consumer never has to distinguish "claimed nothing" from "was not asked".
const bare = { ...manifest };
delete bare.extensionsUsed;
delete bare.extensionsRequired;
const plain = buildSceneDocumentWithGltfProvenance(
  new Uint8Array(new TextEncoder().encode(JSON.stringify(bare))), {}, gltfModule,
);
assert.deepEqual(plain.provenance.extensionsUsed, []);
assert.deepEqual(plain.provenance.extensionsRequired, []);

console.log('gltf-scene-provenance: OK');
