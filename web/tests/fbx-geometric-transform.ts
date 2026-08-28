/**
 * FBX `Geometric*`: the offset from a Model to its own geometry.
 *
 * FBX applies it to the attached mesh and does not pass it to child nodes, so
 * it cannot be folded into the node transform. Exporters write it for every
 * object whose pivot sits off its mesh origin, and a reader that drops it puts
 * those meshes at the pivot instead of where they were modelled.
 */

import assert from 'node:assert/strict';

import { buildSceneDocumentFromFbx } from '../src/fbx-scene-document.ts';
import { loadFbxViewerAdapter, loadWasm } from './fbx-test-utils.ts';

const { buildSceneFromFbx } = await loadFbxViewerAdapter();
const fbx = await loadWasm('fbx');

const OFFSET = [10, 20, -30];
const CHILD_TRANSLATION = [1, 2, 3];

const scene = {
  rootNodes: [{
    id: 1,
    name: 'Parent',
    matrix: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
    meshes: [{
      name: 'Offset',
      positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
      indices: [0, 1, 2],
      geometricTransform: { translation: OFFSET },
    }],
    children: [{
      id: 2,
      name: 'Child',
      matrix: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, ...CHILD_TRANSLATION, 1],
      meshes: [],
      children: [],
    }],
  }],
  materials: [],
  textures: [],
  animations: [],
};

const written = fbx.create_fbx_scene(scene, { version: 7500 });
assert.ok(written.success, `write failed: ${written.error}`);
const parsed = fbx.parse_fbx(new Uint8Array(written.binary_data));
assert.ok(parsed.success, `parse failed: ${parsed.error}`);

// 1. The offset survives the round trip as authored components plus the
//    composed matrix a consumer places the mesh with.
const mesh = parsed.scene.rootNodes[0].meshes[0];
assert.deepEqual(Array.from(mesh.geometricTransform.translation), OFFSET, 'authored translation');
assert.equal(mesh.geometricTransform.matrix.length, 16, 'composed matrix');
assert.deepEqual(
  Array.from(mesh.geometricTransform.matrix).slice(12, 15),
  OFFSET,
  'composed matrix translation',
);

// 2. The child node must not move with it: that is the whole reason the offset
//    cannot live on the node.
const child = parsed.scene.rootNodes[0].children[0];
assert.deepEqual(Array.from(child.matrix).slice(12, 15), CHILD_TRANSLATION, 'child stays put');

// 3. The preview places the mesh at `node.world * geometric`, and says so per
//    renderable rather than by moving the node.
const viewer = await buildSceneFromFbx(parsed);
const renderable = viewer.renderables[0];
assert.ok(renderable.geometricMatrix, 'renderable carries the geometric matrix');
assert.deepEqual(Array.from(renderable.geometricMatrix).slice(12, 15), OFFSET, 'renderable offset');
assert.deepEqual(
  Array.from(renderable.node.localMatrix || []).slice(12, 15),
  [0, 0, 0],
  'the node itself is unmoved',
);

// 4. A glTF-shaped document has no geometric transform, so the mesh gets its
//    own child node carrying the offset -- in document units and axes.
const document = buildSceneDocumentFromFbx(parsed);
const owner = document.nodes[0];
assert.equal(owner.mesh, undefined, 'the offset mesh does not hang off the node itself');
const meshNodes = (owner.children || []).map((index: number) => document.nodes[index]);
const carrier = meshNodes.find((node: any) => Number.isInteger(node.mesh));
assert.ok(carrier?.matrix, 'the mesh sits on a child node with a matrix');
const translation = carrier.matrix.slice(12, 15).map((value: number) => Math.round(value * 1000) / 1000);
assert.notDeepEqual(translation, [0, 0, 0], 'the child node carries the offset');

console.log('FBX geometric transform ok:', translation.join(', '));
