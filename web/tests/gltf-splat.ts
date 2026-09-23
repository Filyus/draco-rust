/**
 * `KHR_gaussian_splatting` as a splat cloud: the paths the browser tests do
 * not reach.
 *
 * Those load a float32 splat through the whole page and check it against the
 * same splat read from a PLY. What they cannot reach cheaply is everything the
 * specification allows besides float32 -- normalized and plain integer
 * attributes -- and every way a primitive can fall short of the extension,
 * where the answer is a warning and a point cloud rather than a guess.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { splatCloudFromSceneDocument } from '../src/gltf-splat.ts';
import { tinySplatPly } from './smoke-fixtures.ts';
import type {
  ComponentType, SceneAccessor, SceneDocument, SceneGaussianSplatting, ScenePrimitive,
} from '../src/scene-document.ts';

/** An accessor of `components` values per element, in the given storage. */
function accessor(
  values: number[],
  components: number,
  componentType: ComponentType = 5126,
  normalized = false,
): SceneAccessor {
  const count = values.length / components;
  let bytes: Uint8Array;
  switch (componentType) {
    case 5126: bytes = new Uint8Array(new Float32Array(values).buffer); break;
    case 5122: bytes = new Uint8Array(new Int16Array(values).buffer); break;
    case 5120: bytes = new Uint8Array(new Int8Array(values).buffer); break;
    case 5121: bytes = new Uint8Array(values); break;
    default: throw new Error(`no fixture storage for ${componentType}`);
  }
  return { bytes, componentType, components, count, normalized };
}

const SRGB: SceneGaussianSplatting = { kernel: 'ellipse', colorSpace: 'srgb_rec709_display' };

/**
 * A document of one node placing one primitive. `attributes` maps semantics
 * to accessors; the node takes whatever transform fields are given.
 */
function documentOf(
  attributes: Record<string, SceneAccessor>,
  node: Record<string, unknown> = {},
  primitive: Partial<ScenePrimitive> = {},
): SceneDocument {
  const accessors: SceneAccessor[] = [];
  const map: Record<string, number> = {};
  for (const [semantic, value] of Object.entries(attributes)) {
    map[semantic] = accessors.length;
    accessors.push(value);
  }
  return {
    version: 1,
    resources: [],
    textures: [],
    materials: [],
    accessors,
    meshes: [{ primitives: [{ attributes: map, mode: 0, gaussianSplatting: SRGB, ...primitive }] }],
    nodes: [{ mesh: 0, ...node }],
    rootNodes: [0],
    skins: [],
    animations: [],
    warnings: [],
  };
}

const P = 'KHR_gaussian_splatting:';

/** One splat, float32 throughout, degree 0. */
function oneSplat(overrides: Record<string, SceneAccessor> = {}) {
  return {
    POSITION: accessor([1, 2, 3], 3),
    [`${P}ROTATION`]: accessor([0, 0, 0, 1], 4),
    [`${P}SCALE`]: accessor([0.5, 0.25, 0.125], 3),
    [`${P}OPACITY`]: accessor([0.75], 1),
    [`${P}SH_DEGREE_0_COEF_0`]: accessor([0.1, 0.2, 0.3], 3),
    ...overrides,
  };
}

function near(actual: ArrayLike<number>, expected: number[], tolerance: number, what: string) {
  assert.equal(actual.length, expected.length, `${what}: length`);
  expected.forEach((value, index) => {
    assert.ok(Math.abs(actual[index] - value) <= tolerance,
      `${what}[${index}] is ${actual[index]}, expected ${value}`);
  });
}

// ---------------------------------------------------------------------------
// Domains: glTF stores what a PLY stores the log or logit of
// ---------------------------------------------------------------------------

{
  const { cloud, warnings } = splatCloudFromSceneDocument(documentOf(oneSplat()));
  assert.deepEqual(warnings, []);
  assert.ok(cloud);
  near(cloud.positions, [1, 2, 3], 0, 'positions stay where an unrotated node puts them');
  near(cloud.scales, [0.5, 0.25, 0.125], 0, 'scales are lengths already');
  near(cloud.alphas, [0.75], 0, 'opacity is alpha already');
  // glTF's (x, y, z, w) becomes the cloud's (w, x, y, z).
  near(cloud.rotations, [1, 0, 0, 0], 0, 'identity, reordered');
  near(cloud.dc, [0.1, 0.2, 0.3], 1e-7, 'the degree-0 harmonic is untouched');
  assert.equal(cloud.shDegree, 0);
  near(cloud.shFrame, [1, 0, 0, 0, 1, 0, 0, 0, 1], 0, 'no rotation, no harmonic frame');
  assert.equal(cloud.colorSpace, 'srgb');
}

