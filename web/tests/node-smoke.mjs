import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { assertSmokeResults } from './smoke-fixtures.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

async function load(name) {
  const module = await import(pathToFileURL(resolve(pkg, `${name}.js`)));
  const wasm = await readFile(resolve(pkg, `${name}_bg.wasm`));
  await module.default({ module_or_path: wasm });
  return module;
}

const [reader, writer] = await Promise.all([
  load('gltf_reader'),
  load('gltf_writer'),
]);
assertSmokeResults(reader, writer);
console.log('Node WASM smoke passed');
