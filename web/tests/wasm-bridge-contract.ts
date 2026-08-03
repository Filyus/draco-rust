/**
 * What the hand-written wasm bridges accept, and what they refuse.
 *
 * The bridges replaced serde with `js_sys::Reflect` plumbing, which moved
 * validation from a derive onto code nobody exercises from Rust: `js_sys` needs
 * a JavaScript runtime, so `cargo test` reaches only the `*_internal` functions
 * underneath. Everything the conversion layer decides — which array types a
 * field admits, what an absent field means, which values are refused — is
 * therefore only testable from here, against the built modules.
 *
 * The refusals matter more than the acceptances. A float-to-integer `as` in
 * Rust saturates rather than wrapping, so an out-of-range index silently lands
 * on a real vertex and 0..1 colours silently become near-black bytes. Both
 * produce a plausible file rather than an error, which is the failure these
 * gates exist to keep out.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

async function load(name: string) {
  const module = await import(pathToFileURL(resolve(pkg, `${name}.js`)).href);
  await module.default({ module_or_path: await readFile(resolve(pkg, `${name}_bg.wasm`)) });
  return module;
}

const [ply, drc, stl, obj, fbx] = await Promise.all(
  ['ply', 'drc', 'stl', 'obj', 'fbx'].map(load),
);

const positions = new Float32Array([0, 0, 0, 1, 0, 0, 1, 1, 0]);
const indices = new Uint32Array([0, 1, 2]);
const plyOptions = {
  include_normals: false, include_uvs: false, include_colors: true, precision: 6, format: 'ascii',
};

/** The vertex rows of an ASCII PLY, which is where colours land. */
function plyVertexRows(result: { success: boolean; data?: string; error?: string }) {
  assert.ok(result.success, `expected a PLY, got: ${result.error}`);
  return result.data!.split('end_header\n')[1].trim().split('\n');
}

// ---------------------------------------------------------------------------
// Colour domain
// ---------------------------------------------------------------------------

// Bytes are the domain the writers hold colours in, and the one every reader
// hands them over in.
{
  const rows = plyVertexRows(ply.create_ply({
    name: 't',
    positions,
    indices,
    colors: new Uint8Array([255, 255, 255, 255, 255, 0, 0, 255, 128, 128, 128, 255]),
  }, plyOptions));
  assert.equal(rows[0], '0.000000 0.000000 0.000000 255 255 255 255');
  assert.equal(rows[2], '1.000000 1.000000 0.000000 128 128 128 255');
}

// A plain array carries the same bytes: `flattenSceneDocument` produces one.
{
  const rows = plyVertexRows(ply.create_ply({
    name: 't',
    positions,
    indices,
    colors: [255, 255, 255, 255, 255, 0, 0, 255, 128, 128, 128, 255],
  }, plyOptions));
  assert.equal(rows[0], '0.000000 0.000000 0.000000 255 255 255 255');
}

// The gate this file exists for. The FBX reader holds colours as 0..1 floats,
// and casting those to bytes writes an almost black mesh that looks like a
// successful export. Refused by type, because once the values are in, nothing
// on the Rust side can tell 0..1 from 0..255.
for (const [name, module, create, options] of [
  ['ply', ply, 'create_ply', plyOptions],
  ['drc', drc, 'create_drc', { include_colors: true }],
] as const) {
  for (const floats of [
    new Float32Array([1, 1, 1, 1, 1, 0, 0, 1, 0.5, 0.5, 0.5, 1]),
    new Float64Array([1, 1, 1, 1, 1, 0, 0, 1, 0.5, 0.5, 0.5, 1]),
  ]) {
    const result = (module as any)[create]({ name: 't', positions, indices, colors: floats }, options);
    assert.equal(result.success, false, `${name} accepted a ${floats.constructor.name} of colours`);
    assert.match(result.error, /0\.\.255 bytes/, `${name}: ${result.error}`);
  }
}

// A byte value out of range is a caller bug rather than something to clamp.
{
  const result = ply.create_ply({
    name: 't', positions, indices, colors: [300, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255],
  }, plyOptions);
  assert.equal(result.success, false, 'a 300 byte was accepted');
  assert.match(result.error, /within 0..=255/);
}

// ---------------------------------------------------------------------------
// Index validation
// ---------------------------------------------------------------------------

// Saturation is what makes these dangerous: `-1 as u32` is 0, a real vertex, so
// the file comes out with a degenerate triangle and no complaint.
for (const [label, bad, pattern] of [
  ['negative', [0, 1, -1], /whole numbers|within 0/],
  ['fractional', [0, 1, 1.5], /whole numbers/],
  ['past u32', [0, 1, 2 ** 32], /within 0/],
  ['not a number', [0, 1, 'x'], /only numbers/],
] as const) {
  for (const [name, call] of [
    ['ply', () => ply.create_ply({ name: 't', positions, indices: bad }, plyOptions)],
    ['stl', () => stl.create_stl({ positions, indices: bad }, { format: 'binary' })],
    ['obj', () => obj.create_obj({ name: 't', positions, indices: bad }, { precision: 6 })],
    ['drc', () => drc.create_drc({ name: 't', positions, indices: bad }, {})],
  ] as const) {
    const result = call() as { success: boolean; error?: string };
    assert.equal(result.success, false, `${name} accepted a ${label} index`);
    assert.match(result.error!, pattern, `${name} / ${label}: ${result.error}`);
  }
}

