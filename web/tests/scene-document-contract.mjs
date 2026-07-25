import assert from 'node:assert/strict';
import {
    SCENE_DOCUMENT_VERSION,
    assertValidSceneDocument,
    cloneSceneDocument,
    createSceneDocument,
    sceneDocumentTransferables,
    validateSceneDocument,
} from '../www/scene-document.js';

function bytes(values) {
    return new Uint8Array(values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength));
}

const positions = bytes(new Float32Array([
    0, 0, 0,
    1, 0, 0,
    0, 1, 0,
]));
const indices = bytes(new Uint16Array([0, 1, 2]));
const times = bytes(new Float32Array([0, 1]));
const translations = bytes(new Float32Array([0, 0, 0, 1, 0, 0]));
const inverseBind = bytes(new Float32Array([
    1, 0, 0, 0,
    0, 1, 0, 0,
    0, 0, 1, 0,
    0, 0, 0, 1,
]));
const morph = bytes(new Float32Array([
    0, 0, 0,
    0, 0.25, 0,
    0, 0, 0,
]));

const document = createSceneDocument({
    resources: [{ name: 'albedo.png', mimeType: 'image/png', bytes: new Uint8Array([137, 80, 78, 71]) }],
    textures: [{ name: 'albedo', resource: 0, sampler: { wrapS: 10497, wrapT: 10497 } }],
    materials: [{
        name: 'paint',
        baseColorFactor: [1, 0.5, 0.25, 1],
        metallicFactor: 0,
        roughnessFactor: 0.75,
        emissiveFactor: [0, 0, 0],
        baseColorTexture: { texture: 0, texCoord: 0 },
    }],
    accessors: [
        { bytes: positions, componentType: 5126, components: 3, count: 3 },
        { bytes: indices, componentType: 5123, components: 1, count: 3 },
        { bytes: times, componentType: 5126, components: 1, count: 2 },
        { bytes: translations, componentType: 5126, components: 3, count: 2 },
        { bytes: morph, componentType: 5126, components: 3, count: 3 },
        { bytes: inverseBind, componentType: 5126, components: 16, count: 1 },
    ],
    meshes: [{
        name: 'Triangle',
        weights: [0],
        primitives: [{
            attributes: { POSITION: 0 },
            indices: 1,
            material: 0,
            targets: [{ POSITION: 4 }],
        }],
    }],
    nodes: [{
        name: 'Mesh',
        translation: [0, 0, 0],
        rotation: [0, 0, 0, 1],
        scale: [1, 1, 1],
        mesh: 0,
        skin: 0,
        weights: [0],
        children: [1],
    }, {
        name: 'Joint',
        translation: [0, 0, 0],
        rotation: [0, 0, 0, 1],
        scale: [1, 1, 1],
    }],
    rootNodes: [0],
    skins: [{ name: 'Skin', joints: [1], inverseBindMatrices: 5, skeleton: 1 }],
    animations: [{
        name: 'Move',
        duration: 1,
        samplers: [{ input: 2, output: 3, interpolation: 'LINEAR' }],
        channels: [{ sampler: 0, node: 0, path: 'translation' }],
    }],
});

assert.equal(document.version, SCENE_DOCUMENT_VERSION);
const result = assertValidSceneDocument(document);
assert.deepEqual(result.errors, []);
assert.equal(result.capabilities.resources, true);
assert.equal(result.capabilities.skins, true);
assert.equal(result.capabilities.morphTargets, true);
assert.equal(result.capabilities.animations, true);
assert.equal(result.capabilities.maxSkinJoints, 1);
assert.equal(result.capabilities.maxMorphTargets, 1);

const cloned = cloneSceneDocument(document);
assert.notEqual(cloned.accessors[0].bytes, document.accessors[0].bytes);
assert.deepEqual(cloned.accessors[0].bytes, document.accessors[0].bytes);
assert.equal(sceneDocumentTransferables(document).length, document.accessors.length + document.resources.length);

const matrixDocument = cloneSceneDocument(document);
delete matrixDocument.nodes[1].translation;
delete matrixDocument.nodes[1].rotation;
delete matrixDocument.nodes[1].scale;
matrixDocument.nodes[1].matrix = [
    1, 0, 0, 0,
    0, 1, 0, 0,
    0, 0, 1, 0,
    0, 0, 0, 1,
];
const matrixResult = validateSceneDocument(matrixDocument);
assert.equal(matrixResult.ok, true);
assert.equal(matrixResult.capabilities.matrixNodes, true);
assert.match(matrixResult.warnings.join('\n'), /matrix local transform/);

const invalid = cloneSceneDocument(document);
invalid.accessors[0].bytes = new Uint8Array(1);
invalid.nodes[1].children = [0];
const invalidResult = validateSceneDocument(invalid);
assert.equal(invalidResult.ok, false);
assert.match(invalidResult.errors.join('\n'), /bytes length/);
assert.match(invalidResult.errors.join('\n'), /hierarchy cycle/);

console.log('SceneDocument contract validation passed');
