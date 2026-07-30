/**
 * An FBX states which way is up, and the importer has to believe it.
 *
 * Six `GlobalSettings` fields say which axis is up, which points front and which
 * points right, each with a sign; glTF fixes all three. The importer ignored
 * them, which is correct for exactly the files that happen to be Y-up. Every
 * real-world fixture in this repository is one, so nothing caught it — and this
 * workspace's own writer is not, so a round trip came back rotated.
 *
 * Built by hand rather than from a fixture: the point is a file whose axes are
 * not glTF's, and the corpus has none. The Z-up system used here is the one the
 * FBX writer declares, so this is the round trip it takes part in.
 */
import assert from 'node:assert/strict';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import type { buildSceneDocumentFromFbx as BuildSceneDocumentFromFbx } from '../src/fbx-scene-document.ts';

const here = dirname(fileURLToPath(import.meta.url));
const { buildSceneDocumentFromFbx } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'fbx-scene-document.ts')).href
) as { buildSceneDocumentFromFbx: typeof BuildSceneDocumentFromFbx };

/**
 * The system the FBX writer declares: up along −Z, front along +Y, metres.
 *
 * Which is what its geometry change actually produces — glTF (x, y, z) written
 * as (x, z, −y) puts glTF's +Y up on FBX −Z. The two signs used to say the
 * opposite, so the file described an orientation it did not contain.
 */
const Z_UP = {
  upAxis: 2,
  upAxisSign: -1,
  frontAxis: 1,
  frontAxisSign: 1,
  coordAxis: 0,
  coordAxisSign: 1,
  unitScaleFactor: 100,
};

/** What Mixamo, Blender and the bunny all state: +Y up, +Z front, centimetres. */
const Y_UP = {
  upAxis: 1,
  upAxisSign: 1,
  frontAxis: 2,
  frontAxisSign: 1,
  coordAxis: 0,
  coordAxisSign: 1,
  unitScaleFactor: 1,
};

/**
 * One node, one triangle and one scale curve, in whatever system is given.
 *
 * The translation sits on the FBX up axis and the geometry uses three distinct
 * coordinates, so a mapping that swaps the wrong pair or drops a sign cannot
 * come out looking right. The scale curve is there because per-axis scale
 * permutes rather than rotates, which is a separate rule from the rest.
 */
function scene(globalSettings: Record<string, number>, translation: number[]) {
  return {
    warnings: [],
    scene: {
      globalSettings,
      rootNodes: [{
        id: 1,
        name: 'Object',
        matrix: [
          1, 0, 0, 0,
          0, 1, 0, 0,
          0, 0, 1, 0,
          ...translation, 1,
        ],
        meshes: [{
          name: 'Geometry',
          positions: [1, 2, 3, 0, 0, 0, 1, 0, 0],
          normals: [1, 2, 3, 0, 0, 1, 0, 0, 1],
          indices: [0, 1, 2],
        }],
        children: [],
      }],
      animations: [{
        name: 'Spin',
        duration: 1,
        channels: [{
          nodeId: 1,
          nodeName: 'Object',
          path: 'scale',
          sampler: { input: [0, 1], output: [2, 3, 4, 2, 3, 4], interpolation: 'linear' },
        }],
      }],
    },
  };
}

function positions(document: ReturnType<typeof buildSceneDocumentFromFbx>, semantic: string) {
  const index = document.meshes[0].primitives[0].attributes[semantic];
  const accessor = document.accessors[index];
  return Array.from(new Float32Array(
    accessor.bytes.buffer,
    accessor.bytes.byteOffset,
    accessor.bytes.byteLength / 4,
  )).map((value) => Math.round(value * 1e6) / 1e6);
}

function scaleCurve(document: ReturnType<typeof buildSceneDocumentFromFbx>) {
  const clip = document.animations[0];
  const channel = clip.channels.find((entry) => entry.path === 'scale')!;
  const accessor = document.accessors[clip.samplers[channel.sampler].output];
  return Array.from(new Float32Array(
    accessor.bytes.buffer,
    accessor.bytes.byteOffset,
    accessor.bytes.byteLength / 4,
  )).slice(0, 3);
}

// The writer's own output, read back: FBX (x, y, z) has to arrive as glTF
// (x, -z, y), which is the exact inverse of the change the writer applied.
const zUp = buildSceneDocumentFromFbx(scene(Z_UP, [0, -2.818, 0]));

assert.deepEqual(
  positions(zUp, 'POSITION'),
  [1, -3, 2, 0, 0, 0, 1, 0, 0],
  'FBX (x, y, z) has to arrive as glTF (x, -z, y)',
);
assert.deepEqual(
  positions(zUp, 'NORMAL'),
  [1, -3, 2, 0, -1, 0, 0, -1, 0],
  'normals turn with the basis, and are not scaled by the unit factor',
);
// UnitScaleFactor 100 means metres, so the coordinates above are unscaled.
assert.deepEqual(
  zUp.nodes[0].matrix!.slice(12, 15).map((value) => Math.round(value * 1e6) / 1e6),
  [0, 0, -2.818],
  'the node transform is conjugated, not merely rotated as a point',
);
assert.deepEqual(
  scaleCurve(zUp).map((value) => Math.round(value * 1e6) / 1e6),
  [2, 4, 3],
  'per-axis scale follows the axes: glTF Y takes the FBX up axis, Z the front one',
);

// The same file declared Y-up has to come through the way it always did: the
// axes are already glTF's, and only the centimetre factor applies.
const yUp = buildSceneDocumentFromFbx(scene(Y_UP, [0, 100, 0]));
assert.deepEqual(
  positions(yUp, 'POSITION'),
  [0.01, 0.02, 0.03, 0, 0, 0, 0.01, 0, 0],
  'a Y-up file is scaled and not rotated',
);
assert.deepEqual(
  positions(yUp, 'NORMAL'),
  [1, 2, 3, 0, 0, 1, 0, 0, 1],
  'and its normals are left exactly alone',
);
assert.deepEqual(
  yUp.nodes[0].matrix!.slice(12, 15).map((value) => Math.round(value * 1e6) / 1e6),
  [0, 1, 0],
  '100 centimetres is one metre',
);
assert.deepEqual(scaleCurve(yUp), [2, 3, 4], 'and its scale curve keeps its order');

console.log('FBX axis basis and unit scale passed');
