/**
 * Primitives that share a source accessor share it in the document too.
 *
 * Splitting one mesh into primitives by material is how most authored assets
 * are built, and those primitives keep referencing the same POSITION, NORMAL
 * and index accessors. The importer reads geometry per primitive, already
 * materialized, so the bytes alone cannot reveal the sharing — hence the source
 * accessor index the reader now reports. Without it, DuplicateMeshes turned
 * five accessors into thirty-five copies, in the document, in every GLB written
 * from it and in every FBX.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import type { buildSceneDocumentFromGltf as BuildSceneDocumentFromGltf } from '../src/gltf-scene-document.ts';
import type { lowerSceneDocumentToGltf as LowerSceneDocumentToGltf } from '../src/scene-document-gltf.ts';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');
const pkg = resolve(here, '..', 'www', 'pkg');
const model = resolve(repoRoot, 'testdata', 'DuplicateMeshes', 'duplicate_meshes.gltf');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

const { buildSceneDocumentFromGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
) as { buildSceneDocumentFromGltf: typeof BuildSceneDocumentFromGltf };
const { lowerSceneDocumentToGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'scene-document-gltf.ts')).href
) as { lowerSceneDocumentToGltf: typeof LowerSceneDocumentToGltf };

const manifest = JSON.parse(await readFile(model, 'utf8'));
const resources: Record<string, Uint8Array> = {};
for (const entry of [...(manifest.buffers || []), ...(manifest.images || [])]) {
  if (typeof entry.uri === 'string' && !entry.uri.startsWith('data:')) {
    resources[entry.uri] = new Uint8Array(await readFile(resolve(dirname(model), entry.uri)));
  }
}

const references = manifest.meshes.flatMap((mesh: any) => mesh.primitives.flatMap((primitive: any) => [
  ...Object.values(primitive.attributes),
  ...(primitive.indices === undefined ? [] : [primitive.indices]),
]));
assert.equal(references.length, 35, 'the fixture must exercise sharing: 35 references');
assert.equal(new Set(references).size, 5, 'over 5 distinct accessors');

const document = buildSceneDocumentFromGltf(new Uint8Array(await readFile(model)), resources, gltfModule);
assert.equal(
  document.accessors.length,
  5,
  `shared accessors were copied per primitive: ${document.accessors.length} accessors for 5 sources`,
);

// Sharing has to be the same sharing, not merely the same count.
const [first, second] = document.meshes[0].primitives;
assert.equal(first.attributes.POSITION, second.attributes.POSITION);
assert.equal(first.indices, second.indices);
assert.notEqual(first.attributes.POSITION, first.indices, 'an attribute and an index stream must stay separate accessors');

// An accessor read as both an attribute and an index stream must not collapse:
// the writer assigns one bufferView target per accessor, first writer winning,
// so sharing one would emit a buffer view with the wrong target.
const lowered = lowerSceneDocumentToGltf(document);
const output = JSON.parse(new TextDecoder().decode(lowered.json));
const targets = new Map(output.accessors.map((accessor: any) => [
  accessor.bufferView,
  output.bufferViews[accessor.bufferView].target,
]));
for (const mesh of output.meshes) {
  for (const primitive of mesh.primitives) {
    for (const accessor of Object.values(primitive.attributes) as number[]) {
      assert.equal(targets.get(output.accessors[accessor].bufferView), 34962, 'attributes must target ARRAY_BUFFER');
    }
    assert.equal(
      targets.get(output.accessors[primitive.indices].bufferView),
      34963,
      'indices must target ELEMENT_ARRAY_BUFFER',
    );
  }
}

const { validateBytes } = await import('gltf-validator');
const resourceMap = lowered.resources as Record<string, Uint8Array>;
const report = await validateBytes(lowered.json, {
  externalResourceFunction: async (uri: string) => resourceMap[uri] ?? new Uint8Array(),
});
assert.deepEqual(
  report.issues.messages.filter((issue) => issue.severity === 0),
  [],
  'the deduplicated export must still validate as glTF 2.0',
);

console.log(`glTF accessor dedup passed (${references.length} references, ${document.accessors.length} accessors, ${lowered.resources['scene.bin'].length} binary bytes)`);
