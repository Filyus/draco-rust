import fs from 'node:fs';
import assert from 'node:assert/strict';
import { gzipSync } from 'node:zlib';
import { performance } from 'node:perf_hooks';
import draco3d from '../../web/node_modules/draco3d/draco3d.js';
import { MeshoptEncoder, MeshoptDecoder } from './target/bench/node_modules/meshoptimizer/index.js';
import {
  decode_draco,
  initSync,
} from './target/bench/node_modules/@draco-rust/decoder/point-cloud/index.js';
import {
  decode_draco as decodeFull,
  initSync as initFull,
} from './target/bench/node_modules/@draco-rust/decoder/index.js';

const here = new URL('./', import.meta.url);
const base = 'KHR_gaussian_splatting',
  child = 'TEST_gaussian_splatting_compression_draco';
const mesh = 'KHR_draco_mesh_compression',
  opt = 'KHR_meshopt_compression';
const raw = fs.readFileSync('testdata/GaussianSplats/tiny-splat.bin');
const source = JSON.parse(fs.readFileSync('testdata/GaussianSplats/tiny-splat.gltf'));
const original = fs.readFileSync(new URL('mixed.glb', here));
function parse(bytes) {
  const end = 20 + bytes.readUInt32LE(12);
  return [JSON.parse(bytes.subarray(20, end)), bytes.subarray(end + 8)];
}
function pack(doc, bin) {
  const json = Buffer.from(JSON.stringify(doc));
  const padded = Math.ceil(json.length / 4) * 4;
  const bytes = Buffer.alloc(28 + padded + Math.ceil(bin.length / 4) * 4);
  bytes.write('glTF');
  bytes.writeUInt32LE(2, 4);
  bytes.writeUInt32LE(bytes.length, 8);
  bytes.writeUInt32LE(padded, 12);
  bytes.write('JSON', 16);
  bytes.fill(32, 20, 20 + padded);
  json.copy(bytes, 20);
  bytes.writeUInt32LE(bytes.length - 28 - padded, 20 + padded);
  bytes.write('BIN\0', 24 + padded);
  bin.copy(bytes, 28 + padded);
  return bytes;
}
await Promise.all([MeshoptEncoder.ready, MeshoptDecoder.ready]);
const mod = await draco3d.createDecoderModule();
const [doc, bin] = parse(original);
const prim = doc.meshes[1].primitives[0];
const offset = doc.bufferViews[1].byteOffset;
const stride = source.bufferViews[0].byteStride,
  count = source.accessors[0].count;
const compressed = Buffer.from(
  MeshoptEncoder.encodeGltfBuffer(raw, count, stride, 'ATTRIBUTES', 1),
);
const alternativeBin = Buffer.concat([bin.subarray(0, offset), compressed]);
doc.extensionsRequired = [base, mesh, opt];
doc.extensionsUsed = [...doc.extensionsRequired];
delete prim.extensions[base].extensions;
doc.buffers = [
  { byteLength: alternativeBin.length },
  { byteLength: raw.length, extensions: { [opt]: { fallback: true } } },
];
doc.bufferViews[1] = {
  buffer: 1,
  byteLength: raw.length,
  byteStride: stride,
  extensions: {
    [opt]: {
      buffer: 0,
      byteOffset: offset,
      byteLength: compressed.length,
      count,
      byteStride: stride,
      mode: 'ATTRIBUTES',
    },
  },
};
for (const [semantic, index] of Object.entries(prim.attributes)) {
  const src = source.accessors[source.meshes[0].primitives[0].attributes[semantic]];
  Object.assign(doc.accessors[index], { bufferView: 1, byteOffset: src.byteOffset });
}
const alternative = pack(doc, alternativeBin);
fs.writeFileSync(new URL('target/mixed-meshopt.glb', here), alternative);
function load(glb) {
  const [scene, data] = parse(glb);
  const result = new Uint8Array(raw.length);
  const floats = new Float32Array(result.buffer);
  for (const {
    primitives: [p],
  } of scene.meshes) {
    const splat = p.mode === 0;
    if (splat && scene.extensionsRequired.includes(opt)) {
      const ext =
        scene.bufferViews[scene.accessors[p.attributes.POSITION].bufferView].extensions[opt];
      MeshoptDecoder.decodeGltfBuffer(
        result,
        ext.count,
        ext.byteStride,
        data.subarray(ext.byteOffset, ext.byteOffset + ext.byteLength),
        ext.mode,
      );
      continue;
    }
    const ext = splat ? p.extensions[base].extensions[child] : p.extensions[mesh];
    const view = scene.bufferViews[ext.bufferView];
    const input = new mod.DecoderBuffer(),
      dec = new mod.Decoder(),
      shape = splat ? new mod.PointCloud() : new mod.Mesh();
    try {
      input.Init(
        data.subarray(view.byteOffset, view.byteOffset + view.byteLength),
        view.byteLength,
      );
      const status = splat
        ? dec.DecodeBufferToPointCloud(input, shape)
        : dec.DecodeBufferToMesh(input, shape);
      assert.ok(status.ok());
      if (!splat) continue;
      for (const [semantic, id] of Object.entries(ext.attributes)) {
        const attr = dec.GetAttributeByUniqueId(shape, id);
        const n = attr.num_components(),
          bytes = shape.num_points() * n * 4;
        const ptr = mod._malloc(bytes);
        try {
          assert.ok(dec.GetAttributeDataArrayForAllPoints(shape, attr, mod.DT_FLOAT32, bytes, ptr));
          const col = mod.HEAPF32.subarray(ptr / 4, ptr / 4 + bytes / 4);
          const start =
            source.accessors[source.meshes[0].primitives[0].attributes[semantic]].byteOffset / 4;
          for (let i = 0; i < count; i++)
            floats.set(col.subarray(i * n, (i + 1) * n), (i * stride) / 4 + start);
        } finally {
          mod._free(ptr);
        }
      }
    } finally {
      mod.destroy(shape);
      mod.destroy(input);
      mod.destroy(dec);
    }
  }
  return result;
}
const files = [original, alternative],
  times = [[], []];
