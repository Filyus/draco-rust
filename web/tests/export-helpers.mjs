/**
 * The flat writers, exercised.
 *
 * OBJ, PLY and the flattened FBX path had no test of any kind: they were
 * reachable only through the browser, behind a DOM the node suites cannot
 * construct. Now that the route helpers take their settings as an argument
 * instead of reading checkboxes, the writers can be driven directly.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

const { modules } = await import(pathToFileURL(resolve(here, '..', 'src', 'app', 'state.ts')).href);
for (const name of ['obj', 'ply', 'fbx']) {
  const module = await import(pathToFileURL(resolve(pkg, `${name}.js`)).href);
  await module.default({ module_or_path: await readFile(resolve(pkg, `${name}_bg.wasm`)) });
  modules[name] = { loaded: true, module };
}

const {
  exportToFbx,
  exportToObj,
  exportToPly,
  prepareMeshesForExport,
} = await import(pathToFileURL(resolve(here, '..', 'src', 'app', 'export-branches.ts')).href);

const all = { includeNormals: true, includeUvs: true };
const quad = [
  {
    name: 'front',
    positions: [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0],
    indices: [0, 1, 2, 0, 2, 3],
    normals: [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1],
    uvs: [0, 0, 1, 0, 1, 1, 0, 1],
  },
  {
    name: 'back',
    positions: [0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1],
    indices: [0, 1, 2, 0, 2, 3],
    normals: [0, 0, -1, 0, 0, -1, 0, 0, -1, 0, 0, -1],
    uvs: [0, 0, 1, 0, 1, 1, 0, 1],
  },
];

const single = prepareMeshesForExport([quad[0]], all);
const both = prepareMeshesForExport(quad, all);

// OBJ takes the whole list: one mesh and several go down different wasm calls.
const obj = await exportToObj(single, all);
assert.equal(obj.success, true, `single-mesh OBJ: ${obj.error || ''}`);
assert.match(obj.data, /^v /m, 'OBJ output has no vertices');
assert.match(obj.data, /^vn /m, 'normals were requested but not written');
assert.match(obj.data, /^vt /m, 'UVs were requested but not written');

const objMulti = await exportToObj(both, all);
assert.equal(objMulti.success, true, `multi-mesh OBJ: ${objMulti.error || ''}`);
assert.equal((objMulti.data.match(/^o /gm) || []).length, 2, 'each mesh needs its own OBJ object');

// The toggles travel two ways: preparation drops the arrays, and the writer
// is told separately. Both are checked, because either one alone would make
// the other look like it works.
const bare = prepareMeshesForExport([quad[0]], { includeNormals: false, includeUvs: false });
assert.equal(bare[0].normals, null);
assert.equal(bare[0].uvs, null);
const objBare = await exportToObj(bare, { includeNormals: false, includeUvs: false });
assert.equal(objBare.success, true, `bare OBJ: ${objBare.error || ''}`);
assert.doesNotMatch(objBare.data, /^vn /m, 'normals were excluded but written anyway');
assert.doesNotMatch(objBare.data, /^vt /m, 'UVs were excluded but written anyway');

// The writer's own option, isolated: full data in, exclusion asked for.
const objOptionOnly = await exportToObj(single, { includeNormals: false, includeUvs: true });
assert.equal(objOptionOnly.success, true, `option-only OBJ: ${objOptionOnly.error || ''}`);
assert.doesNotMatch(objOptionOnly.data, /^vn /m, 'the writer ignored include_normals');
assert.match(objOptionOnly.data, /^vt /m, 'excluding normals must not drop UVs');

// PLY holds one mesh, so several arrive merged — with indices rebased.
const ply = await exportToPly(both, all);
assert.equal(ply.success, true, `PLY: ${ply.error || ''}`);
assert.match(ply.data, /element vertex 8/, 'the merged PLY must carry both quads');
assert.match(ply.data, /element face 4/);

// The flat FBX writer takes the same prepared list and no scene at all.
const fbx = await exportToFbx(both);
assert.equal(fbx.success, true, `flat FBX: ${fbx.error || ''}`);
const magic = Buffer.from(fbx.binary_data.slice(0, 21)).toString('binary');
assert.equal(magic, 'Kaydara FBX Binary  \u0000', 'flat FBX output is not an FBX');
const reparsed = modules.fbx.module.parse_fbx(new Uint8Array(fbx.binary_data));
assert.equal(reparsed.success, true, `flat FBX reparse: ${reparsed.error || ''}`);

console.log('export helper writers passed');
