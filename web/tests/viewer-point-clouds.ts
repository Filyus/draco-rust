/**
 * Point clouds reach the viewer and render as points.
 *
 * Draco encodes geometry without connectivity on a path of its own -- a
 * sequential or KD-tree attribute coder rather than the edgebreaker -- and the
 * testdata tree carries payloads of both kinds. Three things stand between such
 * a file and a visible cloud: the web DRC module has to decode the bitstream at
 * all, the flat scene builder has to present index-less geometry as mode 0
 * instead of forcing triangles on it, and the surface shader has to state a
 * point size, without which OpenGL ES leaves the value undefined and a point
 * draw is free to produce nothing.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '..', '..');

async function loadWasm(name: string) {
  const module = await import(pathToFileURL(resolve(here, '..', 'www', 'pkg', `${name}.js`)).href);
  await module.default({
    module_or_path: await readFile(resolve(here, '..', 'www', 'pkg', `${name}_bg.wasm`)),
  });
  return module;
}

// ---------------------------------------------------------------------------
// The web DRC module decodes point-cloud bitstreams
// ---------------------------------------------------------------------------

// Both encoder paths -- sequential and KD-tree -- across the bitstream
// generations the corpus carries. Before the point-cloud decoder was built
// into the module, every one of these returned an error instead of a mesh.
{
  const drc = await loadWasm('drc');
  const payloads = [
    'legacy_draco/point_cloud_pos_norm.seq.1.0.0.drc',
    'legacy_draco/point_cloud_pos_norm.seq.1.1.0.drc',
    'legacy_draco/point_cloud_pos_norm.kd.1.3.0.drc',
  ];
  for (const name of payloads) {
    const bytes = new Uint8Array(await readFile(resolve(repo, 'testdata', name)));
    const result = drc.parse_drc_bytes(bytes);
    assert.equal(result.success, true, `${name} did not decode: ${result.error}`);
    const mesh = result.meshes[0];
    assert.equal(mesh.positions.length / 3, 4, `${name}: unexpected point count`);
    assert.equal(mesh.indices.length, 0, `${name}: a cloud carries no indices`);
    assert.equal(mesh.normals.length / 3, 4, `${name}: the normals did not survive`);
  }
}

// ---------------------------------------------------------------------------
// The flat scene builder presents index-less meshes as points, not triangles
// ---------------------------------------------------------------------------

// Every flat reader states its index stream unconditionally, so an empty one
// is a file with no faces -- what a PLY vertex list or a point-cloud .drc
// decodes to. Forcing mode 4 on it asked the GPU to draw triangles from a
// stream that never arrives in threes, which shows nothing.
{
  const { buildSceneFromMeshes } = await import(
    pathToFileURL(resolve(here, '..', 'src', 'mesh-loader.ts')).href
  );

  const cloud = await buildSceneFromMeshes({
    meshes: [{
      name: 'cloud',
      positions: new Float32Array([0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 1]),
      indices: [],
    }],
  });
  const cloudPrimitive = cloud.meshes[0].primitives[0];
  assert.equal(cloudPrimitive.mode, 0, 'an index-less mesh must draw as points');
  assert.equal(cloudPrimitive.indices, undefined, 'a cloud carries no index buffer');

  const mesh = await buildSceneFromMeshes({
    meshes: [{
      name: 'mesh',
      positions: new Float32Array([0, 0, 0, 1, 0, 0, 1, 1, 0]),
      indices: [0, 1, 2],
    }],
  });
  const meshPrimitive = mesh.meshes[0].primitives[0];
  assert.equal(meshPrimitive.mode, 4, 'an indexed mesh keeps drawing as triangles');
  assert.equal(meshPrimitive.indices.count, 3);
}

console.log('viewer-point-clouds: ok');