// The quantized storage the extension allows: rotation as normalized short,
// opacity as normalized byte, scale as a plain byte the node scales back.
{
  const document = documentOf(oneSplat({
    [`${P}ROTATION`]: accessor([0, 0, 32767, 32767], 4, 5122, true),
    [`${P}OPACITY`]: accessor([191], 1, 5121, true),
    [`${P}SCALE`]: accessor([2, 4, 8], 3, 5121, false),
  }), { scale: [0.25, 0.25, 0.25] });
  const { cloud, warnings } = splatCloudFromSceneDocument(document);
  assert.deepEqual(warnings, []);
  assert.ok(cloud);
  near(cloud.alphas, [191 / 255], 1e-7, 'a normalized byte is a fraction of 255');
  near(cloud.scales, [0.5, 1, 2], 1e-6, 'a plain byte is its value, times the node scale');
  // A normalized short cannot hold a unit quaternion exactly, and the reader
  // renormalizes it: a half turn about Z, (0, 0, 1, 1) / sqrt 2.
  near(cloud.rotations, [Math.SQRT1_2, 0, 0, Math.SQRT1_2], 1e-6, 'renormalized');
  near(cloud.positions, [0.25, 0.5, 0.75], 1e-7, 'positions take the whole node');
}

// A node rotation moves positions and orientations into the world and leaves
// its inverse as the harmonics' frame.
{
  const s = Math.SQRT1_2;
  // A quarter turn about Z: x goes to y.
  const document = documentOf(oneSplat({ POSITION: accessor([1, 0, 0], 3) }), { rotation: [0, 0, s, s] });
  const { cloud } = splatCloudFromSceneDocument(document);
  assert.ok(cloud);
  near(cloud.positions, [0, 1, 0], 1e-6, 'x turned onto y');
  near(cloud.rotations, [s, 0, 0, s], 1e-6, 'the node turns the splat');
  // World to local: y goes back to x.
  near(cloud.shFrame, [0, 1, 0, -1, 0, 0, 0, 0, 1], 1e-6, 'the inverse rotation, row-major');
}

// ---------------------------------------------------------------------------
// Harmonics: complete degrees only, in the order they are declared
// ---------------------------------------------------------------------------

{
  const band1 = [0, 1, 2].map((m) => [10 + m, 20 + m, 30 + m]);
  const document = documentOf(oneSplat(Object.fromEntries(band1.map((rgb, m) => [
    `${P}SH_DEGREE_1_COEF_${m}`, accessor(rgb, 3),
  ]))));
  const { cloud, warnings } = splatCloudFromSceneDocument(document);
  assert.deepEqual(warnings, []);
  assert.ok(cloud);
  assert.equal(cloud.shDegree, 1);
  near(cloud.sh, band1.flat(), 0, 'coefficient-major, rgb inside, m from -1 to 1');
}

// Part of a degree is forbidden, and is reported rather than read quietly as
// the degree below.
{
  const document = documentOf(oneSplat({
    [`${P}SH_DEGREE_1_COEF_0`]: accessor([1, 1, 1], 3),
    [`${P}SH_DEGREE_1_COEF_1`]: accessor([1, 1, 1], 3),
  }));
  const { cloud, warnings } = splatCloudFromSceneDocument(document);
  assert.ok(cloud);
  assert.equal(cloud.shDegree, 0);
  assert.equal(warnings.length, 1);
  assert.match(warnings[0], /only partly present/);
}

// ---------------------------------------------------------------------------
// What is not a splat as the extension defines one
// ---------------------------------------------------------------------------

// The draft form in circulation: colour in COLOR_0, harmonics as VEC4, no
// OPACITY and no degree-0 harmonic. Named, and left to draw as points.
{
  const draft = oneSplat({ COLOR_0: accessor([255, 128, 0, 200], 4, 5121, true) });
  delete (draft as Record<string, unknown>)[`${P}OPACITY`];
  delete (draft as Record<string, unknown>)[`${P}SH_DEGREE_0_COEF_0`];
  const { cloud, warnings } = splatCloudFromSceneDocument(documentOf(draft));
  assert.equal(cloud, null);
  assert.equal(warnings.length, 1);
  assert.match(warnings[0], /missing or mistyped: OPACITY, SH_DEGREE_0_COEF_0/);
}

// The extension requires POINTS.
{
  const { cloud, warnings } = splatCloudFromSceneDocument(documentOf(oneSplat(), {}, { mode: 4 }));
  assert.equal(cloud, null);
  assert.match(warnings[0], /must be POINTS/);
}

