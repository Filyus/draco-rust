/**
 * `KHR_mesh_quantization` assets (anything gltfpack touched) store morph deltas
 * as integers and animation outputs as normalized integers. The preview blends
 * morph deltas through a float texture and interpolates float weights, so both
 * have to be expanded on load — otherwise the mesh silently stays at its rest
 * pose while the clip plays, which is what a quantized facecap.glb used to do.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });
const { buildSceneFromGltf } = await import(pathToFileURL(resolve(here, '..', 'www', 'gltf-loader.js')).href);

/** Concatenates typed arrays into one buffer, 4-byte aligning each view. */
function packViews(views) {
  const offsets = [];
  let length = 0;
  for (const view of views) {
    offsets.push(length);
    length += Math.ceil(view.byteLength / 4) * 4;
  }
  const bytes = new Uint8Array(length);
  views.forEach((view, index) => {
    bytes.set(new Uint8Array(view.buffer, view.byteOffset, view.byteLength), offsets[index]);
  });
  return { bytes, offsets };
}

function glb(json, bin) {
  const jsonBytes = new TextEncoder().encode(JSON.stringify(json));
  const jsonChunk = new Uint8Array(Math.ceil(jsonBytes.length / 4) * 4).fill(0x20);
  jsonChunk.set(jsonBytes);
  const binChunk = new Uint8Array(Math.ceil(bin.length / 4) * 4);
  binChunk.set(bin);
  const total = 12 + 8 + jsonChunk.length + 8 + binChunk.length;
  const out = new Uint8Array(total);
  const view = new DataView(out.buffer);
  view.setUint32(0, 0x46546c67, true);
  view.setUint32(4, 2, true);
  view.setUint32(8, total, true);
  view.setUint32(12, jsonChunk.length, true);
  view.setUint32(16, 0x4e4f534a, true);
  out.set(jsonChunk, 20);
  view.setUint32(20 + jsonChunk.length, binChunk.length, true);
  view.setUint32(24 + jsonChunk.length, 0x004e4942, true);
  out.set(binChunk, 28 + jsonChunk.length);
  return out;
}

// Positions are raw quantized counts, morph deltas share that space, morph
// normals are normalized bytes, and the weight track is a normalized byte.
const positions = new Uint16Array([0, 0, 0, 1000, 0, 0, 0, 1000, 0]);
const morphPositions = new Int16Array([0, 0, 0, 250, 0, 0, 0, -250, 0]);
const morphNormals = new Int8Array([0, 0, 127, 0, 0, 127, 0, 0, 127]);
const indices = new Uint16Array([0, 1, 2]);
const times = new Float32Array([0, 1]);
const weights = new Uint8Array([0, 255]);

const { bytes: bin, offsets } = packViews([
  positions, morphPositions, morphNormals, indices, times, weights,
]);

const document = {
  asset: { version: '2.0' },
  extensionsUsed: ['KHR_mesh_quantization'],
  extensionsRequired: ['KHR_mesh_quantization'],
  scene: 0,
  scenes: [{ nodes: [0] }],
  nodes: [{ mesh: 0, weights: [0], scale: [0.001, 0.001, 0.001] }],
  meshes: [{
    primitives: [{
      attributes: { POSITION: 0 },
      indices: 3,
      targets: [{ POSITION: 1, NORMAL: 2 }],
    }],
  }],
  animations: [{
    samplers: [{ input: 4, output: 5, interpolation: 'LINEAR' }],
    channels: [{ sampler: 0, target: { node: 0, path: 'weights' } }],
  }],
  accessors: [
    { bufferView: 0, componentType: 5123, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1000, 1000, 0] },
    { bufferView: 1, componentType: 5122, count: 3, type: 'VEC3' },
    { bufferView: 2, componentType: 5120, count: 3, type: 'VEC3', normalized: true },
    { bufferView: 3, componentType: 5123, count: 3, type: 'SCALAR' },
    { bufferView: 4, componentType: 5126, count: 2, type: 'SCALAR' },
    { bufferView: 5, componentType: 5121, count: 2, type: 'SCALAR', normalized: true },
  ],
  bufferViews: [
    { buffer: 0, byteOffset: offsets[0], byteLength: positions.byteLength },
    { buffer: 0, byteOffset: offsets[1], byteLength: morphPositions.byteLength },
    { buffer: 0, byteOffset: offsets[2], byteLength: morphNormals.byteLength },
    { buffer: 0, byteOffset: offsets[3], byteLength: indices.byteLength },
    { buffer: 0, byteOffset: offsets[4], byteLength: times.byteLength },
    { buffer: 0, byteOffset: offsets[5], byteLength: weights.byteLength },
  ],
  buffers: [{ byteLength: bin.length }],
};

const source = glb(document, bin);
const warnings = [];
const scene = await buildSceneFromGltf(source, {}, gltfModule, {
  onLog: (message, level) => { if (level === 'warning') warnings.push(message); },
  loadImage: async () => null,
});

const primitive = scene.meshes[0].primitives[0];

assert.ok(primitive.morphPositions[0], 'quantized morph positions were dropped');
assert.equal(primitive.morphPositions[0].componentType, 5126);
assert.deepEqual(
  Array.from(new Float32Array(primitive.morphPositions[0].bytes.buffer, 0, 9)),
  Array.from(morphPositions, Number),
  'plain quantized deltas must keep their counts; the node scale converts them',
);

assert.ok(primitive.morphNormals[0], 'quantized morph normals were dropped');
assert.deepEqual(
  Array.from(new Float32Array(primitive.morphNormals[0].bytes.buffer, 0, 9)),
  Array.from(morphNormals, (value) => Math.max(value / 127, -1)),
  'normalized deltas must be expanded to unit range',
);

const sampler = scene.animations[0].channels[0].sampler;
assert.ok(sampler.output instanceof Float32Array, 'normalized weights stayed integer');
assert.deepEqual(Array.from(sampler.output), [0, 1]);

assert.deepEqual(
  warnings.filter((message) => /unsupported/i.test(message)),
  [],
  'quantized accessors must not be reported as unsupported',
);

// The portable document is float-only, so the same asset has to survive the
// SceneDocument path that drives the scene tree and every export.
const { buildSceneDocumentFromGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
);
const { assertValidSceneDocument } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'scene-document.ts')).href
);

const sceneDocument = buildSceneDocumentFromGltf(source, {}, gltfModule);
assertValidSceneDocument(sceneDocument);
// The Rust reader resolves quantized attributes into the document losslessly,
// so telling the user they were "omitted" — or that the asset requires
// something the portable subset lacks — is a false alarm about a file that
// came through intact.
assert.deepEqual(
  sceneDocument.warnings.filter((message) => /omitted from SceneDocument|outside the portable/.test(message)),
  [],
  'reader-resolved extensions must not be reported as dropped from the document',
);
const output = sceneDocument.accessors[sceneDocument.animations[0].samplers[0].output];
assert.equal(output.componentType, 5126);
assert.deepEqual(Array.from(new Float32Array(output.bytes.buffer, output.bytes.byteOffset, 2)), [0, 1]);

console.log('gltf-quantized-morph: OK');
