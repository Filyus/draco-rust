/**
 * `PrimitiveReader` stands in for `readPrimitive`: whatever the batch size,
 * and whether a request follows the order it was given or not, every
 * primitive comes back as `readPrimitive` returns it.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { PrimitiveReader, documentPrimitiveOrder } from '../src/gltf-primitive-reader.ts';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');
const pkg = resolve(here, '..', 'www', 'pkg');
const models = [
  resolve(repoRoot, 'testdata', 'gltf_transform', 'two_objects_sequential.glb'),
  resolve(repoRoot, 'testdata', 'gltf_transform', 'two_objects_edgebreaker_speed0.glb'),
];

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

/** Everything a primitive carries, as one comparable value; frees it. */
function snapshot(packed: any) {
  try {
    const attributes = [];
    for (let index = 0; index < packed.attributeCount(); index += 1) {
      attributes.push([
        packed.attributeSemantic(index),
        packed.attributeComponents(index),
        packed.attributeComponentType(index),
        Array.from(new Uint8Array(packed.attributeBytes(index))),
      ]);
    }
    const indices = packed.hasIndices() ? Array.from(new Uint8Array(packed.indexBytes())) : null;
    return { mode: packed.mode(), attributes, indices };
  } finally {
    packed.free();
  }
}

for (const model of models) {
  const asset = new gltfModule.GltfAsset(new Uint8Array(await readFile(model)), '2.0');
  try {
    const counts = Array.from({ length: asset.meshCount() }, (_, mesh) => asset.primitiveCount(mesh));
    const order = documentPrimitiveOrder(counts);
    assert.ok(order.length >= 2, `${model} has fewer than two primitives`);
    const expected = order.map(([mesh, primitive]) => snapshot(asset.readPrimitive(mesh, primitive)));

    for (const batch of [1, 2, 32]) {
      const reader = new PrimitiveReader(asset, order, batch);
      try {
        const read = order.map(([mesh, primitive]) => snapshot(reader.read(mesh, primitive)));
        assert.deepEqual(read, expected, `batch ${batch}`);
      } finally {
        reader.dispose();
      }
    }

    // A request off the planned order is read on its own, and the planned
    // ones still come back right afterwards.
    const reader = new PrimitiveReader(asset, order, 32);
    try {
      const [lastMesh, lastPrimitive] = order[order.length - 1];
      assert.deepEqual(snapshot(reader.read(lastMesh, lastPrimitive)), expected[expected.length - 1]);
      const read = order.map(([mesh, primitive]) => snapshot(reader.read(mesh, primitive)));
      assert.deepEqual(read, expected, 'after an off-order request');
    } finally {
      reader.dispose();
    }

    // Disposing with a batch fetched and not read frees the rest.
    const partial = new PrimitiveReader(asset, order, 32);
    snapshot(partial.read(order[0][0], order[0][1]));
    partial.dispose();
    partial.dispose();
  } finally {
    asset.free();
  }
}

console.log(`gltf-primitive-reader: OK (${models.length} models)`);
