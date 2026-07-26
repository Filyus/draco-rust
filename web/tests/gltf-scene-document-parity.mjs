import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { assertValidSceneDocument } from '../src/scene-document.ts';
import { buildSceneDocumentFromGltf } from '../src/gltf-scene-document.ts';
import { buildViewerSceneFromDocument } from '../src/scene-document-viewer.ts';
import { foxBin, foxGltf, here, loadWasm, readBytes } from './fbx-test-utils.mjs';

const gltf = await loadWasm('gltf');
const { buildSceneFromGltf } = await import(pathToFileURL(resolve(here, '..', 'www', 'gltf-loader.js')));
const { Viewer } = await import(pathToFileURL(resolve(here, '..', 'www', 'viewer.js')));
const resources = {
    'Fox.bin': await readBytes(foxBin),
    'Texture.png': new Uint8Array(await readFile(resolve(here, '..', '..', 'testdata', 'Fox', 'glTF', 'Texture.png'))),
};
const source = await readBytes(foxGltf);

const document = buildSceneDocumentFromGltf(source, resources, gltf);
const validation = assertValidSceneDocument(document);
assert.equal(validation.capabilities.skins, true);
assert.equal(validation.capabilities.animations, true);
assert.equal(document.nodes.length, 26);
assert.equal(document.meshes.length, 1);
assert.equal(document.skins.length, 1);
assert.equal(document.skins[0].joints.length, 24);
assert.equal(document.animations.length, 3);
assert.equal(document.resources.length, 1);
assert.equal(document.resources[0].mimeType, 'image/png');
assert.equal(document.meshes[0].primitives[0].attributes.POSITION >= 0, true);
assert.equal(document.meshes[0].primitives[0].attributes.JOINTS_0 >= 0, true);
assert.equal(document.meshes[0].primitives[0].attributes.WEIGHTS_0 >= 0, true);

const portableScene = buildViewerSceneFromDocument(document);
const directScene = await buildSceneFromGltf(source, resources, gltf);
assert.equal(portableScene.nodes.length, directScene.nodes.length);
assert.equal(portableScene.renderables.length, directScene.renderables.length);
assert.equal(portableScene.skins[0].joints.length, directScene.skins[0].joints.length);
assert.equal(portableScene.animations.length, directScene.animations.length);
assert.deepEqual(
    Array.from(portableScene.skins[0].joints[0].inverseBind),
    Array.from(directScene.skins[0].joints[0].inverseBind),
);

function sample(scene, time) {
    const probe = Object.create(Viewer.prototype);
    probe.scene = scene;
    probe.animation = { clipIndex: 0, time: 0 };
    assert.equal(probe.seekAnimation(time), true);
    probe._updateWorldMatrices();
    return scene.nodes.map((node) => Array.from(node.world));
}

const time = portableScene.animations[0].duration * 0.5;
const expected = sample(directScene, time);
const actual = sample(portableScene, time);
let worst = 0;
for (let node = 0; node < actual.length; node += 1) {
    for (let component = 0; component < 16; component += 1) {
        worst = Math.max(worst, Math.abs(actual[node][component] - expected[node][component]));
    }
}
assert.ok(worst < 1e-6, `Fox viewer parity drift ${worst}`);

const values = new Float32Array([
    0, 0, 0, 1, 0, 0, 0, 1, 0,
    0, 0, 0, 0, 0.25, 0, 0, 0, 0,
]);
const morphSource = new TextEncoder().encode(JSON.stringify({
    asset: { version: '2.0' },
    buffers: [{ uri: `data:application/octet-stream;base64,${Buffer.from(values.buffer).toString('base64')}`, byteLength: values.byteLength }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }, { buffer: 0, byteOffset: 36, byteLength: 36 }],
    accessors: [
        { bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] },
        { bufferView: 1, componentType: 5126, count: 3, type: 'VEC3' },
    ],
    meshes: [{ weights: [0], primitives: [{ attributes: { POSITION: 0 }, targets: [{ POSITION: 1 }] }] }],
    nodes: [{ mesh: 0 }], scenes: [{ nodes: [0] }], scene: 0,
}));
const morphDocument = buildSceneDocumentFromGltf(morphSource, {}, gltf);
assert.equal(morphDocument.meshes[0].primitives[0].targets.length, 1);
assert.equal(morphDocument.nodes[0].weights.length, 1);
assertValidSceneDocument(morphDocument);

console.log(`glTF SceneDocument Fox parity passed (max world drift=${worst.toExponential(2)})`);
