/**
 * `decode_draco` against upstream's decoder, attribute by attribute.
 *
 * three.js's `DRACOLoader` reads each attribute with upstream's
 * `GetAttributeByUniqueId` and `GetAttributeDataArrayForAllPoints`, asking for
 * the typed array the glTF accessor needs; the loader built on `decode_draco`
 * has to hand back the same bytes. For every `.drc` fixture upstream's
 * `draco3d` 1.5.7 decodes, every attribute, and every array type the loader
 * can ask for, this compares the two: the values byte for byte, and a refusal
 * where upstream's conversion returns false. The index and the stream's
 * geometry type are compared as well, and a request by semantic finds the
 * attribute upstream's `GetAttributeId` does.
 */
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import draco3d from 'draco3d';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');
const testdata = resolve(here, '..', '..', 'testdata');

const drc = await import(pathToFileURL(resolve(pkg, 'drc.js')).href);
await drc.default({ module_or_path: await readFile(resolve(pkg, 'drc_bg.wasm')) });

const upstream: any = await draco3d.createDecoderModule({});
const types = [
  ['Int8Array', Int8Array, upstream.DT_INT8],
  ['Uint8Array', Uint8Array, upstream.DT_UINT8],
  ['Int16Array', Int16Array, upstream.DT_INT16],
  ['Uint16Array', Uint16Array, upstream.DT_UINT16],
  ['Int32Array', Int32Array, upstream.DT_INT32],
  ['Uint32Array', Uint32Array, upstream.DT_UINT32],
  ['Float32Array', Float32Array, upstream.DT_FLOAT32],
] as const;
const semantics = ['POSITION', 'NORMAL', 'COLOR', 'TEX_COORD', 'GENERIC'] as const;

const bytesOf = (array: ArrayBufferView) => new Uint8Array(array.buffer, array.byteOffset, array.byteLength);

let files = 0;
const renumbered: string[] = [];
let compared = 0;
let refusals = 0;
function compareStream(name: string, data: Uint8Array) {
  const decoder = new upstream.Decoder();
  const isMesh = data[7] === 1;
  const geometry = isMesh ? new upstream.Mesh() : new upstream.PointCloud();
  const signed = new Int8Array(data.buffer, data.byteOffset, data.byteLength);
  const status = isMesh
    ? decoder.DecodeArrayToMesh(signed, signed.byteLength, geometry)
    : decoder.DecodeArrayToPointCloud(signed, signed.byteLength, geometry);
  try {
    if (!status.ok()) return; // A stream upstream refuses is another test's subject.
    const ours = drc.decode_draco(data, undefined);
    assert.ok(ours.success, `${name}: ${ours.error}`);
    assert.equal(ours.geometry, isMesh ? 'mesh' : 'point_cloud', name);
    files += 1;

    // Upstream's index, and whether ours numbers the points the same way. On
    // the legacy non-manifold test_nm streams it does not: the same triangles
    // reach the same values at every corner, through points numbered
    // differently. Those are compared corner by corner instead.
    let theirIndex: Uint32Array | null = null;
    let samePoints = true;
    if (isMesh) {
      const count = geometry.num_faces() * 3;
      const ptr = upstream._malloc(count * 4);
      decoder.GetTrianglesUInt32Array(geometry, count * 4, ptr);
      theirIndex = new Uint32Array(upstream.HEAPF32.buffer, ptr, count).slice();
      upstream._free(ptr);
      assert.equal(ours.index.length, count, `${name}: index length`);
      samePoints = ours.index.every((corner: number, i: number) => corner === theirIndex![i]);
      if (!samePoints) renumbered.push(name);
    } else {
      assert.equal(ours.index, null, `${name}: a point cloud has no index`);
    }
    // The values each triangle corner reaches, or each point's when the
    // numbering agrees.
    const reach = (array: ArrayBufferView & ArrayLike<number>, index: ArrayLike<number> | null, itemSize: number) => {
      if (samePoints || !index) return bytesOf(array);
      const width = (array as any).BYTES_PER_ELEMENT * itemSize;
      const out = new Uint8Array(index.length * width);
      const bytes = bytesOf(array);
      for (let corner = 0; corner < index.length; corner += 1) {
        out.set(bytes.subarray(index[corner] * width, (index[corner] + 1) * width), corner * width);
      }
      return out;
    };

    const ids = Array.from({ length: geometry.num_attributes() }, (_, i) => decoder.GetAttribute(geometry, i).unique_id());
    for (let i = 0; i < geometry.num_attributes(); i += 1) {
      const id = ids[i];
      // Streams before 1.3 give every attribute unique id 0. A lookup by id
      // then finds the first of them on both sides, which is what is checked
      // for the rest; their values are compared through the first.
      if (ids.indexOf(id) !== i) {
        const first = drc.decode_draco(data, [{ id }]);
        assert.equal(
          first.attributes[0].itemSize,
          decoder.GetAttributeByUniqueId(geometry, id).num_components(),
          `${name}: a shared id ${id} finds the first attribute that has it`,
        );
        continue;
      }
      const attribute = decoder.GetAttribute(geometry, i);
      const itemSize = attribute.num_components();
      for (const [typeName, TypedArray, dataType] of types) {
        const byteLength = geometry.num_points() * itemSize * TypedArray.BYTES_PER_ELEMENT;
        const ptr = upstream._malloc(byteLength);
        const ok = decoder.GetAttributeDataArrayForAllPoints(geometry, attribute, dataType, byteLength, ptr);
        const theirArray = new TypedArray(
          new Uint8Array(upstream.HEAPF32.buffer, ptr, byteLength).slice().buffer,
        );
        upstream._free(ptr);
        const result = drc.decode_draco(data, [{ name: 'a', id, type: typeName }]);
        const where = `${name} attribute ${id} as ${typeName}`;
        if (!ok) {
          assert.equal(result.success, false, `${where}: upstream refuses, ours decoded`);
          refusals += 1;
          continue;
        }
        assert.ok(result.success, `${where}: ${result.error}`);
        const [decoded] = result.attributes;
        assert.ok(decoded.array instanceof TypedArray, `${where}: array type`);
        assert.equal(decoded.itemSize, itemSize, `${where}: itemSize`);
        assert.equal(decoded.uniqueId, id, `${where}: uniqueId`);
        assert.equal(decoded.normalized, attribute.normalized(), `${where}: normalized`);
        assert.deepEqual(
          reach(decoded.array, ours.index, itemSize),
          reach(theirArray as any, theirIndex, itemSize),
          `${where}: values`,
        );
        compared += 1;
      }
    }

    for (const semantic of semantics) {
      const id = decoder.GetAttributeId(geometry, upstream[semantic]);
      const result = drc.decode_draco(data, [{ semantic, type: 'Float32Array' }]);
      if (id === -1) {
        assert.equal(result.attributes.length, 0, `${name}: no ${semantic}, so nothing returned`);
      } else if (result.success) {
        const theirId = decoder.GetAttribute(geometry, id).unique_id();
        assert.equal(result.attributes[0].uniqueId, theirId, `${name}: the first ${semantic}`);
      }
    }
  } finally {
    upstream.destroy(status);
    upstream.destroy(geometry);
    upstream.destroy(decoder);
  }
}

