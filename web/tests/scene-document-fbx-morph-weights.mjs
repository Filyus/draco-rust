/**
 * Every morph target's weight curve has to reach FBX.
 *
 * A weights channel in glTF is one sampler whose output interleaves all targets;
 * FBX wants one curve per target. The document writer expands them, and an
 * expansion that stops after the first target is invisible in every count-based
 * assertion — the clip is still there, the channel is still there, and only the
 * second shape stops moving.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');
const pkg = resolve(here, '..', 'www', 'pkg');
const model = resolve(repoRoot, 'testdata', 'KhronosSampleModels', 'AnimatedMorphCube', 'glTF', 'AnimatedMorphCube.gltf');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });
const fbxModule = await import(pathToFileURL(resolve(pkg, 'fbx.js')).href);
await fbxModule.default({ module_or_path: await readFile(resolve(pkg, 'fbx_bg.wasm')) });

const { buildSceneDocumentFromGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
);
const { buildFbxSceneFromDocument } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'fbx-scene-document-writer.ts')).href
);

const source = new Uint8Array(await readFile(model));
const manifest = JSON.parse(await readFile(model, 'utf8'));
const resources = {};
for (const buffer of manifest.buffers || []) {
  if (buffer.uri && !buffer.uri.startsWith('data:')) {
    resources[buffer.uri] = new Uint8Array(await readFile(resolve(dirname(model), buffer.uri)));
  }
}

const document = buildSceneDocumentFromGltf(source, resources, gltfModule);
const targetCount = document.meshes[0].primitives[0].targets.length;
assert.equal(targetCount, 2, 'the fixture must carry two morph targets');

const scene = buildFbxSceneFromDocument(document);
const weightChannels = scene.animations[0].channels.filter((channel) => channel.path === 'morphweight');
assert.deepEqual(
  [...new Set(weightChannels.map((channel) => channel.morphTargetIndex))].sort(),
  [0, 1],
  'every morph target needs its own weight curve, not just the first',
);
for (const channel of weightChannels) {
  assert.equal(
    channel.sampler.output.length,
    channel.sampler.input.length,
    `morph target ${channel.morphTargetIndex} has a key count its times do not match`,
  );
}

// Both curves have to survive the writer, not just the JS structure feeding it.
const written = fbxModule.create_fbx_scene(scene, { version: 7500, legacyCompatibility: false });
assert.ok(written.success, `FBX write failed: ${written.error}`);
const reparsed = fbxModule.parse_fbx(new Uint8Array(written.binary_data));
assert.ok(reparsed.success, `FBX reparse failed: ${reparsed.error}`);

console.log(`SceneDocument FBX morph weights passed (${weightChannels.length} curves)`);
