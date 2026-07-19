import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { embeddedTriangle } from './smoke-fixtures.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');
const wantsWrite = process.argv.includes('--write');
const wantsEncoder = process.argv.includes('--draco-encode');
const wantsAccessors = process.argv.includes('--accessors');
const wantsRawResources = process.argv.includes('--raw-resources');
const api = await import(pathToFileURL(resolve(pkg, 'gltf.js')));
const wasm = await readFile(resolve(pkg, 'gltf_bg.wasm'));
await api.default({ module_or_path: wasm });

const asset = new api.GltfAsset(new TextEncoder().encode(embeddedTriangle()), '2.0');
if ((typeof asset.decompress === 'function') !== wantsWrite) {
  throw new Error('glTF write API feature gate is incorrect');
}
if ((typeof asset.compressPrimitive === 'function') !== wantsEncoder) {
  throw new Error('glTF Draco encoder feature gate is incorrect');
}
if ((typeof asset.bufferBytes === 'function') !== wantsRawResources) {
  throw new Error('glTF raw resource API feature gate is incorrect');
}
if ((typeof asset.readAccessor === 'function') !== wantsAccessors) {
  throw new Error('glTF accessor API feature gate is incorrect');
}
if ((typeof asset.bufferViewBytes === 'function') !== wantsRawResources) {
  throw new Error('glTF buffer-view API feature gate is incorrect');
}
if (wantsAccessors) {
  const accessor = asset.readAccessor(0);
  if (accessor.count() !== 3 || accessor.components() !== 3 || accessor.bytes().length !== 36) {
    throw new Error('glTF accessor API returned invalid data');
  }
}
if (wantsRawResources) {
  if (asset.bufferCount() !== 1 || asset.bufferBytes(0).length !== 36) {
    throw new Error('glTF raw resource API returned an invalid buffer');
  }
}
if (wantsEncoder) {
  const bytes = asset.compressPrimitive(0, 0, 5, 5);
  if (bytes === 0) {
    throw new Error('glTF Draco encoder wrote no payload');
  }
  const reloaded = new api.GltfAsset(asset.glb(2), '2.0').summary();
  if (!reloaded.success || !reloaded.usesDraco) {
    throw new Error(`glTF Draco output failed to reload: ${JSON.stringify(reloaded)}`);
  }
}
console.log(
  `glTF features smoke passed (${[
    wantsWrite && 'write',
    wantsEncoder && 'encode',
    wantsAccessors && 'accessors',
    wantsRawResources && 'raw-resources',
  ].filter(Boolean).join(',') || 'read'})`,
);
