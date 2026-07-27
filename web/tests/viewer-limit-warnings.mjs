/**
 * Both importers reach the same renderer, so both have to report its limits —
 * and report them in the same words.
 *
 * The glTF loader has always said what the preview cannot show: morph tangents
 * it derives rather than reads, and morph poses wider than one frame blends.
 * The SceneDocument adapter said none of it, which was invisible while nothing
 * previewed through the document. It is about to, so the sentences now come
 * from one place, and this asserts they arrive identical rather than merely
 * similar — a user who loads one asset through both paths must not be told two
 * different things about it.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

const { buildSceneFromGltf } = await import(pathToFileURL(resolve(here, '..', 'src', 'gltf-loader.ts')).href);
const { buildSceneDocumentFromGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
);
const { buildViewerSceneFromDocument } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'scene-document-viewer.ts')).href
);
const { MAX_ACTIVE_MORPH_TARGETS } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'viewer-scene.ts')).href
);

/** One more target than a single frame can blend, so every limit trips. */
const TARGETS = MAX_ACTIVE_MORPH_TARGETS + 1;

const positions = new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]);
const tangents = new Float32Array([1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1]);
const indices = new Uint16Array([0, 1, 2]);
// Every target nudges one vertex, so none of them is a no-op the reader could
// legitimately drop before the limit is reached.
const targetDeltas = new Float32Array(TARGETS * 9);
for (let target = 0; target < TARGETS; target += 1) targetDeltas[target * 9] = 0.01 * (target + 1);
const times = new Float32Array([0, 1]);
// One keyframe holding every target at once: that is the pose the preview
// cannot blend, and it has to be reported per clip as well as per mesh.
const weightKeys = new Float32Array(2 * TARGETS).fill(1);

const views = [positions, tangents, indices, targetDeltas, times, weightKeys];
const offsets = [];
let length = 0;
for (const view of views) {
  offsets.push(length);
  length += Math.ceil(view.byteLength / 4) * 4;
}
const binary = new Uint8Array(length);
views.forEach((view, index) => {
  binary.set(new Uint8Array(view.buffer, view.byteOffset, view.byteLength), offsets[index]);
});

const accessors = [
  { bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] },
  { bufferView: 1, componentType: 5126, count: 3, type: 'VEC4' },
  { bufferView: 2, componentType: 5123, count: 3, type: 'SCALAR' },
  ...Array.from({ length: TARGETS }, (_, target) => ({
    bufferView: 3, byteOffset: target * 36, componentType: 5126, count: 3, type: 'VEC3',
  })),
  { bufferView: 4, componentType: 5126, count: 2, type: 'SCALAR' },
  { bufferView: 5, componentType: 5126, count: 2 * TARGETS, type: 'SCALAR' },
];
const timesAccessor = 3 + TARGETS;

const manifest = {
  asset: { version: '2.0' },
  scene: 0,
  scenes: [{ nodes: [0] }],
  // Every weight non-zero at rest, which is the per-mesh limit the preview
  // reports separately from the per-keyframe one.
  nodes: [{ mesh: 0, weights: new Array(TARGETS).fill(1) }],
  meshes: [{
    name: 'Wide',
    primitives: [{
      attributes: { POSITION: 0, TANGENT: 1 },
      indices: 2,
      // A target may only refine a semantic the primitive declares, so TANGENT
      // is part of a valid fixture rather than decoration.
      targets: Array.from({ length: TARGETS }, (_, target) => ({ POSITION: 3 + target, TANGENT: 1 })),
    }],
  }],
  animations: [{
    name: 'Everything',
    samplers: [{ input: timesAccessor, output: timesAccessor + 1, interpolation: 'LINEAR' }],
    channels: [{ sampler: 0, target: { node: 0, path: 'weights' } }],
  }],
  accessors,
  bufferViews: views.map((view, index) => ({
    buffer: 0, byteOffset: offsets[index], byteLength: view.byteLength,
  })),
  buffers: [{
    byteLength: binary.length,
    uri: `data:application/octet-stream;base64,${Buffer.from(binary).toString('base64')}`,
  }],
};

const source = new Uint8Array(new TextEncoder().encode(JSON.stringify(manifest)));

/** Only the statements about the renderer; the rest is each path's own business. */
function limitWarnings(warnings) {
  return warnings.filter((warning) => /^Morph |^Animation .*targets at once/.test(warning)).sort();
}

const preview = limitWarnings((await buildSceneFromGltf(source, {}, gltfModule, {})).warnings);
const document = buildSceneDocumentFromGltf(source, {}, gltfModule);
const portable = limitWarnings(buildViewerSceneFromDocument(document).warnings);

assert.deepEqual(portable, preview, 'the two paths must report the renderer limits identically');
assert.deepEqual(preview, [
  `Animation Everything: a weights keyframe drives ${TARGETS} targets at once; the preview blends the ${MAX_ACTIVE_MORPH_TARGETS} strongest`,
  `Morph mesh Wide holds ${TARGETS} non-zero weights; the preview blends the ${MAX_ACTIVE_MORPH_TARGETS} strongest`,
  'Morph tangents on mesh 0 primitive 0 are ignored because the preview derives its tangent frame from deformed geometry and UVs',
], 'all three limits must be reported, spelled out here so a silent drop is visible');

console.log('viewer-limit-warnings: OK');
