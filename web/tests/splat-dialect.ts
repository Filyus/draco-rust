/**
 * The splat dialect, against the rows measured from Blender's importer.
 *
 * `dev/docs/format-research/notes/blender-gsplat.md` records what Blender's
 * `convert_gsplat_ply_to_point_cloud` produces for values picked so that each
 * one reads off a distinct number. Those rows are the oracle here: if this
 * module and that table ever disagree, one of them is wrong about the dialect
 * and the disagreement is the finding.
 *
 * The rows:
 *
 *   scale_* = 0.0       -> scale [1, 1, 1]          exp(0)
 *   opacity = 0.0       -> alpha 0.5                sigmoid(0)
 *   f_dc = 1, 2, 3      -> [1, 2, 3]                untouched, a raw harmonic
 *   rot = 1, 0, 0, 0    -> [1, 0, 0, 0]             (w, x, y, z), normalized
 */
import assert from 'node:assert/strict';

import { isSplatPly, readSplatCloud, splatPropertyNames } from '../src/splat.ts';
import type { SelectedProperties } from '../src/splat.ts';

// ---------------------------------------------------------------------------
// Recognition follows Blender's rule exactly
// ---------------------------------------------------------------------------

{
  const required = splatPropertyNames();
  assert.equal(required.length, 11);
  assert.ok(isSplatPly(required), 'the eleven on their own are a splat');
  assert.ok(
    isSplatPly([...required, 'f_rest_0', 'f_rest_1', 'x', 'y', 'z']),
    'f_rest is optional and extra properties do not disqualify',
  );

  // Any one missing and Blender imports ordinary geometry, so this must too.
  for (const name of required) {
    const short = required.filter((other) => other !== name);
    assert.ok(!isSplatPly(short), `without ${name} it must not read as a splat`);
  }
  assert.ok(!isSplatPly(['x', 'y', 'z', 'red', 'green', 'blue']), 'a plain cloud is not a splat');
}

// ---------------------------------------------------------------------------
// The activations are the dialect's, and the measured rows say which
// ---------------------------------------------------------------------------

function selected(values: Record<string, number[]>, count: number): SelectedProperties {
  const properties: Record<string, Float32Array> = {};
  for (const [name, plane] of Object.entries(values)) {
    properties[name] = Float32Array.from(plane);
  }
  return {
    success: true,
    count,
    positions: Float32Array.from({ length: count * 3 }, (_, i) => i),
    properties,
  };
}

{
  const one = selected({
    f_dc_0: [1], f_dc_1: [2], f_dc_2: [3],
    opacity: [0],
    scale_0: [0], scale_1: [0], scale_2: [0],
    rot_0: [1], rot_1: [0], rot_2: [0], rot_3: [0],
  }, 1);

  const cloud = readSplatCloud(one);
  assert.ok(cloud, 'the eleven properties read as a splat');
  assert.equal(cloud.count, 1);

  // exp(0) is one metre on every axis.
  assert.deepEqual([...cloud.scales], [1, 1, 1]);
  // sigmoid(0) is a half.
  assert.equal(cloud.alphas[0], 0.5);
  // The DC term is a raw harmonic and is not touched on the way in.
  assert.deepEqual([...cloud.dc], [1, 2, 3]);
  // (w, x, y, z), which is the file's order and not `xyzw`.
  assert.deepEqual([...cloud.rotations], [1, 0, 0, 0]);
}

// A quaternion arrives normalized, as Blender normalizes on import.
{
  const cloud = readSplatCloud(selected({
    f_dc_0: [0], f_dc_1: [0], f_dc_2: [0],
    opacity: [0],
    scale_0: [0], scale_1: [0], scale_2: [0],
    rot_0: [2], rot_1: [0], rot_2: [0], rot_3: [0],
  }, 1));
  assert.ok(cloud);
  assert.deepEqual([...cloud.rotations], [1, 0, 0, 0], 'a doubled quaternion is the same rotation');
}

// A zero quaternion names no rotation. The identity is a choice, and the point
// of making it here is that the alternative is a NaN that takes the splat off
// screen without saying why.
{
  const cloud = readSplatCloud(selected({
    f_dc_0: [0], f_dc_1: [0], f_dc_2: [0],
    opacity: [0],
    scale_0: [0], scale_1: [0], scale_2: [0],
    rot_0: [0], rot_1: [0], rot_2: [0], rot_3: [0],
  }, 1));
  assert.ok(cloud);
  assert.ok(cloud.rotations.every(Number.isFinite), 'no NaN reaches the renderer');
  assert.deepEqual([...cloud.rotations], [1, 0, 0, 0]);
}

// The planes are not confused with one another: three splats, each property
// carrying a value that names itself.
{
  const count = 3;
  const plane = (base: number) => Array.from({ length: count }, (_, i) => base + i);
  const cloud = readSplatCloud(selected({
    f_dc_0: plane(10), f_dc_1: plane(20), f_dc_2: plane(30),
    opacity: plane(0),
    scale_0: plane(0), scale_1: plane(1), scale_2: plane(2),
    rot_0: plane(1), rot_1: plane(0), rot_2: plane(0), rot_3: plane(0),
  }, count));
  assert.ok(cloud);
  for (let splat = 0; splat < count; splat += 1) {
    assert.equal(cloud.dc[splat * 3], 10 + splat);
    assert.equal(cloud.dc[splat * 3 + 1], 20 + splat);
    assert.equal(cloud.dc[splat * 3 + 2], 30 + splat);
    assert.ok(Math.abs(cloud.scales[splat * 3] - Math.exp(splat)) < 1e-4);
    assert.ok(Math.abs(cloud.scales[splat * 3 + 1] - Math.exp(1 + splat)) < 1e-4);
  }
}

// Something that is not a splat is not forced into being one.
{
  assert.equal(
    readSplatCloud(selected({ x: [0], y: [0], z: [0] }, 1)),
    null,
    'a cloud with no splat properties reads as nothing',
  );
  assert.equal(
    readSplatCloud({ success: false, error: 'no', count: 0, positions: new Float32Array(), properties: {} }),
    null,
    'a failed read is not a splat either',
  );
}

console.log('splat dialect: recognition and activations match the measured rows');
