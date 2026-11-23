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

// The standalone encoder's named color channel has the same explicit bit
// control as the glTF encoder. It must reach the quantizer rather than merely
// be accepted by the JavaScript option parser.
{
  const input = {
    name: 'color-quantization',
    positions,
    indices,
    colors: new Uint8Array([
      0, 32, 64, 255,
      128, 160, 192, 255,
      255, 224, 192, 255,
    ]),
  };
  const exact = drc.encode_draco(input, {});
  const quantized = drc.encode_draco(input, { color_bits: 2 });
  assert.ok(exact.success, `unquantized color encode: ${exact.error}`);
  assert.ok(quantized.success, `color_bits encode: ${quantized.error}`);
  const exactColors = drc.decode_draco(exact.bytes, [{ semantic: 'COLOR' }]);
  const quantizedColors = drc.decode_draco(quantized.bytes, [{ semantic: 'COLOR' }]);
  assert.ok(exactColors.success && quantizedColors.success);
  assert.ok(exactColors.attributes[0].array instanceof Uint8Array);
  assert.ok(quantizedColors.attributes[0].array instanceof Float32Array);
  assert.notDeepEqual(
    quantizedColors.attributes[0].array,
    exactColors.attributes[0].array,
    'color_bits did not affect the stream',
  );
}

/** The vertex rows of an ASCII PLY, which is where colours land. */
function plyVertexRows(result: { success: boolean; text?: string; error?: string }) {
  assert.ok(result.success, `expected a PLY, got: ${result.error}`);
  return result.text!.split('end_header\n')[1].trim().split('\n');
}

// ---------------------------------------------------------------------------
// Colour domain
// ---------------------------------------------------------------------------

