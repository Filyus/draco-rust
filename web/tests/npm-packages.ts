/**
 * The `@draco-rust/*` packages as a consumer gets them.
 *
 * Packs what `build-tool --npm` left in `web/npm/dist`, installs the tarballs
 * into a scratch project, and imports every entry through the package's own
 * `exports` map: each decoder entry decodes what it claims and refuses what it
 * leaves out, the encoder's output decodes back, each glTF entry reads a Draco
 * GLB and writes it out again, and FBX round-trips a mesh.
 *
 *   cargo run --manifest-path build-tool/Cargo.toml -- --npm
 *   npm run test:npm-packages
 */
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const dist = fileURLToPath(new URL('../npm/dist/', import.meta.url));
const testdata = fileURLToPath(new URL('../../testdata/', import.meta.url));
const packages = ['decoder', 'encoder', 'gltf', 'fbx', 'obj', 'ply', 'stl'];
const version = readFileSync(fileURLToPath(new URL('../npm/VERSION', import.meta.url)), 'utf8').trim();
for (const name of packages) {
  const manifest = join(dist, name, 'package.json');
  assert.ok(existsSync(manifest), `web/npm/dist/${name} is missing; build it with build-tool --npm`);
  // One version across the packages, and each says what it was built from.
  const { version: built, 'draco-rust': from } = JSON.parse(readFileSync(manifest, 'utf8'));
  assert.equal(built, version, `@draco-rust/${name} is ${built}, web/npm/VERSION says ${version}`);
  assert.match(from.commit, /^[0-9a-f]{8}(-dirty)?$/);
  for (const crate of ['draco-core', 'draco-io', 'draco-gltf']) assert.match(from[crate], /^\d+\.\d+\.\d+/);
}

const project = mkdtempSync(join(tmpdir(), 'draco-npm-'));
// Under `npm run`, npm names its own entry point, which runs without a shell.
const npmCli = process.env.npm_execpath;
const run = (args: string[], cwd: string) =>
  (npmCli
    ? execFileSync(process.execPath, [npmCli, ...args], { cwd, stdio: ['ignore', 'pipe', 'inherit'] })
    : execFileSync('npm', args, { cwd, stdio: ['ignore', 'pipe', 'inherit'], shell: process.platform === 'win32' })
  ).toString();
