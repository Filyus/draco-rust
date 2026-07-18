import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { embeddedTriangle, externalTriangle, triangleBytes } from './smoke-fixtures.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

async function load(name) {
  const module = await import(pathToFileURL(resolve(pkg, `${name}.js`)));
  const wasm = await readFile(resolve(pkg, `${name}_bg.wasm`));
  await module.default({ module_or_path: wasm });
  return module;
}

const [documentApi, compactApi] = await Promise.all([
  load('gltf_document'),
  load('gltf_compact'),
]);
const input = new TextEncoder().encode(embeddedTriangle());
const document = new documentApi.GltfDocument(input, '2.0');
const summary = document.summary();
if (!summary.success || summary.meshCount !== 1 || summary.primitiveCount !== 1) {
  throw new Error(`document smoke failed: ${JSON.stringify(summary)}`);
}

const compact = new compactApi.CompactDocument(input, '2.0');
const primitive = compact.readPrimitive(0, 0);
if ('GeometryWriteOptions' in compactApi || typeof compactApi.CompactDocument.fromGeometry === 'function') {
  throw new Error('default compact artifact unexpectedly includes writer API');
}
if (
  compact.meshCount() !== 1
  || compact.primitiveCount(0) !== 1
  || primitive.attributeCount() !== 1
  || primitive.attributeSemantic(0) !== 'POSITION'
  || primitive.attributeBytes(0).length !== 36
) {
  throw new Error('compact geometry smoke failed');
}

const glb = document.glb(2);
const roundtrip = new documentApi.GltfDocument(glb, '2.0').summary();
if (!roundtrip.success || roundtrip.meshCount !== 1) {
  throw new Error(`GLB roundtrip failed: ${JSON.stringify(roundtrip)}`);
}

let missingFailed = false;
try {
  new documentApi.GltfDocument(new TextEncoder().encode(externalTriangle()), '2.0');
} catch (error) {
  missingFailed = String(error).includes('missing.bin');
}
if (!missingFailed) {
  throw new Error('missing-resource smoke failed');
}
const resolved = documentApi.GltfDocument.withResources(
  new TextEncoder().encode(externalTriangle()),
  { 'missing.bin': triangleBytes() },
  '2.0',
).summary();
if (!resolved.success || resolved.meshCount !== 1) {
  throw new Error(`resource-map smoke failed: ${JSON.stringify(resolved)}`);
}
console.log('Node WASM smoke passed');