for (const name of (await readdir(testdata)).filter((name) => name.endsWith('.drc')).sort()) {
  compareStream(name, new Uint8Array(await readFile(resolve(testdata, name))));
}

// No fixture carries a normalized integer attribute, a negative one, or values
// at a type's edges, which is where the conversion rules differ. These streams
// do: one per stored type, normalized and not, written by upstream's encoder
// so neither side of the comparison wrote them.
const encoder: any = await draco3d.createEncoderModule({});
const stored = [
  ['AddInt8Attribute', Int8Array, [-128, -1, 0, 1, 64, 127]],
  ['AddUInt8Attribute', Uint8Array, [0, 1, 127, 128, 200, 255]],
  ['AddInt16Attribute', Int16Array, [-32768, -129, -1, 0, 255, 32767]],
  ['AddUInt16Attribute', Uint16Array, [0, 1, 255, 256, 40000, 65535]],
  // Upstream's encoder runs out of bounds on 32-bit values that span most of
  // their range, and refuses a uint32 above INT32_MAX (COMPATIBILITY.md), so
  // these stay where it encodes them -- still past what 8 and 16 bits hold.
  ['AddInt32Attribute', Int32Array, [-3000000, -70000, -1, 0, 70000, 3000000]],
  ['AddUInt32Attribute', Uint32Array, [0, 1, 65535, 65536, 70000, 3000000]],
  ['AddFloatAttribute', Float32Array, [-129.5, -0.25, 0, 0.5, 1, 1.5, 254.75, 3e9]],
] as const;
let synthetic = 0;
for (const [add, Stored, values] of stored) {
  for (const normalized of [false, true]) {
    const points = 6;
    const components = 2;
    const array = new (Stored as any)(points * components);
    for (let i = 0; i < array.length; i += 1) array[i] = values[i % values.length];
    const builder = new encoder.MeshBuilder();
    const mesh = new encoder.Mesh();
    const faces = new Uint32Array([0, 1, 2, 3, 4, 5]);
    builder.AddFacesToMesh(mesh, 2, faces);
    const position = new Float32Array(points * 3).map((_, i) => i);
    builder.AddFloatAttribute(mesh, encoder.POSITION, points, 3, position);
    const id = builder[add](mesh, encoder.GENERIC, points, components, array);
    builder.SetNormalizedFlagForAttribute(mesh, id, normalized);
    const enc = new encoder.Encoder();
    enc.SetEncodingMethod(encoder.MESH_SEQUENTIAL_ENCODING);
    const out = new encoder.DracoInt8Array();
    const length = enc.EncodeMeshToDracoBuffer(mesh, out);
    assert.ok(length > 0, `${add} normalized=${normalized}: upstream encoded nothing`);
    const data = new Uint8Array(length);
    for (let i = 0; i < length; i += 1) data[i] = out.GetValue(i);
    compareStream(`${Stored.name}${normalized ? ' normalized' : ''}`, data);
    synthetic += 1;
    for (const object of [out, enc, mesh, builder]) encoder.destroy(object);
  }
}
assert.equal(synthetic, 14);

// A request for an id the stream does not have is an error, not a silent hole.
const bunny = new Uint8Array(await readFile(resolve(testdata, 'bunny_cpp_standard.drc')));
const missing = drc.decode_draco(bunny, [{ name: 'x', id: 4242, type: 'Float32Array' }]);
assert.equal(missing.success, false);
assert.match(missing.error, /unique id 4242/);

assert.ok(files >= 20, `only ${files} fixtures decoded`);
// Which streams number their points differently is pinned, so a change in
// either direction shows up here.
assert.deepEqual(renumbered, [
  'test_nm.obj.edgebreaker.0.10.0.drc',
  'test_nm.obj.edgebreaker.0.9.1.drc',
  'test_nm.obj.edgebreaker.1.0.0.drc',
  'test_nm.obj.edgebreaker.1.1.0.drc',
  'test_nm_quant.0.9.0.drc',
]);
console.log(
  `draco-attribute-parity: ${compared} attribute arrays over ${files} streams (${synthetic} written for the edges) ` +
    'match upstream byte for byte, ' +
    `${refusals} refusals agree; ${renumbered.length} legacy streams compared per corner`,
);
