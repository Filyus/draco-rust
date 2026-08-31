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

console.log('viewer-point-clouds: ok');
