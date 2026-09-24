/**
 * The splat dialect, against the rows measured from Blender's importer.
 *
 * Each row is what Blender's `convert_gsplat_ply_to_point_cloud` produced for
 * a value picked so that it reads off a distinct number, measured against a
 * daily build. Those rows are the oracle here: if this module and them ever
 * disagree, one of them is wrong about the dialect and the disagreement is the
 * finding.
 *
 * The rows:
 *
 *   scale_* = 0.0       -> scale [1, 1, 1]          exp(0)
 *   opacity = 0.0       -> alpha 0.5                sigmoid(0)
 *   f_dc = 1, 2, 3      -> [1, 2, 3]                untouched, a raw harmonic
 *   rot = 1, 0, 0, 0    -> [1, 0, 0, 0]             (w, x, y, z), normalized
 *
 * One row reads differently here than in that table, on purpose: the cloud is
 * turned upright on the way out, because the file's frame is Y-down. So the
 * identity quaternion arrives as the half turn that does it, and positions
 * come back with y and z negated.
 */
import assert from 'node:assert/strict';

import {
  REQUIRED_SPLAT_PROPERTIES, SPLAT_BUDGET, coefficientsFor, degreeForRestCount, isSplatPly,
  readSplatCloud, selectedFromNamedAttributes, splatBitsFor, viewDirectionForSh,
} from '../src/splat.ts';
import type { SelectedProperties } from '../src/splat.ts';

/**
 * Elementwise, as numbers.
 *
 * Negating a zero gives `-0`, which is the same number as `0` in every
 * arithmetic that follows and a different value to `deepStrictEqual`. Turning
 * a cloud upright negates coordinates, so half the expected rows would fail on
 * a distinction no renderer can observe.
 */
function sameNumbers(actual: ArrayLike<number>, expected: number[], what: string) {
  assert.equal(actual.length, expected.length, `${what}: length`);
  for (let i = 0; i < expected.length; i += 1) {
    assert.ok(
      Math.abs(actual[i] - expected[i]) < 1e-6,
      `${what}[${i}]: ${actual[i]} against ${expected[i]}`,
    );
  }
}

// ---------------------------------------------------------------------------
// Recognition follows Blender's rule exactly
// ---------------------------------------------------------------------------

{
  // The eleven that decide; the harmonics are optional.
  const required = [...REQUIRED_SPLAT_PROPERTIES];
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

  // exp(0) is one metre on every axis. A turn does not stretch anything, so
  // this row reads the same whichever way up the cloud ends.
  assert.deepEqual([...cloud.scales], [1, 1, 1]);
  // sigmoid(0) is a half.
  assert.equal(cloud.alphas[0], 0.5);
  // The DC term is a raw harmonic and is not touched on the way in.
  assert.deepEqual([...cloud.dc], [1, 2, 3]);
  // Blender reads (1, 0, 0, 0) as the identity, in (w, x, y, z). What comes
  // out here is that identity turned upright, which is the half turn itself.
  sameNumbers(cloud.rotations, [0, 1, 0, 0], 'the identity, turned upright');
}

// ---------------------------------------------------------------------------
// The file is Y-down, and the cloud that comes out is not
// ---------------------------------------------------------------------------

/** A rotation matrix from a `(w, x, y, z)` quaternion, as the shader builds it. */
function matrixOf(q: readonly number[]): number[][] {
  const [w, x, y, z] = q;
  return [
    [1 - 2*(y*y + z*z), 2*(x*y - w*z), 2*(x*z + w*y)],
    [2*(x*y + w*z), 1 - 2*(x*x + z*z), 2*(y*z - w*x)],
    [2*(x*z - w*y), 2*(y*z + w*x), 1 - 2*(x*x + y*y)],
  ];
}

{
  // Positions turn by a half rotation about X.
  const cloud = readSplatCloud(selected({
    f_dc_0: [0], f_dc_1: [0], f_dc_2: [0],
    opacity: [0],
    scale_0: [0], scale_1: [0], scale_2: [0],
    rot_0: [1], rot_1: [0], rot_2: [0], rot_3: [0],
  }, 1));
  assert.ok(cloud);
  // `selected` fills positions with 0, 1, 2.
  sameNumbers(cloud.positions, [0, -1, -2], 'y and z change sign');
}

