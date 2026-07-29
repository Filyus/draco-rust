import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { validateBytes } from 'gltf-validator';

import { decodeFirstDracoPrimitive } from './draco-interop.ts';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');
const api = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href) as any;
const wasm = await readFile(resolve(pkg, 'gltf_bg.wasm'));
await api.default({ module_or_path: wasm });

const values = new Float32Array([
  0, 0, 0,
  1, 0, 0,
  0, 1, 0,
]);
const positions = new Uint8Array(values.buffer.slice(0));
const indices = new Uint8Array(new Uint16Array([0, 1, 2]).buffer);
const geometry = new api.PackedGeometry(4);
geometry.addAttribute('POSITION', 3, 3, 5126, false, positions);
geometry.setIndices(3, 5123, indices);
geometry.validate('2.0');

const options = new api.GeometryWriteOptions();
const draco = process.argv.includes('--draco');
if (draco) {
  if (typeof options.useDraco !== 'function') {
    throw new Error('Draco writer method is missing');
  }
  options.useDraco(5, 5, false);
}
const document = api.GltfAsset.fromGeometry(geometry, '2.0', options);
const glb = document.glb(2);
const validation = await validateBytes(glb, { uri: 'packed-geometry.glb' });
if (validation.issues.numErrors !== 0) {
  throw new Error(`generated GLB is invalid: ${JSON.stringify(validation.issues.messages)}`);
}
const reloaded = new api.GltfAsset(glb, '2.0');
const packed = reloaded.readPrimitive(0, 0);
if (
  packed.attributeSemantic(0) !== 'POSITION'
  || packed.attributeElementCount(0) !== 3
  || packed.attributeBytes(0).length !== 36
  || packed.indexCount() !== 3
) {
  throw new Error('glTF write roundtrip failed');
}
if (draco) {
  const decoded = await decodeFirstDracoPrimitive(glb);
  if (
    decoded.points !== 3
    || decoded.faces !== 1
    || decoded.declaredPoints !== decoded.points
    || decoded.declaredIndices !== decoded.faces * 3
  ) {
    throw new Error(`official draco3d metadata mismatch: ${JSON.stringify(decoded)}`);
  }
}
const bundle = document.gltfBundle();
if (bundle.resourceCount() !== 1 || bundle.resourceBytes(0).length === 0) {
  throw new Error('glTF JSON bundle is incomplete');
}
console.log(`glTF ${draco ? 'Draco' : 'raw'} writer smoke passed`);
