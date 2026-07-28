/**
 * Instance transforms, including the quantized rotation the extension allows.
 *
 * EXT_mesh_gpu_instancing lets ROTATION arrive as normalized BYTE or SHORT
 * rather than FLOAT. No public asset uses either — the one Khronos instancing
 * model is float, and so is every other instancing file anyone ships — so the
 * reader that decoded those bytes as float32 produced garbage matrices and
 * nothing in the corpus could say so.
 *
 * `InstancedQuadsQuantized.gltf` is `InstancedQuads.gltf` with the rotations
 * quantized and nothing else changed, so the two must compose to the same
 * matrices to within the quantization step. That is the whole assertion: it
 * needs no expected values of its own, because the float file is the expected
 * value.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');
const pkg = resolve(here, '..', 'www', 'pkg');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

const { buildSceneDocumentFromGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
);
const { buildViewerSceneFromDocument } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'scene-document-viewer.ts')).href
);

async function instancesOf(name) {
  const path = resolve(repoRoot, 'testdata', name);
  const data = new Uint8Array(await readFile(path));
  const scene = buildViewerSceneFromDocument(buildSceneDocumentFromGltf(data, {}, gltfModule));
  const node = scene.nodes.find((entry) => entry.instancing);
  assert.ok(node, `${name} has a node carrying EXT_mesh_gpu_instancing`);
  return node.instancing;
}

const float = await instancesOf('InstancedQuads.gltf');
const quantized = await instancesOf('InstancedQuadsQuantized.gltf');

assert.equal(quantized.count, 4, 'four copies, as the fixture states');
assert.equal(quantized.count, float.count);
assert.equal(quantized.matrices.length, float.matrices.length);

// A normalized SHORT holds a unit interval in 1/32767 steps, and a matrix
// element is a product of two quaternion components, so the error compounds
// but stays in that neighbourhood. A reader that decoded the bytes as float32
// is not close to this — it is off by whole units and by half the matrices.
const worst = Math.max(...Array.from(
  quantized.matrices, (value, index) => Math.abs(value - float.matrices[index]),
));
assert.ok(
  worst < 1e-4,
  `quantized instance matrices differ from the float ones by ${worst}, which is not a rounding step`,
);

// And they are real transforms rather than an accidental match on zeroes: the
// four copies stand apart along x, and each is turned further than the last.
const translationX = Array.from({ length: 4 }, (_, index) => quantized.matrices[index * 16 + 12]);
assert.deepEqual(translationX, [-2.25, -0.75, 0.75, 2.25], 'the copies keep their spacing');
const turn = Array.from({ length: 4 }, (_, index) => Math.atan2(
  quantized.matrices[index * 16 + 1], quantized.matrices[index * 16 + 0],
));
for (let index = 1; index < turn.length; index += 1) {
  assert.ok(turn[index] > turn[index - 1], 'each copy is turned further than the one before it');
}

console.log('viewer-instancing: OK');
