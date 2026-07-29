/**
 * A glTF module built without the converter profile must say so.
 *
 * `accessors` and `raw-resources` are cargo features, so a release-profile
 * module satisfies the hand-written `GltfAsset` interface at compile time and
 * then lacks half of it at run time. Until this check existed, the first
 * skinned, animated or morphed asset failed deep inside the loader as
 * `asset.readAccessor is not a function` — a message that names neither the
 * cause nor the cure, and one that cost a full CI job to diagnose.
 *
 * The stub below is the release profile as the front-end sees it: everything
 * the `read` feature provides, and nothing else.
 */
import assert from 'node:assert/strict';

import { buildSceneFromGltf } from '../src/gltf-loader.ts';
import { buildSceneDocumentFromGltf } from '../src/gltf-scene-document.ts';

let freed = 0;

/** A GltfAsset carrying only what the default `read` feature builds. */
function releaseProfileAsset() {
  return {
    free() { freed += 1; },
    json: () => new TextEncoder().encode(JSON.stringify({ asset: { version: '2.0' } })),
    minifiedJson: () => new Uint8Array(),
    glb: () => new Uint8Array(),
    validate() {},
    meshCount: () => 0,
    primitiveCount: () => 0,
    readPrimitive() { throw new Error('no primitives in this stub'); },
  };
}

const releaseModule = { GltfAsset: { withResources: releaseProfileAsset } } as any;
const source = new Uint8Array([0x67, 0x6c, 0x54, 0x46]);

for (const [label, open] of [
  ['preview', () => buildSceneFromGltf(source, {}, releaseModule, {})],
  ['document', () => buildSceneDocumentFromGltf(source, {}, releaseModule)],
] as const) {
  // The document builder throws synchronously and the preview asynchronously,
  // so the call itself goes inside the try rather than onto a promise chain.
  let error: any = null;
  try {
    await open();
  } catch (thrown) {
    error = thrown;
  }
  assert.ok(error, `${label} accepted a module without accessor reads`);
  const message = String(error.message);
  assert.match(message, /converter profile/, `${label} did not name the profile: ${message}`);
  assert.match(message, /readAccessor and bufferViewBytes/, `${label} did not name what is missing: ${message}`);
  assert.match(message, /--app/, `${label} did not name the fix: ${message}`);
}

// The asset owns WASM memory, so the guard has to fire inside the try block
// that frees it rather than before the asset exists.
assert.equal(freed, 2, 'a rejected asset was leaked instead of freed');

// A module that has the methods is not rejected: the check must be about the
// build profile, not about being handed a stub.
const converterModule = {
  GltfAsset: {
    withResources: () => ({
      ...releaseProfileAsset(),
      readAccessor() { throw new Error('reached the loader'); },
      bufferViewBytes: () => new Uint8Array(),
    }),
  },
} as any;
let passed: any = null;
try {
  await buildSceneFromGltf(source, {}, converterModule, {});
} catch (thrown) {
  passed = thrown;
}
assert.doesNotMatch(String(passed?.message ?? ''), /converter profile/, 'a converter-profile module was rejected');

console.log('gltf-build-profile: OK');
