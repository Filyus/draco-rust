import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { embeddedTriangle } from './smoke-fixtures.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');
const wantsWrite = process.argv.includes('--write');
const wantsEncoder = process.argv.includes('--draco-encode');
const api = await import(pathToFileURL(resolve(pkg, 'gltf_document.js')));
const wasm = await readFile(resolve(pkg, 'gltf_document_bg.wasm'));
await api.default({ module_or_path: wasm });

const document = new api.GltfDocument(new TextEncoder().encode(embeddedTriangle()), '2.0');
if ((typeof document.decompress === 'function') !== wantsWrite) {
  throw new Error('document write API feature gate is incorrect');
}
if ((typeof document.compressPrimitive === 'function') !== wantsEncoder) {
  throw new Error('document Draco encoder feature gate is incorrect');
}
if (wantsEncoder) {
  const bytes = document.compressPrimitive(0, 0, 5, 5);
  if (bytes === 0) {
    throw new Error('document Draco encoder wrote no payload');
  }
  const reloaded = new api.GltfDocument(document.glb(2), '2.0').summary();
  if (!reloaded.success || !reloaded.usesDraco) {
    throw new Error(`document Draco output failed to reload: ${JSON.stringify(reloaded)}`);
  }
}
console.log(`document features smoke passed (${wantsEncoder ? 'encode' : 'write'})`);
