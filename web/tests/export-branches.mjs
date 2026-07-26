/**
 * What an export route costs has to reach the caller.
 *
 * Five of the six routes computed warnings and threw them away — most
 * structurally, in `prepareFbxSceneForExport`, which rebuilds the scene object
 * from named fields and simply did not name `warnings`. The user converting a
 * lit, textured glTF to FBX was told nothing.
 *
 * These gates are on the pure route helpers rather than on `exportFile`, which
 * needs a browser; the Playwright suite covers the wiring end to end.
 */
import assert from 'node:assert/strict';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const {
  mergeMeshes,
  prepareFbxAnimationForExport,
  prepareFbxSceneForExport,
  prepareMeshesForExport,
} = await import(pathToFileURL(resolve(here, '..', 'src', 'app', 'export-branches.ts')).href);

const settings = { includeNormals: true, includeUvs: true };

// The regression this file exists for.
const prepared = prepareFbxSceneForExport({
  rootNodes: [],
  materials: [],
  textures: [],
  animations: [],
  warnings: ['Skin 0 has unsupported inverse bind matrices'],
}, settings);
assert.deepEqual(
  prepared.warnings,
  ['Skin 0 has unsupported inverse bind matrices'],
  'preparing a scene for FBX export must not drop what the builder reported',
);
assert.deepEqual(prepareFbxSceneForExport({}, settings).warnings, [], 'a scene without warnings yields an empty list');

// GlobalSettings carry the source unit scale; rebuilding the object dropped
// those too, which silently rescaled a re-exported FBX.
assert.deepEqual(
  prepareFbxSceneForExport({ globalSettings: { unitScaleFactor: 2.54 } }, settings).globalSettings,
  { unitScaleFactor: 2.54 },
);

// Preparation is driven by explicit settings now, not by reading checkboxes.
const mesh = {
  name: 'quad',
  positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
  indices: [0, 1, 2],
  normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
  uvs: [0, 0, 1, 0, 0, 1],
  uvSets: [{ name: 'UV1', mapping: 'byPolygonVertex', reference: 'direct', values: [0, 0], indices: [] }],
};
const [withAll] = prepareMeshesForExport([mesh], { includeNormals: true, includeUvs: true });
assert.equal(withAll.normals.length, 9);
assert.equal(withAll.uvs.length, 6);
assert.equal(withAll.uvSets[0].name, 'UV1');
const [without] = prepareMeshesForExport([mesh], { includeNormals: false, includeUvs: false });
assert.equal(without.normals, null);
assert.equal(without.uvs, null);
assert.equal(without.uvSets.length, 1, 'extra layer sets are not what the normals/UV toggles control');

// Legacy FBX importers mishandle cubic tangents; the flag has to flatten them.
const clip = {
  name: 'Take 001',
  duration: 1,
  channels: [{
    nodeName: 'Cube',
    nodeId: 1,
    path: 'translation',
    sampler: { input: [0, 1], output: [0, 0, 0, 1, 1, 1], interpolation: 'cubic', inTangents: [1], outTangents: [1] },
  }],
};
assert.equal(prepareFbxAnimationForExport(clip, false).channels[0].sampler.interpolation, 'cubic');
const legacy = prepareFbxAnimationForExport(clip, true).channels[0].sampler;
assert.equal(legacy.interpolation, 'linear');
assert.equal(legacy.inTangents, null);
assert.equal(legacy.outTangents, null);

// Morph weight channels address a target; losing that index collapses every
// shape onto the first one.
const morphClip = prepareFbxAnimationForExport({
  name: 'Morph',
  duration: 1,
  channels: [
    { nodeName: 'Cube', nodeId: 1, path: 'morphweight', morphTargetIndex: 1, sampler: { input: [0], output: [0] } },
  ],
}, false);
assert.equal(morphClip.channels[0].morphTargetIndex, 1);

// Merging rebases indices; PLY is the only consumer and it takes one mesh.
const merged = mergeMeshes([
  { positions: [0, 0, 0, 1, 0, 0, 0, 1, 0], indices: [0, 1, 2] },
  { positions: [2, 0, 0, 3, 0, 0, 2, 1, 0], indices: [0, 1, 2] },
]);
assert.deepEqual(merged.indices, [0, 1, 2, 3, 4, 5]);
assert.equal(merged.positions.length, 18);

// The element-at-a-time append in mergeMeshes is load-bearing: spreading a
// buffer this size into push() overflows the call stack.
const big = { positions: new Array(300000).fill(1), indices: [0, 1, 2] };
assert.equal(mergeMeshes([big, big]).positions.length, 600000);

console.log('export branch helpers passed');
