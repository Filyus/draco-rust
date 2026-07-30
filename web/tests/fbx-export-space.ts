/**
 * Both export spaces have to survive their own round trip.
 *
 * FBX declares its coordinate system rather than fixing one, so writing it is a
 * choice: `meters-y-up` is glTF's own space and needs no conversion, and
 * `meters-z-up` is what a good deal of existing FBX looks like. A file written
 * in either and read back has to come out where it started — which is only true
 * if the writer's conversion, the writer's declaration and the importer's
 * reading of that declaration all agree. They did not: the declaration said
 * Z-up while the geometry was turned the other way, and the importer read
 * neither.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import type { buildSceneDocumentFromFbx as BuildSceneDocumentFromFbx } from '../src/fbx-scene-document.ts';
import type { buildFbxSceneFromDocument as BuildFbxSceneFromDocument } from '../src/fbx-scene-document-writer.ts';
import type { buildSceneDocumentFromMeshes as BuildSceneDocumentFromMeshes } from '../src/mesh-scene-document.ts';
import type { FbxExportSpaceName } from '../src/fbx-space.ts';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

const fbxModule = await import(pathToFileURL(resolve(pkg, 'fbx.js')).href);
await fbxModule.default({ module_or_path: await readFile(resolve(pkg, 'fbx_bg.wasm')) });

const { buildSceneDocumentFromMeshes } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'mesh-scene-document.ts')).href
) as { buildSceneDocumentFromMeshes: typeof BuildSceneDocumentFromMeshes };
const { buildFbxSceneFromDocument } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'fbx-scene-document-writer.ts')).href
) as { buildFbxSceneFromDocument: typeof BuildFbxSceneFromDocument };
const { buildSceneDocumentFromFbx } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'fbx-scene-document.ts')).href
) as { buildSceneDocumentFromFbx: typeof BuildSceneDocumentFromFbx };

/** Distinct on every axis, so a swapped pair or a dropped sign cannot hide. */
const POSITIONS = [1, 2, 3, -4, 5, -6, 7, -8, 9];
const TRANSLATION = [0.25, -1.5, 2.75];

const source = buildSceneDocumentFromMeshes([{
  name: 'Probe',
  positions: POSITIONS,
  indices: [0, 1, 2],
  normals: [0, 1, 0, 0, 1, 0, 0, 1, 0],
}]);
source.nodes[0].translation = [...TRANSLATION];

/**
 * A quarter turn about glTF's +Y, animated.
 *
 * Rotations are the one channel the writer converted by hand rather than from
 * the space -- the conversion was spelled out for the one Z-up convention it
 * used to emit -- so a Y-up export was turning them and nothing else. A curve is
 * the only way to reach that path: a static node's rotation goes through its
 * matrix instead.
 */
const QUARTER_TURN_Y = [0, Math.SQRT1_2, 0, Math.SQRT1_2];
source.animations.push({
  name: 'Turn',
  duration: 1,
  samplers: [{
    input: appendFloats(source, [0, 1], 1),
    output: appendFloats(source, [...QUARTER_TURN_Y, ...QUARTER_TURN_Y], 4),
    interpolation: 'LINEAR',
  }],
  channels: [{ sampler: 0, node: 0, path: 'rotation' }],
});

function appendFloats(document: typeof source, values: number[], components: number): number {
  const index = document.accessors.length;
  document.accessors.push({
    bytes: new Uint8Array(Float32Array.from(values).buffer),
    componentType: 5126,
    components,
    count: values.length / components,
    normalized: false,
  });
  return index;
}

function readPositions(document: ReturnType<typeof buildSceneDocumentFromFbx>): number[] {
  const primitive = document.meshes[0].primitives[0];
  const accessor = document.accessors[primitive.attributes.POSITION];
  return Array.from(new Float32Array(
    accessor.bytes.buffer,
    accessor.bytes.byteOffset,
    accessor.bytes.byteLength / 4,
  ));
}

function nodeTranslation(document: ReturnType<typeof buildSceneDocumentFromFbx>): number[] {
  const matrix = document.nodes[0].matrix;
  assert.ok(matrix, 'the imported node must carry a matrix');
  return matrix.slice(12, 15);
}

function close(actual: number[], expected: number[], label: string) {
  assert.equal(actual.length, expected.length, `${label}: length`);
  for (let index = 0; index < actual.length; index += 1) {
    assert.ok(
      Math.abs(actual[index] - expected[index]) < 1e-4,
      `${label}: component ${index} is ${actual[index]}, expected ${expected[index]}`,
    );
  }
}

for (const space of ['meters-y-up', 'meters-z-up'] as FbxExportSpaceName[]) {
  const scene = buildFbxSceneFromDocument(source, { space });
  const written = fbxModule.create_fbx_scene(scene, {});
  assert.ok(written.success, `${space}: ${written.error}`);

  const parsed = fbxModule.parse_fbx(new Uint8Array(written.binary_data));
  assert.ok(parsed.success, `${space}: ${parsed.error}`);
  const document = buildSceneDocumentFromFbx(parsed);

  close(readPositions(document), POSITIONS, `${space} positions`);
  close(nodeTranslation(document), TRANSLATION, `${space} node translation`);

  // The animated rotation has to come back as the same quarter turn about the
  // same axis. FBX stores Euler degrees, so the comparison happens after the
  // importer has turned them back into a quaternion, either sign of it.
  const clip = document.animations[0];
  const rotation = clip.channels.find((entry) => entry.path === 'rotation');
  assert.ok(rotation, `${space}: the rotation channel survived`);
  const keys = document.accessors[clip.samplers[rotation.sampler].output];
  const first = Array.from(new Float32Array(
    keys.bytes.buffer, keys.bytes.byteOffset, 4,
  ));
  const flipped = first.map((value) => -value);
  const matches = QUARTER_TURN_Y.every((value, axis) => Math.abs(first[axis] - value) < 1e-3)
    || QUARTER_TURN_Y.every((value, axis) => Math.abs(flipped[axis] - value) < 1e-3);
  assert.ok(matches, `${space}: rotation key came back as ${first.join(', ')}`);

  const settings = parsed.scene.globalSettings;
  assert.equal(settings.unitScaleFactor, 100, `${space}: metres`);
  assert.equal(settings.upAxis, space === 'meters-y-up' ? 1 : 2, `${space}: up axis`);

  // The round trip above only proves the writer and the reader agree; both read
  // the same declaration, so a declaration that contradicts the geometry passes
  // it. This is the part that does not: the coordinates in the file are checked
  // against what the file itself claims about its axes. glTF component `r` has
  // to land on FBX axis `axes[r]` with sign `signs[r]`.
  const axes = [settings.coordAxis, settings.upAxis, settings.frontAxis];
  const signs = [settings.coordAxisSign, settings.upAxisSign, settings.frontAxisSign];
  const coordinates: number[] = parsed.scene.rootNodes[0].meshes[0].positions;
  const claimed = new Array(POSITIONS.length).fill(0);
  for (let vertex = 0; vertex * 3 < POSITIONS.length; vertex += 1) {
    for (let component = 0; component < 3; component += 1) {
      claimed[vertex * 3 + axes[component]] = signs[component] * POSITIONS[vertex * 3 + component];
    }
  }
  close(Array.from(coordinates), claimed, `${space} coordinates against the declaration`);
  console.log(`PASS ${space} FBX round trip, and its declaration matches its coordinates`);
}
