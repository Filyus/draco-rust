import assert from 'node:assert/strict';
import { createSceneDocument } from '../src/scene-document.ts';
import { buildViewerSceneFromDocument } from '../src/scene-document-viewer.ts';

globalThis.WebGL2RenderingContext = class {};
const { Viewer } = await import('../www/viewer.js');

function bytes(values) {
    return new Uint8Array(values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength));
}

const document = createSceneDocument({
    resources: [{ name: 'texture.png', mimeType: 'image/png', bytes: new Uint8Array([137, 80, 78, 71]) }],
    textures: [{ name: 'texture', resource: 0 }],
    materials: [{
        baseColorFactor: [0.25, 0.5, 1, 1],
        metallicFactor: 0.1,
        roughnessFactor: 0.8,
        emissiveFactor: [0, 0, 0],
        baseColorTexture: { texture: 0 },
    }],
    accessors: [
        { bytes: bytes(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0])), componentType: 5126, components: 3, count: 3 },
        { bytes: bytes(new Uint16Array([0, 1, 2])), componentType: 5123, components: 1, count: 3 },
        { bytes: bytes(new Float32Array([0, 1])), componentType: 5126, components: 1, count: 2 },
        { bytes: bytes(new Float32Array([0, 0, 0, 2, 0, 0])), componentType: 5126, components: 3, count: 2 },
        { bytes: bytes(new Float32Array([
            1, 0, 0, 0,
            0, 1, 0, 0,
            0, 0, 1, 0,
            0, 0, 0, 1,
        ])), componentType: 5126, components: 16, count: 1 },
    ],
    // The second primitive deliberately declares no material: glTF says such a
    // primitive takes the renderer's own default, never materials[0].
    meshes: [{
        primitives: [
            { attributes: { POSITION: 0 }, indices: 1, material: 0 },
            { attributes: { POSITION: 0 }, indices: 1 },
        ],
    }],
    nodes: [{
        name: 'Mesh', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0, skin: 0, children: [1],
    }, {
        name: 'Joint', translation: [0, 1, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1],
    }],
    rootNodes: [0],
    skins: [{ joints: [1], inverseBindMatrices: 4 }],
    animations: [{
        name: 'Slide', duration: 1,
        samplers: [{ input: 2, output: 3, interpolation: 'LINEAR' }],
        channels: [{ sampler: 0, node: 0, path: 'translation' }],
    }],
});

const scene = buildViewerSceneFromDocument(document);
assert.equal(scene.nodes.length, 2);
assert.equal(scene.renderables.length, 1);
assert.equal(scene.skins[0].joints[0].node, scene.nodes[1]);
assert.deepEqual(Array.from(scene.skins[0].joints[0].inverseBind), [
    1, 0, 0, 0,
    0, 1, 0, 0,
    0, 0, 1, 0,
    0, 0, 0, 1,
]);
assert.equal(scene.meshes[0].primitives[0].attributes.POSITION.count, 3);
assert.equal(scene.materials[0].baseColorTexture, 0);
assert.equal(scene.meshes[0].primitives[0].materialIndex, 0);
assert.equal(
    scene.meshes[0].primitives[1].materialIndex,
    -1,
    'a primitive without a material must not borrow materials[0]',
);
assert.equal(scene.textures[0].image, undefined, 'runtime adapter must not inject browser image handles');
assert.ok(scene.textures[0].bytes instanceof Uint8Array);

const probe = Object.create(Viewer.prototype);
probe.scene = scene;
probe.animation = { clipIndex: 0, time: 0 };
assert.equal(probe.seekAnimation(0.5), true);
probe._updateWorldMatrices();
assert.ok(Math.abs(scene.nodes[0].trs.translation[0] - 1) < 1e-6);
assert.ok(Math.abs(scene.nodes[0].world[12] - 1) < 1e-6);
assert.ok(Math.abs(scene.nodes[1].world[12] - 1) < 1e-6, 'child world transform must retain the parent animation');

console.log('SceneDocument viewer parity passed');
