import assert from 'node:assert/strict';
import { createSceneDocument } from '../src/scene-document.ts';
import { buildViewerSceneFromDocument } from '../src/scene-document-viewer.ts';

globalThis.WebGL2RenderingContext = class {};
const { Viewer } = await import('../src/viewer.ts');

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

// What the portable form cost the asset and what the renderer cannot show are
// different questions. The scene report presents the first under its own
// warning source; a scene handed to the viewer carries only the second, so
// nobody looking at a frame is told something was "omitted from SceneDocument".
const documented = createSceneDocument({
    warnings: ['Unsupported glTF extensions omitted from SceneDocument: KHR_materials_sheen'],
    accessors: [
        { bytes: bytes(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0])), componentType: 5126, components: 3, count: 3 },
    ],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }],
    nodes: [{ name: 'Mesh', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0 }],
    rootNodes: [0],
});
assert.deepEqual(
    buildViewerSceneFromDocument(documented).warnings,
    [],
    'the preview scene must not repeat what the document says about the portable subset',
);
assert.deepEqual(
    documented.warnings,
    ['Unsupported glTF extensions omitted from SceneDocument: KHR_materials_sheen'],
    'and the document must keep saying it, since the scene report is what shows it',
);

// A node no scene reaches draws nothing in glTF, so it must not become a
// renderable here and must not widen the frame either. The document contract
// permits it — only a root is required to have no parent — and the glTF loader
// has always walked from the roots, so this is the rule the two paths share.
const orphaned = createSceneDocument({
    accessors: [
        { bytes: bytes(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0])), componentType: 5126, components: 3, count: 3 },
        { bytes: bytes(new Float32Array([0, 0, 0, 50, 0, 0, 0, 50, 0])), componentType: 5126, components: 3, count: 3 },
    ],
    meshes: [
        { primitives: [{ attributes: { POSITION: 0 } }] },
        { primitives: [{ attributes: { POSITION: 1 } }] },
    ],
    nodes: [
        { name: 'Drawn', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0 },
        { name: 'Stranded', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 1 },
    ],
    rootNodes: [0],
});

const orphanedScene = buildViewerSceneFromDocument(orphaned);
assert.equal(orphanedScene.nodes.length, 2, 'an unreachable node still belongs to the scene it came from');
assert.deepEqual(
    orphanedScene.renderables.map((renderable) => renderable.node.name),
    ['Drawn'],
    'a node no scene reaches must not be drawn',
);
assert.deepEqual(
    orphanedScene.aabb.max,
    [1, 1, 0],
    'the frame must come from what is drawn, not from every mesh in the document',
);

// KHR_mesh_quantization stores POSITION as a normalized integer as readily as
// a float, and the GPU reads it through the accessor's own `normalized` flag.
// Measured raw, ShaderBall.glb spans 32767 units instead of two, and the
// camera dutifully frames that box — the model draws at its true size, a speck
// at the origin of an apparently empty viewport.
const quantized = createSceneDocument({
    accessors: [
        {
            bytes: bytes(new Int16Array([0, 0, 0, 32767, 0, 0, 0, 16384, 0])),
            componentType: 5122,
            components: 3,
            count: 3,
            normalized: true,
        },
    ],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }],
    nodes: [{ name: 'Quantized', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0 }],
    rootNodes: [0],
});

const quantizedScene = buildViewerSceneFromDocument(quantized);
assert.deepEqual(
    quantizedScene.aabb.max.map((value) => Math.round(value * 1000) / 1000),
    [1, 0.5, 0],
    'a normalized POSITION must be measured as the unit fraction the GPU draws',
);

console.log('SceneDocument viewer parity passed');