for (const f of files) assert.deepEqual(Buffer.from(load(f)), raw);
// The same splat stream through the published `@draco-rust` decoder: the
// attributes come back in their own types, floats included, so the check is
// the same byte-for-byte comparison the Google decoder passes.
initSync({
  module: new WebAssembly.Module(
    fs.readFileSync(
      new URL('target/bench/node_modules/@draco-rust/decoder/point-cloud/index_bg.wasm', here),
    ),
  ),
});
{
  // `doc` was mutated to build the Meshopt arm; parse the original again.
  const [freshDoc, data] = parse(original);
  const prim = freshDoc.meshes[1].primitives[0];
  const ext = prim.extensions[base].extensions[child];
  const view = freshDoc.bufferViews[ext.bufferView];
  const stream = data.subarray(view.byteOffset, view.byteOffset + view.byteLength);
  const result = decode_draco(stream);
  assert.equal(result.geometry, 'point_cloud');
  const rebuilt = Buffer.from(raw);
  const floats = new Float32Array(rebuilt.buffer);
  for (const [semantic, id] of Object.entries(ext.attributes)) {
    const attr = result.attributes.find((a) => a.uniqueId === id);
    assert.ok(attr, semantic);
    const start =
      source.accessors[source.meshes[0].primitives[0].attributes[semantic]].byteOffset / 4;
    for (let i = 0; i < count; i++)
      for (let c = 0; c < attr.itemSize; c++)
        floats[(i * stride) / 4 + start + c] = attr.array[i * attr.itemSize + c];
  }
  assert.deepEqual(rebuilt, raw);

  initFull({
    module: fs.readFileSync(
      new URL('target/bench/node_modules/@draco-rust/decoder/index_bg.wasm', here),
    ),
  });
  assert.deepEqual(decodeFull(stream), result);
  const triangle = freshDoc.meshes[0].primitives[0].extensions[mesh];
  const meshView = freshDoc.bufferViews[triangle.bufferView];
  const start = meshView.byteOffset ?? 0;
  const decodedMesh = decodeFull(data.subarray(start, start + meshView.byteLength));
  assert.equal(decodedMesh.geometry, 'mesh');
}
for (let round = 0; round < 120; round++) {
  for (const k of [round % 2, 1 - (round % 2)]) {
    const start = performance.now();
    load(files[k]);
    if (round >= 20) times[k].push(performance.now() - start);
  }
}
for (const [i, label] of ['Draco', 'Draco + Meshopt'].entries()) {
  const ordered = times[i].sort((a, b) => a - b);
  console.log(
    label,
    'GLB bytes',
    files[i].length,
    'gzip bytes',
    gzipSync(files[i]).length,
    'warm median ms',
    ((ordered[49] + ordered[50]) / 2).toFixed(3),
  );
}
for (const variant of ['', 'gltf/']) {
  const prefix = 'web/node_modules/three/examples/jsm/libs/draco/' + variant;
  const files = ['draco_decoder.wasm', 'draco_wasm_wrapper.js'].map((f) =>
    fs.readFileSync(prefix + f),
  );
  if (variant)
    files.push(
      fs.readFileSync(new URL('target/bench/node_modules/meshoptimizer/meshopt_decoder.mjs', here)),
    );
  console.log(
    variant ? 'mesh-only Draco + Meshopt' : 'full Draco',
    'decoder gzip bytes',
    files.reduce((sum, f) => sum + gzipSync(f).length, 0),
  );
}
{
  const pkg = 'target/bench/node_modules/@draco-rust/decoder/';
  const pair = (build) =>
    [pkg + build + 'index.js', pkg + build + 'index_bg.wasm'].map((f) =>
      fs.readFileSync(new URL(f, here)),
    );
  console.log(
    '@draco-rust point-cloud',
    'decoder gzip bytes',
    pair('point-cloud/').reduce((sum, f) => sum + gzipSync(f).length, 0),
  );
  console.log(
    '@draco-rust full',
    'decoder gzip bytes',
    pair('').reduce((sum, f) => sum + gzipSync(f).length, 0),
  );
}