// The quaternion has to turn with the positions or a gaussian ends up oriented
// against the scene it sits in, which no single-value check would catch. So
// this asserts the invariant instead.
//
// The invariant is R' = M R, not M R Mᵀ. A gaussian's covariance is R S Sᵀ Rᵀ;
// turning the world by M makes it M R S Sᵀ Rᵀ Mᵀ = (M R) S Sᵀ (M R)ᵀ, so the
// orientation is composed with the turn and the axis lengths are untouched.
// Conjugation would be the answer to a different question -- re-expressing the
// same gaussian in rotated coordinates rather than moving it.
{
  const quaternions = [
    [1, 0, 0, 0],
    [0.5, 0.5, 0.5, 0.5],
    [0.9238795, 0.3826834, 0, 0],
    [0.7071068, 0, 0.7071068, 0],
    [0.6, -0.4, 0.5, 0.4795832],
  ];
  for (const q of quaternions) {
    const length = Math.hypot(...q);
    const unit = q.map((v) => v / length);
    const cloud = readSplatCloud(selected({
      f_dc_0: [0], f_dc_1: [0], f_dc_2: [0],
      opacity: [0],
      scale_0: [0], scale_1: [0], scale_2: [0],
      rot_0: [unit[0]], rot_1: [unit[1]], rot_2: [unit[2]], rot_3: [unit[3]],
    }, 1));
    assert.ok(cloud);

    const turned = matrixOf([...cloud.rotations]);
    const original = matrixOf(unit);
    const sign = [1, -1, -1];
    for (let row = 0; row < 3; row += 1) {
      for (let col = 0; col < 3; col += 1) {
        const expected = sign[row] * original[row][col];
        assert.ok(
          Math.abs(turned[row][col] - expected) < 1e-5,
          `q=${unit} element ${row},${col}: ${turned[row][col]} against ${expected}`,
        );
      }
    }
  }
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
  sameNumbers(cloud.rotations, [0, 1, 0, 0], 'a doubled quaternion is the same rotation');
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
  sameNumbers(cloud.rotations, [0, 1, 0, 0], 'the identity, turned upright');
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
    // The turn leaves the scales alone: a rotation stretches nothing.
    assert.ok(Math.abs(cloud.scales[splat * 3 + 2] - Math.exp(2 + splat)) < 1e-4);
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

// ---------------------------------------------------------------------------
// The harmonics: the staircase, and the transpose
// ---------------------------------------------------------------------------

// Blender floors the degree rather than salvaging the remainder, and this has
// to floor it the same way or the two disagree about what a file contains.
{
  for (const [count, degree] of [
    [0, 0], [3, 0], [4, 0], [6, 0], [9, 1], [12, 1], [15, 1],
    [24, 2], [30, 2], [45, 3], [72, 3],
  ] as const) {
    assert.equal(degreeForRestCount(count), degree, `${count} f_rest values`);
  }
  // A count that is not a multiple of three loses everything, which is the
  // importer's behaviour and not a rounding of it.
  assert.equal(degreeForRestCount(46), 0, '46 is not three channels of anything');
  assert.deepEqual([0, 1, 2, 3].map(coefficientsFor), [0, 3, 8, 15]);
}

// The file stores every coefficient of red, then of green, then of blue. The
// cloud stores one coefficient at a time with rgb inside. Getting that
// backwards does not fail, it tints — so the row from the note is asserted
// directly: f_rest .10 .11 .12 | .20 .21 .22 | .30 .31 .32 reads back as
// (.10,.20,.30), (.11,.21,.31), (.12,.22,.32).
{
  const rest: Record<string, number[]> = {};
  const values = [0.10, 0.11, 0.12, 0.20, 0.21, 0.22, 0.30, 0.31, 0.32];
  values.forEach((v, i) => { rest[`f_rest_${i}`] = [v]; });
  const cloud = readSplatCloud(selected({
    f_dc_0: [0], f_dc_1: [0], f_dc_2: [0],
    opacity: [0],
    scale_0: [0], scale_1: [0], scale_2: [0],
    rot_0: [1], rot_1: [0], rot_2: [0], rot_3: [0],
    ...rest,
  }, 1));
  assert.ok(cloud);
  assert.equal(cloud.shDegree, 1, 'nine values are degree 1');
  sameNumbers(cloud.sh, [0.10, 0.20, 0.30, 0.11, 0.21, 0.31, 0.12, 0.22, 0.32], 'transposed sh');
}

// A file with no harmonics is still a splat, and says it carries none.
{
  const cloud = readSplatCloud(selected({
    f_dc_0: [0], f_dc_1: [0], f_dc_2: [0],
    opacity: [0],
    scale_0: [0], scale_1: [0], scale_2: [0],
    rot_0: [1], rot_1: [0], rot_2: [0], rot_3: [0],
  }, 1));
  assert.ok(cloud);
  assert.equal(cloud.shDegree, 0);
  assert.equal(cloud.sh.length, 0);
}

// The harmonics stay in the file's frame while the positions are turned, so
// the direction asked of them has to be turned back. It is the same two signs.
{
  const dir = viewDirectionForSh([0, 0, 0], [0, 2, 0]);
  sameNumbers(dir, [0, -1, 0], 'y flips back');
  const away = viewDirectionForSh([1, 1, 1], [1, 1, 4]);
  sameNumbers(away, [0, 0, -1], 'z flips back');
  const unit = viewDirectionForSh([0, 0, 0], [3, 4, 0]);
  assert.ok(Math.abs(Math.hypot(...unit) - 1) < 1e-6, 'and it comes back normalized');
}

// ---------------------------------------------------------------------------
// The same dialect, arriving from a Draco payload instead of a PLY header
// ---------------------------------------------------------------------------

// Draco says nothing about what a generic attribute means, so a splat payload
// carries its column names in attribute metadata. A multi-component attribute
// has to be spread back into the PLY's numbered names, because every rule in
// the module is written against those.
{
  const extras = [
    { name: 'f_dc', components: 3, values: [0.1, 0.2, 0.3, 1.1, 1.2, 1.3] },
    { name: 'opacity', components: 1, values: [0, 0] },
    { name: 'scale', components: 3, values: [0, 0, 0, 0, 0, 0] },
    { name: 'rot', components: 4, values: [1, 0, 0, 0, 1, 0, 0, 0] },
    // Unnamed attributes are the ones Draco could not label, and they cannot
    // be guessed at: a payload with these alone is not a splat.
    { name: null, components: 1, values: [9, 9] },
  ];
  const selected = selectedFromNamedAttributes(2, [0, 0, 0, 1, 1, 1], extras);
  assert.ok(isSplatPly(Object.keys(selected.properties)), 'the names come back numbered');
  sameNumbers([...selected.properties.f_dc_1], [0.2, 1.2], 'and the components are un-interleaved');

  const cloud = readSplatCloud(selected);
  assert.ok(cloud, 'and it reads as a splat cloud');
  assert.equal(cloud.count, 2);
  // The same activations as from a PLY: sigmoid(0), exp(0).
  assert.equal(cloud.alphas[0], 0.5);
  assert.equal(cloud.scales[0], 1);
}

// A payload whose attributes carry no names is ordinary geometry, not a splat
// with missing labels.
{
  const selected = selectedFromNamedAttributes(1, [0, 0, 0], [
    { name: null, components: 3, values: [1, 2, 3] },
  ]);
  assert.equal(Object.keys(selected.properties).length, 0);
  assert.equal(readSplatCloud(selected), null);
}

// A splat PLY's properties arrive as one Float32Array each and are taken as
// they are: copying them would briefly double a payload of hundreds of
// megabytes.
{
  const opacity = new Float32Array([0, 1]);
  const selected = selectedFromNamedAttributes(2, [0, 0, 0, 1, 1, 1], [
    { name: 'opacity', components: 1, values: opacity },
  ]);
  assert.equal(selected.properties.opacity, opacity, 'the same array, not a copy');
}

// The budget a splat is written with: harmonics coarsest, colour, opacity and
// scale finest, and nothing about the name decided by accident -- `f_dc` is
// colour, not a harmonic band, however alike the names look.
{
  assert.equal(splatBitsFor('f_rest_0'), SPLAT_BUDGET.harmonics);
  assert.equal(splatBitsFor('f_rest_44'), SPLAT_BUDGET.harmonics);
  for (const name of ['f_dc_0', 'f_dc_2', 'opacity', 'scale_1']) {
    assert.equal(splatBitsFor(name), SPLAT_BUDGET.appearance, name);
  }
  for (const name of ['rot_0', 'rot_3', 'confidence']) {
    assert.equal(splatBitsFor(name), SPLAT_BUDGET.other, name);
  }
  assert.ok(SPLAT_BUDGET.harmonics < SPLAT_BUDGET.other);
  assert.ok(SPLAT_BUDGET.other < SPLAT_BUDGET.appearance);
}

console.log('splat dialect: recognition, activations and harmonics match the measured rows');
