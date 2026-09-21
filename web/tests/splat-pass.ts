/**
 * The half of splat drawing that is not a shader.
 *
 * Packing and ordering decide what the GPU is handed; the shader decides what
 * it does with it. Only the first half runs without a browser, and it is where
 * a wrong answer is silent — a sort that is backwards draws a scene inside out
 * and still draws a scene.
 */
import assert from 'node:assert/strict';

import { makeSortScratch, packSplats, sortOrder } from '../src/viewer/splat-pass.ts';
import type { SplatCloud } from '../src/splat.ts';

const STRIDE = 16;

function cloudOf(positions: number[][]): SplatCloud {
  const count = positions.length;
  const cloud: SplatCloud = {
    count,
    positions: Float32Array.from(positions.flat()),
    scales: new Float32Array(count * 3),
    rotations: new Float32Array(count * 4),
    alphas: new Float32Array(count),
    dc: new Float32Array(count * 3),
    // The harmonics are the shader's business; nothing packed or sorted here
    // touches them, so these arms carry none.
    sh: new Float32Array(0),
    shDegree: 0,
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
// Packing lays each splat out as the four texels the shader fetches
// ---------------------------------------------------------------------------

{
  const cloud = cloudOf([[10, 11, 12], [20, 21, 22]]);
  const packed = packSplats(cloud);
  assert.equal(packed.length, 2 * STRIDE);
  // Four RGBA texels: centre and alpha, scale, rotation, colour. The two spare
  // floats are padding and must stay zero, or the shader reads them as data.
  assert.deepEqual(
    [...packed.subarray(0, STRIDE)].map((v) => Math.round(v * 10) / 10),
    [10, 11, 12, 0.4, 0.1, 0.2, 0.3, 0, 1, 0, 0, 0, 0.5, 0.6, 0.7, 0],
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

// The scratch is reused rather than reallocated, which is what makes a re-sort
// per camera move affordable.
{
  const cloud = cloudOf([[0, 0, 1], [0, 0, 2]]);
  const scratch = makeSortScratch(2);
  const out = sortOrder(cloud, [0, 0, 1], scratch);
  assert.equal(out, scratch.order, 'the scratch buffer is what comes back');
  assert.deepEqual([...out], [1, 0]);
}

// A counting sort has one failure a comparator sort cannot have: every splat
// at the same depth divides by a range of zero.
{
  const cloud = cloudOf([[0, 0, 5], [0, 0, 5], [0, 0, 5]]);
  const out = sortOrder(cloud, [0, 0, 1]);
  assert.equal(out.length, 3);
  assert.deepEqual([...out].sort((a, b) => a - b), [0, 1, 2], 'every splat is placed once');
}

// And a real scene's worth. A counting sort is exact only to the width of a
// bucket -- two splats inside one keep the order they arrived in -- so that
// width is the tolerance, and asserting anything tighter would be asserting
// something this sort does not promise. Sixteen bits over a 200-unit scene is
// 3 mm, far under what a splat is wide.
{
  const count = 5000;
  const spread = 100;
  const positions = Array.from({ length: count }, (_, i) => [0, 0, Math.sin(i) * spread]);
  const cloud = cloudOf(positions);
  const bucket = (2 * spread) / ((1 << 16) - 1);
  const out = sortOrder(cloud, [0, 0, 1]);
  assert.deepEqual([...out].sort((a, b) => a - b), [...Array(count).keys()], 'a permutation');
  for (let i = 1; i < count; i += 1) {
    const before = cloud.positions[out[i - 1] * 3 + 2];
    const after = cloud.positions[out[i] * 3 + 2];
    assert.ok(before >= after - bucket, `splat ${i} is out of order by more than a bucket`);
  }
}

console.log('splat pass: packing and the counting sort hold');
