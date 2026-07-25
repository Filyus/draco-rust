import assert from 'node:assert/strict';
import { buildFbxSceneFromDocument } from '../www/fbx-scene-document-writer.js';
import { lowerSceneDocumentToGltf } from '../www/scene-document-gltf.js';
import { buildViewerSceneFromDocument } from '../www/scene-document-viewer.js';
import { assertValidSceneDocument, createSceneDocument } from '../www/scene-document.js';

function bytes(values) {
    return new Uint8Array(values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength));
}

const positions = bytes(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]));
const indices = bytes(new Uint16Array([0, 1, 2]));
const uv0 = bytes(new Float32Array([0, 0, 1, 0, 0, 1]));
const uv1 = bytes(new Float32Array([0, 1, 1, 1, 0, 0]));
const colors = bytes(new Float32Array([1, 0, 0, 1, 0, 1, 0, 1, 0, 0, 1, 1]));
const tangents = bytes(new Float32Array([1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1]));
const joints0 = bytes(new Uint16Array([0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3]));
const joints1 = bytes(new Uint16Array([4, 5, 6, 7, 4, 5, 6, 7, 4, 5, 6, 7]));
const weights0 = bytes(new Float32Array(12).fill(0.125));
const weights1 = bytes(new Float32Array(12).fill(0.125));
const inverseBinds = bytes(new Float32Array(Array.from({ length: 8 }, (_, index) => {
    const matrix = new Array(16).fill(0);
    matrix[0] = matrix[5] = matrix[10] = matrix[15] = 1;
    return matrix;
}).flat()));

const document = createSceneDocument({
    accessors: [
        { bytes: positions, componentType: 5126, components: 3, count: 3 },
        { bytes: indices, componentType: 5123, components: 1, count: 3 },
        { bytes: uv0, componentType: 5126, components: 2, count: 3 },
        { bytes: uv1, componentType: 5126, components: 2, count: 3 },
        { bytes: colors, componentType: 5126, components: 4, count: 3 },
        { bytes: tangents, componentType: 5126, components: 4, count: 3 },
        { bytes: joints0, componentType: 5123, components: 4, count: 3 },
        { bytes: weights0, componentType: 5126, components: 4, count: 3 },
        { bytes: joints1, componentType: 5123, components: 4, count: 3 },
        { bytes: weights1, componentType: 5126, components: 4, count: 3 },
        { bytes: inverseBinds, componentType: 5126, components: 16, count: 8 },
    ],
    meshes: [{ name: 'Attributes', primitives: [{
        attributes: {
            POSITION: 0, TEXCOORD_0: 2, TEXCOORD_1: 3, COLOR_0: 4, TANGENT: 5,
            JOINTS_0: 6, WEIGHTS_0: 7, JOINTS_1: 8, WEIGHTS_1: 9,
        }, indices: 1,
    }] }],
    nodes: [{ name: 'Mesh', mesh: 0, skin: 0, children: [1, 2, 3, 4, 5, 6, 7, 8] },
        ...Array.from({ length: 8 }, (_, index) => ({ name: `Joint_${index}`, translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1] }))],
    rootNodes: [0],
    skins: [{ name: 'EightInfluences', joints: [1, 2, 3, 4, 5, 6, 7, 8], inverseBindMatrices: 10 }],
});

assertValidSceneDocument(document);
const lowered = lowerSceneDocumentToGltf(document);
assert.deepEqual(Object.keys(lowered.json ? JSON.parse(new TextDecoder().decode(lowered.json)).meshes[0].primitives[0].attributes : {}).sort(),
    ['COLOR_0', 'JOINTS_0', 'JOINTS_1', 'POSITION', 'TANGENT', 'TEXCOORD_0', 'TEXCOORD_1', 'WEIGHTS_0', 'WEIGHTS_1']);

const viewerScene = buildViewerSceneFromDocument(document);
assert.equal(viewerScene.meshes[0].primitives[0].attributes.TEXCOORD_1.components, 2);
assert.match(viewerScene.warnings.join('\n'), /first four influences/);

const fbxScene = buildFbxSceneFromDocument(document);
const weightsPreserved = fbxScene.rootNodes[0].meshes[0].skin.clusters.reduce((total, cluster) => total + cluster.weights.length, 0);
assert.equal(weightsPreserved, 24);
assert.match(fbxScene.warnings.join('\n'), /does not yet emit this layer/);

console.log('SceneDocument portable attributes and eight-influence preservation passed');