// Bytes are the domain the writers hold colours in, and the one every reader
// hands them over in.
{
  const rows = plyVertexRows(ply.write_ply({
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
  const rows = plyVertexRows(ply.write_ply({
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
  ['ply', ply, 'write_ply', plyOptions],
  ['drc', drc, 'encode_draco', { include_colors: true }],
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
  const result = ply.write_ply({
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
    ['ply', () => ply.write_ply({ name: 't', positions, indices: bad }, plyOptions)],
    ['stl', () => stl.write_stl({ positions, indices: bad }, { format: 'binary' })],
    ['obj', () => obj.write_obj({ name: 't', positions, indices: bad }, { precision: 6 })],
    ['drc', () => drc.encode_draco({ name: 't', positions, indices: bad }, {})],
  ] as const) {
    const result = call() as { success: boolean; error?: string };
    assert.equal(result.success, false, `${name} accepted a ${label} index`);
    assert.match(result.error!, pattern, `${name} / ${label}: ${result.error}`);
  }
}

// FBX reads the same field signed, because the sign carries meaning there: a
// negative entry closes a polygon and -1 in materialIndices means no material.
{
  const result = fbx.write_fbx([{
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
  const result = ply.write_ply({ name: 't', positions, indices, ...channels }, {
    ...plyOptions, include_normals: true, include_uvs: true,
  });
  assert.ok(result.success, `${label}: ${result.error}`);
  assert.doesNotMatch(result.text, /property float nx/, `${label} produced a normal element`);
}

// A mesh with no index buffer still exports: `indices: []` is the shape
// `prepareMeshesForExport` produces for a non-indexed source. PLY writes it as
// a vertex element and no face element at all, which is a point cloud rather
// than an empty surface.
{
  const result = ply.write_ply({ name: 't', positions, indices: [] }, plyOptions);
  assert.ok(result.success, `non-indexed mesh: ${result.error}`);
  assert.match(result.text, /element vertex 3/);
  assert.doesNotMatch(result.text, /element face/);
}

// A missing required field is an error rather than an empty mesh.
{
  const result = ply.write_ply({ name: 't', positions }, plyOptions);
  assert.equal(result.success, false, 'a mesh with no indices field was accepted');
}

// `undefined` and `null` both read as absent, which is what `get_field` decides
// and what every optional field downstream depends on.
for (const missing of [undefined, null]) {
  const result = ply.write_ply({ name: 't', positions, indices, colors: missing }, plyOptions);
  assert.ok(result.success, `colors: ${missing} was rejected`);
  assert.doesNotMatch(result.text, /property uchar red/);
}

// ---------------------------------------------------------------------------
// OBJ takes bytes
// ---------------------------------------------------------------------------

{
  const source = 'v 0 0 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\n';
  const parsed = obj.read_obj(new TextEncoder().encode(source));
  assert.ok(parsed.success, `read_obj: ${parsed.error}`);
  assert.equal(parsed.meshes[0].positions.length, 9);
  assert.ok(parsed.meshes[0].positions instanceof Float32Array, 'positions crossed as a plain array');
  assert.ok(parsed.meshes[0].indices instanceof Uint32Array, 'indices crossed as a plain array');
  // The string entry point is gone; bytes are the one way in.
  assert.equal(typeof (obj as any).parse_obj, 'undefined', 'parse_obj is still exported');
  // Invalid UTF-8 is reported rather than thrown across the boundary.
  const broken = obj.read_obj(new Uint8Array([0xff, 0xfe, 0x00]));
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
  const written = fbx.write_fbx([{
    name: 't', positions, indices, materialIndices: [0],
  }], { version: 7500 });
  assert.ok(written.success, `fbx write: ${written.error}`);
  const back = fbx.read_fbx(written.bytes);
  assert.ok(back.success, `fbx read: ${back.error}`);
  const mesh = back.scene.rootNodes[0].meshes[0];
  assert.ok(Array.isArray(mesh.materialIndices), 'materialIndices crossed as a typed array');
  // Geometry, by contrast, is typed: that is the whole point of the bridge.
  assert.ok(mesh.positions instanceof Float32Array, 'positions crossed as a plain array');
}

// ---------------------------------------------------------------------------
// Explicit quantization grid validates source values
// ---------------------------------------------------------------------------

for (const bad of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
  const result = drc.encode_draco({
    name: 'non-finite',
    positions: new Float32Array([0, bad, 0, 1, 0, 0, 1, 1, 0]),
    indices,
  }, {
    position_bits: 10,
    position_origin: [0, 0, 0],
    position_range: 1,
  });
  assert.equal(result.success, false, `explicit grid accepted ${bad} in positions`);
  assert.match(result.error, /NaN|infinite|finite/i, `explicit grid (${bad}): ${result.error}`);
}

// ---------------------------------------------------------------------------
// Metadata dictionaries preserve `__proto__` across decode and recode
// ---------------------------------------------------------------------------

{
  const utf8 = (text: string) => new TextEncoder().encode(text);
  const topEntries = Object.create(null) as Record<string, Uint8Array>;
  topEntries.__proto__ = utf8('top-proto');
  topEntries.normal = utf8('ok');
  const childEntries = Object.create(null) as Record<string, Uint8Array>;
  childEntries.__proto__ = utf8('child-proto');
  const subMetadata = Object.create(null) as Record<string, unknown>;
  subMetadata.__proto__ = { entries: childEntries };
  const extraEntries = Object.create(null) as Record<string, Uint8Array>;
  extraEntries.__proto__ = utf8('attr-proto');

  const meshInput = {
    name: 'proto-meta',
    positions,
    indices,
    extras: [{
      type: 'GENERIC',
      components: 1,
      dataType: 'float32',
      uniqueId: 10,
      normalized: false,
      name: 'weights',
      metadata: { entries: extraEntries },
      values: new Float32Array([0.25, 0.5, 0.75]),
    }],
    metadata: { entries: topEntries, subMetadata },
  };

  const firstEncode = drc.encode_draco(meshInput, {});
  assert.ok(firstEncode.success, `encode with __proto__ metadata: ${firstEncode.error}`);

  const checkMetadata = (meta: any, label: string) => {
    assert.ok(meta, `${label}: metadata missing`);
    assert.equal(Object.getPrototypeOf(meta.entries), null, `${label}: entries prototype`);
    assert.equal(Object.getPrototypeOf(meta.strings), null, `${label}: strings prototype`);
    assert.equal(Object.getPrototypeOf(meta.subMetadata), null, `${label}: subMetadata prototype`);
    assert.ok(Object.keys(meta.entries).includes('__proto__'), `${label}: __proto__ in entries keys`);
    assert.deepEqual(meta.entries.__proto__, utf8('top-proto'), `${label}: entries.__proto__ bytes`);
    assert.equal(meta.strings.__proto__, 'top-proto', `${label}: strings.__proto__`);
    assert.ok(Object.keys(meta.subMetadata).includes('__proto__'), `${label}: __proto__ in subMetadata keys`);
    const child = meta.subMetadata.__proto__;
    assert.equal(Object.getPrototypeOf(child.entries), null, `${label}: child entries prototype`);
    assert.ok(Object.keys(child.entries).includes('__proto__'), `${label}: child __proto__ key`);
    assert.equal(child.strings.__proto__, 'child-proto', `${label}: child strings.__proto__`);
  };

  const parsed = drc.parse_drc_bytes(firstEncode.bytes);
  assert.ok(parsed.success, `parse_drc_bytes: ${parsed.error}`);
  checkMetadata(parsed.metadata, 'parse_drc_bytes');
  const parsedExtraMeta = parsed.meshes[0].extras[0].metadata;
  assert.equal(Object.getPrototypeOf(parsedExtraMeta.entries), null);
  assert.ok(Object.keys(parsedExtraMeta.entries).includes('__proto__'));
  assert.equal(parsedExtraMeta.strings.__proto__, 'attr-proto');

  const decoded = drc.decode_draco(firstEncode.bytes, undefined);
  assert.ok(decoded.success, `decode_draco: ${decoded.error}`);
  checkMetadata(decoded.metadata, 'decode_draco');
  const decodedExtra = decoded.attributes.find((a: any) => a.uniqueId === 10);
  assert.ok(decodedExtra?.metadata, 'decode_draco extra metadata missing');
  assert.equal(Object.getPrototypeOf(decodedExtra.metadata.entries), null);
  assert.equal(decodedExtra.metadata.strings.__proto__, 'attr-proto');

  // Re-encode directly from decoded output (null-prototype dicts must be accepted).
  const recoded = drc.encode_draco({
    name: 'proto-meta-recode',
    positions: parsed.meshes[0].positions,
    indices: parsed.meshes[0].indices,
    extras: parsed.meshes[0].extras,
    metadata: parsed.metadata,
  }, {});
  assert.ok(recoded.success, `recode with decoded __proto__ metadata: ${recoded.error}`);

  const reparsed = drc.parse_drc_bytes(recoded.bytes);
  assert.ok(reparsed.success, `reparse: ${reparsed.error}`);
  checkMetadata(reparsed.metadata, 'reparsed');
  assert.equal(reparsed.meshes[0].extras[0].metadata.strings.__proto__, 'attr-proto');

  // Also ensure a normal object with an own `__proto__` property works as input.
  const ownProtoEntries = Object.defineProperty({}, '__proto__', {
    value: utf8('own-proto'),
    enumerable: true,
  });
  const ownEncoded = drc.encode_draco({
    name: 'own-proto-meta',
    positions,
    indices,
    metadata: { entries: ownProtoEntries },
  }, {});
  assert.ok(ownEncoded.success, `encode with own __proto__ entry: ${ownEncoded.error}`);
  const ownDecoded = drc.decode_draco(ownEncoded.bytes, undefined);
  assert.ok(ownDecoded.success, `decode own __proto__: ${ownDecoded.error}`);
  assert.equal(ownDecoded.metadata.strings.__proto__, 'own-proto');
}

console.log('wasm bridge contract passed');