// Linear colour is honoured; an unknown colour space is named and read as the
// display encoding 3DGS trains in.
{
  const linear = splatCloudFromSceneDocument(documentOf(oneSplat(), {}, {
    gaussianSplatting: { kernel: 'ellipse', colorSpace: 'lin_rec709_display' },
  }));
  assert.equal(linear.cloud?.colorSpace, 'linear');
  assert.deepEqual(linear.warnings, []);

  const unknown = splatCloudFromSceneDocument(documentOf(oneSplat(), {}, {
    gaussianSplatting: { kernel: 'ellipse', colorSpace: 'acescg' },
  }));
  assert.equal(unknown.cloud?.colorSpace, 'srgb');
  assert.match(unknown.warnings[0], /"acescg" is not one the extension defines/);
}

// A non-uniform node scale shears a rotated gaussian into a shape no axis
// triple holds; the mean is used, and said.
{
  const { cloud, warnings } = splatCloudFromSceneDocument(documentOf(oneSplat(), { scale: [1, 2, 4] }));
  assert.ok(cloud);
  near(cloud.scales, [1, 0.5, 0.25], 1e-6, 'scaled by the mean, 2');
  assert.match(warnings[0], /scales unevenly/);
}

// ---------------------------------------------------------------------------
// A file another tool wrote
// ---------------------------------------------------------------------------

// `testdata/GaussianSplats/tiny-splat.gltf` is the independent half of the
// check: a converter this repository did not write turned `tinySplatPly(64)`
// into the extension, and reading it back has to give the splats that PLY
// holds, domain by domain. The converter keeps the PLY's frame and places it
// under a node with no rotation, so positions and orientations come back as
// the PLY stored them -- not turned upright, which is the PLY reader's doing.
{
  const here = dirname(fileURLToPath(import.meta.url));
  const repoRoot = resolve(here, '..', '..');
  const pkg = resolve(here, '..', 'www', 'pkg');
  const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
  await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });
  const { buildSceneDocumentFromGltf } = await import(
    pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
  );

  const folder = resolve(repoRoot, 'testdata', 'GaussianSplats');
  const document = buildSceneDocumentFromGltf(
    new Uint8Array(await readFile(resolve(folder, 'tiny-splat.gltf'))),
    { 'tiny-splat.bin': new Uint8Array(await readFile(resolve(folder, 'tiny-splat.bin'))) },
    gltfModule,
  );
  const { cloud, warnings } = splatCloudFromSceneDocument(document);
  assert.deepEqual(warnings, []);
  assert.ok(cloud);
  assert.equal(cloud.count, 64);
  assert.equal(cloud.shDegree, 3);
  assert.equal(cloud.colorSpace, 'srgb');

  const { values } = tinySplatPly(64);
  const at = (name: string, splat: number) => Math.fround(values[name][splat]);
  const sigmoid = (x: number) => 1 / (1 + Math.exp(-x));
  for (let splat = 0; splat < 64; splat += 1) {
    near(cloud.positions.subarray(splat * 3, splat * 3 + 3),
      [at('x', splat), at('y', splat), at('z', splat)], 0, `positions of ${splat}`);
    near(cloud.scales.subarray(splat * 3, splat * 3 + 3),
      [0, 1, 2].map((axis) => Math.exp(at(`scale_${axis}`, splat))), 1e-6, `scales of ${splat}`);
    near(cloud.alphas.subarray(splat, splat + 1), [sigmoid(at('opacity', splat))], 1e-6, `alpha of ${splat}`);
    const quaternion = [0, 1, 2, 3].map((i) => at(`rot_${i}`, splat));
    const length = Math.hypot(...quaternion);
    near(cloud.rotations.subarray(splat * 4, splat * 4 + 4),
      quaternion.map((value) => value / length), 1e-6, `rotation of ${splat}`);
    near(cloud.dc.subarray(splat * 3, splat * 3 + 3),
      [0, 1, 2].map((i) => at(`f_dc_${i}`, splat)), 0, `dc of ${splat}`);
    // f_rest is channel-major; the cloud's coefficient k is (red k, green k, blue k).
    const expected = Array.from({ length: 15 }, (_, k) =>
      [at(`f_rest_${k}`, splat), at(`f_rest_${k + 15}`, splat), at(`f_rest_${k + 30}`, splat)]).flat();
    near(cloud.sh.subarray(splat * 45, splat * 45 + 45), expected, 0, `harmonics of ${splat}`);
  }
  near(cloud.shFrame, [1, 0, 0, 0, 1, 0, 0, 0, 1], 0, 'no node rotation, no harmonic frame');
}

console.log('glTF splats: domains, quantized storage, node transforms and harmonics read as specified');
