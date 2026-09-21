/**
 * The half of splat drawing that is not a shader.
 *
 * Packing, ordering and reordering decide what the GPU is handed; the shader
 * decides what it does with it. Only the first half runs without a browser,
 * and it is where a wrong answer is silent — a sort that is backwards draws a
 * scene inside out and still draws a scene.
 */
import assert from 'node:assert/strict';

import { packSplats, reorder, sortOrder } from '../src/viewer/splat-pass.ts';
import type { SplatCloud } from '../src/splat.ts';

const STRIDE = 14;

function cloudOf(positions: number[][]): SplatCloud {
  const count = positions.length;
  const cloud: SplatCloud = {
    count,
    positions: Float32Array.from(positions.flat()),
    scales: new Float32Array(count * 3),
    rotations: new Float32Array(count * 4),
    alphas: new Float32Array(count),
    dc: new Float32Array(count * 3),
  };
  // Values that name their own splat, so a reorder that shuffles fields rather
  // than splats is visible.
  for (let splat = 0; splat < count; splat += 1) {
    cloud.scales.set([splat + 0.1, splat + 0.2, splat + 0.3], splat * 3);
    cloud.rotations.set([1, 0, 0, 0], splat * 4);
    cloud.alphas[splat] = splat + 0.4;
    cloud.dc.set([splat + 0.5, splat + 0.6, splat + 0.7], splat * 3);
  }
  return cloud;
}

// ---------------------------------------------------------------------------
// Packing keeps each splat's fourteen numbers together and in order
// ---------------------------------------------------------------------------

{
  const cloud = cloudOf([[10, 11, 12], [20, 21, 22]]);
  const packed = packSplats(cloud);
  assert.equal(packed.length, 2 * STRIDE);
  assert.deepEqual(
    [...packed.subarray(0, STRIDE)].map((v) => Math.round(v * 10) / 10),
    [10, 11, 12, 0.1, 0.2, 0.3, 1, 0, 0, 0, 0.4, 0.5, 0.6, 0.7],
  );
  assert.equal(packed[STRIDE], 20, 'the second splat starts at the second stride');
}

// ---------------------------------------------------------------------------
// The order is furthest first along the view direction
// ---------------------------------------------------------------------------

{
  // Three splats strung along +z, handed in nearest first.
  const cloud = cloudOf([[0, 0, 1], [0, 0, 2], [0, 0, 3]]);

  // Looking along +z: the largest z is the furthest, and is drawn first so
  // that what is nearer blends over it.
  assert.deepEqual([...sortOrder(cloud, [0, 0, 1])], [2, 1, 0]);
  // Looking the other way, the order turns with it.
  assert.deepEqual([...sortOrder(cloud, [0, 0, -1])], [0, 1, 2]);
  // A direction the splats do not vary along leaves them as they came.
  assert.deepEqual([...sortOrder(cloud, [1, 0, 0])], [0, 1, 2]);
}

// The caller's buffer is filled rather than a new one allocated, which is what
// makes a re-sort per camera move affordable.
{
  const cloud = cloudOf([[0, 0, 1], [0, 0, 2]]);
  const into = new Uint32Array(2);
  const out = sortOrder(cloud, [0, 0, 1], into);
  assert.equal(out, into, 'the same array comes back');
  assert.deepEqual([...into], [1, 0]);
}

// ---------------------------------------------------------------------------
// Reordering moves whole splats
// ---------------------------------------------------------------------------

{
  const cloud = cloudOf([[10, 11, 12], [20, 21, 22], [30, 31, 32]]);
  const packed = packSplats(cloud);
  const into = new Float32Array(packed.length);

  reorder(packed, Uint32Array.from([0, 1, 2]), into);
  assert.deepEqual([...into], [...packed], 'the identity order changes nothing');

  reorder(packed, Uint32Array.from([2, 0, 1]), into);
  // Each slot holds one splat's fourteen numbers, not a mixture: the position
  // and the alpha in a slot must name the same splat.
  assert.equal(into[0], 30);
  assert.ok(Math.abs(into[10] - 2.4) < 1e-5, 'slot 0 carries splat 2 alpha');
  assert.equal(into[STRIDE], 10);
  assert.ok(Math.abs(into[STRIDE + 10] - 0.4) < 1e-5, 'slot 1 carries splat 0 alpha');
  assert.equal(into[STRIDE * 2], 20);
  assert.ok(Math.abs(into[STRIDE * 2 + 10] - 1.4) < 1e-5, 'slot 2 carries splat 1 alpha');
}

console.log('splat pass: packing, ordering and reordering hold');