try {
  for (const name of packages) run(['pack', '--silent', '--pack-destination', project], join(dist, name));
  // three, the decoder's optional peer, from this repository's own install.
  run(['pack', '--silent', '--pack-destination', project], fileURLToPath(new URL('../node_modules/three/', import.meta.url)));
  writeFileSync(join(project, 'package.json'), JSON.stringify({ name: 'consumer', private: true, type: 'module' }));
  const tarballs = readdirSync(project).filter((file) => file.endsWith('.tgz'));
  assert.equal(tarballs.length, packages.length + 1);
  run(['install', '--silent', '--no-audit', '--no-fund', '--offline', ...tarballs.map((file) => `./${file}`)], project);

  writeFileSync(join(project, 'consumer.mjs'), `
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const testdata = ${JSON.stringify(testdata)};
const fixture = async (name) => new Uint8Array(await readFile(testdata + name));
const load = async (spec) => {
  const module = await import(spec);
  const wasm = spec.replace(/^(@draco-rust\\/[a-z]+)(\\/.*)?$/, (_, pkg, sub) => pkg + (sub ?? '') + '/index_bg.wasm');
  await module.default({ module_or_path: await readFile(new URL(import.meta.resolve(wasm))) });
  return module;
};

const streams = {
  mesh: await fixture('bunny_cpp_standard.drc'),
  cloud: await fixture('pc_color.drc'),
  legacy: await fixture('test_nm.obj.edgebreaker.1.0.0.drc'),
};
const decoders = {
  '@draco-rust/decoder': { mesh: true, cloud: true, legacy: false },
  '@draco-rust/decoder/mesh': { mesh: true, cloud: false, legacy: false },
  '@draco-rust/decoder/point-cloud': { mesh: false, cloud: true, legacy: false },
  '@draco-rust/decoder/legacy': { mesh: true, cloud: true, legacy: true },
};
for (const [spec, decodes] of Object.entries(decoders)) {
  const { parse_drc_bytes } = await load(spec);
  for (const [kind, bytes] of Object.entries(streams)) {
    const result = parse_drc_bytes(bytes);
    assert.equal(result.success, decodes[kind], spec + ' on a ' + kind + ' stream: ' + result.error);
    if (!result.success) {
      assert.ok(result.error, spec + ' refused a ' + kind + ' stream without saying why');
      continue;
    }
    const { positions, indices } = result.meshes[0];
    assert.ok(positions instanceof Float32Array && positions.length > 0);
    assert.ok(indices instanceof Uint32Array);
    assert.equal(indices.length > 0, kind !== 'cloud', spec + ' ' + kind + ' faces');
  }
}

{
  const { parse_drc_bytes } = await load('@draco-rust/decoder');
  const overLimit = parse_drc_bytes(streams.cloud, { max_points: 1 });
  assert.equal(overLimit.success, false, 'parse_drc_bytes enforces a caller point ceiling');
  assert.match(overLimit.error, /limit|points/i);
}

const { encode_draco } = await load('@draco-rust/encoder');
const quad = { positions: new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 0]), indices: new Uint32Array([0, 1, 2, 2, 1, 3]) };
const encoded = encode_draco(quad, { position_bits: 14 });
assert.ok(encoded.success, encoded.error);
const { parse_drc_bytes } = await import('@draco-rust/decoder/mesh');
const decoded = parse_drc_bytes(encoded.bytes);
assert.ok(decoded.success, decoded.error);
assert.equal(decoded.meshes[0].indices.length, 6);

const colors = new Uint8Array([
  0, 32, 64, 255,
  128, 160, 192, 255,
  255, 224, 192, 255,
  32, 255, 128, 255,
]);
const colorEncoded = encode_draco({ ...quad, colors }, { color_bits: 2 });
assert.ok(colorEncoded.success, colorEncoded.error);
const { decode_draco: decodeDracoForColor } = await load('@draco-rust/decoder');
const colorDecoded = decodeDracoForColor(colorEncoded.bytes, [{ semantic: 'COLOR' }]);
assert.ok(colorDecoded.success, colorDecoded.error);
const decodedColors = colorDecoded.attributes[0].array;
assert.ok(decodedColors instanceof Float32Array, 'color_bits must emit a quantized float attribute');
const colorMinima = [0, 32 / 255, 64 / 255, 1];
for (let i = 0; i < decodedColors.length; i += 1) {
  const steps = (decodedColors[i] - colorMinima[i % 4]) * 3;
  assert.ok(Math.abs(steps - Math.round(steps)) < 1e-4, 'color was not quantized to two bits');
}
const byteColors = encode_draco({ ...quad, colors }, { color_bits: 0 });
assert.ok(byteColors.success, byteColors.error);
const byteColorDecode = decodeDracoForColor(byteColors.bytes, [{ semantic: 'COLOR' }]);
assert.ok(byteColorDecode.success, byteColorDecode.error);
assert.ok(byteColorDecode.attributes[0].array instanceof Uint8Array, 'zero color_bits must keep byte colors');
const invalidColorBits = encode_draco({ ...quad, colors }, { color_bits: -1 });
assert.equal(invalidColorBits.success, false, 'negative color_bits must be rejected');
assert.match(invalidColorBits.error, /color_bits/);

const glb = await fixture('bun_zipper.glb');
{
  const { GltfAsset, PackedGeometry, GeometryWriteOptions } = await load('@draco-rust/gltf/writer');
  const positions = new Uint8Array(new Float32Array([
    0, 0, 0,
    1, 0, 0,
    0, 1, 0,
  ]).buffer);
  const indices = new Uint8Array(new Uint16Array([0, 1, 2]).buffer);
  const geometry = new PackedGeometry(4);
  geometry.addAttribute('POSITION', 3, 3, 5126, false, positions);
  geometry.setIndices(3, 5123, indices);
  const asset = GltfAsset.fromGeometry(geometry, '2.0', new GeometryWriteOptions());
  asset.compressPrimitive(0, 0, 5, 5, 14, 0, 0, 0, 0, 0, {
    position_quantization_grid: { origin: [0, 0, 0], range: 1 },
  });
  const compressed = asset.glb(2);

  for (const spec of ['@draco-rust/gltf', '@draco-rust/gltf/writer']) {
    const { GltfAsset: Reader } = await load(spec);
    const limited = new Reader(compressed, '2.0', { max_faces: 0 });
    assert.throws(() => limited.readPrimitive(0, 0), /limit|points/i, spec + ' decode ceiling');
    const withResources = Reader.withResources(compressed, {}, '2.0', { max_faces: 0 });
    assert.throws(() => withResources.readPrimitive(0, 0), /limit|points/i, spec + ' resource decode ceiling');
  }

  const pool = (await import('@draco-rust/gltf/pool')).createPool({ workers: 1 });
  await assert.rejects(
    pool.read(compressed, { decode_options: { max_faces: 0 } }),
    /limit|points/i,
    'glTF worker forwards decode ceilings',
  );
  pool.dispose();
}
for (const spec of ['@draco-rust/gltf', '@draco-rust/gltf/validate', '@draco-rust/gltf/writer']) {
  const { GltfAsset } = await load(spec);
  const asset = new GltfAsset(glb, '2.0');
  assert.equal(asset.meshCount(), 1);
  const geometry = asset.readPrimitive(0, 0);
  assert.ok(geometry.attributeCount() > 0 && geometry.indexBytes().length > 0, spec);
  assert.ok(asset.glb(2).length > 0, spec);
  // A JSON Pointer finds what the object accessor finds, and nothing past it.
  const text = (bytes) => new TextDecoder().decode(bytes);
  assert.deepEqual(JSON.parse(text(asset.jsonAt('/meshes/0'))), JSON.parse(text(asset.objectJson('meshes', 0))), spec);
  assert.equal(JSON.parse(text(asset.jsonAt('/asset/version'))), '2.0', spec);
  assert.equal(asset.jsonAt('/meshes/1'), undefined, spec);
  assert.equal(asset.jsonAt('meshes'), undefined, spec + ': a pointer starts with /');
}

const fbx = await load('@draco-rust/fbx');
const written = fbx.write_fbx([quad], {});
assert.ok(written.success, written.error);
const read = fbx.read_fbx(written.bytes);
assert.ok(read.success, read.error);
assert.equal(read.meshes.length, 1);

// Each format reads a file of its own, then writes what it read and reads that
// back with the same positions.
const formats = [
  ['obj', 'read_obj', 'write_obj', 'test_cube_shared.obj', true],
  ['ply', 'read_ply', 'write_ply', 'bun_zipper.ply', true],
  ['ply', 'read_ply', 'write_ply', 'point_cloud_pos.ply', false],
  ['stl', 'read_stl', 'write_stl', 'STL/test_sphere_ascii.stl', true],
  ['stl', 'read_stl', 'write_stl', 'STL/bunny.stl', true],
];
for (const [name, parse, create, file, faces] of formats) {
  const module = await load('@draco-rust/' + name);
  const first = module[parse](await fixture(file));
  assert.ok(first.success, name + ' ' + file + ': ' + first.error);
  const mesh = first.meshes[0];
  assert.ok(mesh.positions.length > 0, name + ' ' + file);
  assert.equal(mesh.indices.length > 0, faces, name + ' ' + file + ' faces');
  const written = module[create](mesh, {});
  assert.ok(written.success, name + ' ' + file + ' write: ' + written.error);
  // Every writer returns the file as bytes; OBJ has it as text too.
  const output = written.bytes;
  const again = module[parse](output);
  assert.ok(again.success, name + ' ' + file + ' reread: ' + again.error);
  assert.equal(again.meshes[0].positions.length, mesh.positions.length, name + ' ' + file + ' round trip');

  // The package's pool gives a worker's answer, the same as the call here.
  const { createPool } = await import('@draco-rust/' + name + '/pool');
  const pool = createPool({ workers: 2 });
  const [parsedThere, writtenThere] = await Promise.all([pool.read(await fixture(file)), pool.write(mesh, {})]);
  assert.deepEqual(parsedThere, first, name + ' ' + file + ': pool read');
  assert.deepEqual(writtenThere, written, name + ' ' + file + ': pool write');
  pool.dispose();
}

// The other pools: FBX both ways, the encoder, and glTF read whole on a worker.
{
  const fbxPool = (await import('@draco-rust/fbx/pool')).createPool({ workers: 1 });
  const writtenThere = await fbxPool.write([quad], {});
  assert.deepEqual(writtenThere, written, 'fbx pool write');
  assert.deepEqual(await fbxPool.read(written.bytes), read, 'fbx pool read');
  const controller = new AbortController();
  const abandoned = fbxPool.read(written.bytes, { signal: controller.signal });
  controller.abort();
  await assert.rejects(abandoned, { name: 'AbortError' });
  fbxPool.dispose();

  const encoderPool = (await import('@draco-rust/encoder/pool')).createPool({ workers: 1 });
  const encodedThere = await encoderPool.encode(quad, { position_bits: 14 });
  assert.deepEqual(encodedThere, encoded, 'encoder pool');
  encoderPool.dispose();

  for (const spec of ['@draco-rust/gltf', '@draco-rust/gltf/validate', '@draco-rust/gltf/writer']) {
    const { GltfAsset } = await load(spec);
    const gltfPool = (await import(spec + '/pool')).createPool({ workers: 1 });
    const there = await gltfPool.read(glb);
    const asset = new GltfAsset(glb, '2.0');
    assert.deepEqual(there.json, asset.json(), spec + ' pool json');
    assert.equal(there.meshes.length, asset.meshCount());
    const geometry = asset.readPrimitive(0, 0);
    const [primitive] = there.meshes[0];
    assert.equal(primitive.attributes.length, geometry.attributeCount());
    primitive.attributes.forEach((attribute, i) => {
      assert.equal(attribute.semantic, geometry.attributeSemantic(i));
      assert.deepEqual(attribute.bytes, geometry.attributeBytes(i), spec + ' ' + attribute.semantic);
    });
    assert.deepEqual(primitive.index.bytes, geometry.indexBytes(), spec + ' pool index');
    gltfPool.dispose();
  }
}
// Each decoder entry's pool decodes on worker threads what the entry decodes
// here, byte for byte, with several streams in flight at once.
const same = (a, b, what) => {
  assert.equal(a.success, b.success, what);
  assert.equal(a.error, b.error, what);
  assert.deepEqual(a.index, b.index, what + ' index');
  assert.equal(a.attributes.length, b.attributes.length, what);
  a.attributes.forEach((attribute, i) => {
    assert.equal(attribute.array.constructor, b.attributes[i].array.constructor, what);
    assert.deepEqual(attribute.array, b.attributes[i].array, what + ' ' + attribute.name);
  });
};
const requests = [
  { name: 'position', semantic: 'POSITION', type: 'Float32Array' },
  { name: 'color', semantic: 'COLOR', type: 'Uint8Array' },
];
for (const spec of Object.keys(decoders)) {
  const { decode_draco } = await load(spec);
  const { createDecoderPool } = await import(spec.replace(/^(@draco-rust\\/decoder)(\\/.*)?$/, '$1$2/pool'));
  const pool = createDecoderPool({ workers: 2 });
  const inputs = [streams.mesh, streams.cloud, streams.legacy, streams.mesh, streams.cloud, streams.mesh];
  const results = await Promise.all(inputs.map((bytes) => pool.decode(bytes, requests)));
  inputs.forEach((bytes, i) => same(results[i], decode_draco(bytes, requests), spec + ' pool, stream ' + i));
  assert.equal(results.filter((result) => result.success).length > 0, true, spec);
  // Without workers the pool decodes here, to the same answer.
  const local = createDecoderPool({ workers: 0 });
  same(await local.decode(streams.mesh), decode_draco(streams.mesh, undefined), spec + ' on this thread');
  pool.dispose();
  local.dispose();
}

{
  const { decode_draco } = await load('@draco-rust/decoder');
  const overLimit = decode_draco(streams.cloud, undefined, { max_points: 1 });
  assert.equal(overLimit.success, false, 'decode_draco enforces a caller point ceiling');
  assert.match(overLimit.error, /limit|points/i);
}
{
  const { createDecoderPool } = await import('@draco-rust/decoder/pool');
  const pool = createDecoderPool({ workers: 1 });
  const copy = streams.mesh.slice();
  assert.ok((await pool.decode(copy)).success);
  assert.equal(copy.byteLength, streams.mesh.byteLength, 'a decode copies the stream unless told otherwise');
  assert.ok((await pool.decode(copy, undefined, { transfer: true })).success);
  assert.equal(copy.byteLength, 0, 'transfer hands the stream to the worker');

  // index: 'auto' reaches the worker: the bunny's points fit sixteen bits.
  const wide = await pool.decode(streams.mesh);
  const narrow = await pool.decode(streams.mesh, undefined, { index: 'auto' });
  assert.ok(wide.index instanceof Uint32Array && narrow.index instanceof Uint16Array);
  assert.deepEqual(Array.from(narrow.index), Array.from(wide.index));
  const limited = await pool.decode(streams.cloud, undefined, { max_points: 1 });
  assert.equal(limited.success, false, 'worker decode forwards caller ceilings');

  // An abort settles the caller at once; the worker's result is dropped and
  // the pool goes on decoding.
  const before = new AbortController();
  before.abort();
  await assert.rejects(pool.decode(streams.mesh, undefined, { signal: before.signal }), { name: 'AbortError' });
  const during = new AbortController();
  const abandoned = pool.decode(streams.mesh, undefined, { signal: during.signal });
  const queued = pool.decode(streams.mesh);
  during.abort();
  await assert.rejects(abandoned, { name: 'AbortError' });
  assert.ok((await queued).success, 'the decode behind an abandoned one completes');
  const reason = new Error('no longer needed');
  const custom = new AbortController();
  const withReason = pool.decode(streams.mesh, undefined, { signal: custom.signal });
  custom.abort(reason);
  await assert.rejects(withReason, (error) => error === reason);

  const pending = pool.decode(streams.mesh);
  pool.dispose();
  await assert.rejects(pending, /disposed/);
  await assert.rejects(pool.decode(streams.mesh), /disposed/);
}

// The three.js loader is three's own DRACOLoader, decoding on the pool, in Node
// as in a browser: the same class GLTFLoader takes, with three as the
// application's peer dependency.
const { REVISION, BufferGeometry, InterleavedBufferAttribute } = await import('three');
const { DRACOLoader: ThreeDRACOLoader } = await import('three/addons/loaders/DRACOLoader.js');
const { GLTFLoader } = await import('three/addons/loaders/GLTFLoader.js');
const { DRACOLoader } = await import('@draco-rust/decoder/three');
const { decode_draco } = await load('@draco-rust/decoder');
const loader = new DRACOLoader(undefined, { workers: 2 }).preload();
assert.ok(loader instanceof ThreeDRACOLoader, "it is three's DRACOLoader");

// A .drc file, read by attribute type.
const cloud = await new Promise((resolve, reject) => loader.parse(streams.cloud.slice().buffer, resolve, reject));
assert.ok(cloud instanceof BufferGeometry);
const reference = decode_draco(streams.cloud, [{ semantic: 'POSITION', type: 'Float32Array' }]);
assert.deepEqual(cloud.attributes.position.array, reference.attributes[0].array);
assert.equal(cloud.index, null, 'a point cloud has no index');

const limitedLoader = new DRACOLoader(undefined, {
  workers: 1,
  decode_options: { max_points: 1 },
});
await assert.rejects(new Promise((resolve, reject) =>
  limitedLoader.parse(streams.cloud.slice().buffer, resolve, reject),
), /limit|points/i);
limitedLoader.dispose();

// GLTFLoader's call: attributes by unique id, in the accessor's types. A
// three-byte colour is padded to four from r181, as three's own does it.
const ids = Object.fromEntries(decode_draco(streams.cloud, undefined).attributes.map((a) => [a.semantic === 'COLOR' ? 'color' : 'position', a.uniqueId]));
const buffer = streams.cloud.slice().buffer;
const types = { position: 'Float32Array', color: 'Uint8Array' };
const viaIds = await new Promise((resolve, reject) => loader.decodeDracoFile(buffer, resolve, ids, types, 'srgb-linear', reject));
assert.ok(viaIds.attributes.color.array instanceof Uint8Array);
assert.equal(viaIds.attributes.color.normalized, true, 'integer colours are normalized');
assert.equal(viaIds.attributes.color instanceof InterleavedBufferAttribute, Number(REVISION) >= 181);
assert.equal(buffer.byteLength, 0, 'the buffer went to the worker');
// A shared primitive asks again with the same buffer and gets the same geometry.
const again = await new Promise((resolve, reject) => loader.decodeDracoFile(buffer, resolve, ids, types, 'srgb-linear', reject));
assert.equal(again, viaIds);

// A whole Draco glTF, through GLTFLoader.
const gltf = await new GLTFLoader().setDRACOLoader(loader).parseAsync((await fixture('bun_zipper.glb')).buffer, '');
let triangles = 0;
gltf.scene.traverse((object) => { if (object.geometry) triangles += object.geometry.index.count / 3; });
assert.equal(triangles, 69451);

const failed = await new Promise((resolve) => loader.decodeDracoFile(new ArrayBuffer(3), () => resolve(null), { position: 0 }, null, undefined, resolve));
assert.ok(failed instanceof Error, 'a stream that does not decode reaches onError');
loader.dispose();

console.log('npm-packages: OK (4 decoder entries, three.js adapter, encoder, 3 glTF entries, FBX, OBJ, PLY, STL, each with its worker pool)');
`);
  process.stdout.write(execFileSync(process.execPath, ['consumer.mjs'], { cwd: project }).toString());
} finally {
  rmSync(project, { recursive: true, force: true });
}