// FBX reads the same field signed, because the sign carries meaning there: a
// negative entry closes a polygon and -1 in materialIndices means no material.
{
  const result = fbx.create_fbx([{
    name: 't', positions, indices, materialIndices: [-1], polygonVertexIndices: [0, 1, -3],
  }], { version: 7500 });
  assert.ok(result.success, `FBX refused a legitimate negative: ${result.error}`);
}

// ---------------------------------------------------------------------------
// Absent, empty and null fields
// ---------------------------------------------------------------------------

// `prepareMeshesForExport` nulls a channel the checkboxes exclude and empties
// one the source lacks. Both have to mean "no attribute" rather than an error.
for (const [label, channels] of [
  ['null channels', { normals: null, uvs: null, colors: null }],
  ['empty channels', { normals: [], uvs: [], colors: [] }],
  ['absent channels', {}],
] as const) {
  const result = ply.create_ply({ name: 't', positions, indices, ...channels }, {
    ...plyOptions, include_normals: true, include_uvs: true,
  });
  assert.ok(result.success, `${label}: ${result.error}`);
  assert.doesNotMatch(result.data, /property float nx/, `${label} produced a normal element`);
}

// A mesh with no index buffer still exports: `indices: []` is the shape
// `prepareMeshesForExport` produces for a non-indexed source. PLY writes it as
// a vertex element and no face element at all, which is a point cloud rather
// than an empty surface.
{
  const result = ply.create_ply({ name: 't', positions, indices: [] }, plyOptions);
  assert.ok(result.success, `non-indexed mesh: ${result.error}`);
  assert.match(result.data, /element vertex 3/);
  assert.doesNotMatch(result.data, /element face/);
}

// A missing required field is an error rather than an empty mesh.
{
  const result = ply.create_ply({ name: 't', positions }, plyOptions);
  assert.equal(result.success, false, 'a mesh with no indices field was accepted');
}

// `undefined` and `null` both read as absent, which is what `get_field` decides
// and what every optional field downstream depends on.
for (const missing of [undefined, null]) {
  const result = ply.create_ply({ name: 't', positions, indices, colors: missing }, plyOptions);
  assert.ok(result.success, `colors: ${missing} was rejected`);
  assert.doesNotMatch(result.data, /property uchar red/);
}

// ---------------------------------------------------------------------------
// OBJ takes bytes
// ---------------------------------------------------------------------------

{
  const source = 'v 0 0 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\n';
  const parsed = obj.parse_obj_bytes(new TextEncoder().encode(source));
  assert.ok(parsed.success, `parse_obj_bytes: ${parsed.error}`);
  assert.equal(parsed.meshes[0].positions.length, 9);
  assert.ok(parsed.meshes[0].positions instanceof Float32Array, 'positions crossed as a plain array');
  assert.ok(parsed.meshes[0].indices instanceof Uint32Array, 'indices crossed as a plain array');
  // The string entry point is gone; bytes are the one way in.
  assert.equal(typeof (obj as any).parse_obj, 'undefined', 'parse_obj is still exported');
  // Invalid UTF-8 is reported rather than thrown across the boundary.
  const broken = obj.parse_obj_bytes(new Uint8Array([0xff, 0xfe, 0x00]));
  assert.equal(broken.success, false);
  assert.match(broken.error, /UTF-8/);
}

// ---------------------------------------------------------------------------
// Array shapes the shell relies on
// ---------------------------------------------------------------------------

// materialIndices is checked with `Array.isArray` in prepareFbxSceneForExport,
// so it has to cross as a plain array. A typed one would read as absent there
// and drop every per-polygon material assignment without a word.
{
  const written = fbx.create_fbx([{
    name: 't', positions, indices, materialIndices: [0],
  }], { version: 7500 });
  assert.ok(written.success, `fbx write: ${written.error}`);
  const back = fbx.parse_fbx(written.binary_data);
  assert.ok(back.success, `fbx read: ${back.error}`);
  const mesh = back.scene.rootNodes[0].meshes[0];
  assert.ok(Array.isArray(mesh.materialIndices), 'materialIndices crossed as a typed array');
  // Geometry, by contrast, is typed: that is the whole point of the bridge.
  assert.ok(mesh.positions instanceof Float32Array, 'positions crossed as a plain array');
}

console.log('wasm bridge contract passed');
