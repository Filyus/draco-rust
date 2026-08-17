// The app decides whether a glTF that failed to build a SceneDocument is a
// warning or an error by matching the message, so the prefix is a contract
// between draco-gltf's error type and app.ts. A corrupted Draco payload must
// keep saying "Draco decode error:" -- if it stops, the file quietly reports a
// successful parse whose meshes never decoded.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { loadWasm, repoRoot } from './fbx-test-utils.ts';
import { buildSceneDocumentWithGltfProvenance } from '../src/gltf-scene-document.ts';

const base = resolve(repoRoot, 'testdata', 'BoxMetaDraco', 'glTF');
const json = new Uint8Array(await readFile(resolve(base, 'BoxMetaDraco.gltf')));
const bufferName = JSON.parse(Buffer.from(json).toString()).buffers[0].uri;
const buffer = new Uint8Array(await readFile(resolve(base, bufferName)));
const gltf = await loadWasm('gltf');

const intact = buildSceneDocumentWithGltfProvenance(json, { [bufferName]: buffer }, gltf);
assert.ok(intact.document.nodes.length > 0, 'the intact Draco asset must still build a document');

const corrupted = buffer.slice();
for (let index = 40; index < 140 && index < corrupted.length; index++) corrupted[index] ^= 0xff;
assert.throws(
    () => buildSceneDocumentWithGltfProvenance(json, { [bufferName]: corrupted }, gltf),
    (error: unknown) => (error instanceof Error ? error.message : String(error)).includes('Draco decode error:'),
    'a corrupted Draco payload must surface draco-core\'s own message behind the "Draco decode error:" prefix',
);

console.log('PASS glTF Draco decode failure keeps its prefix');
