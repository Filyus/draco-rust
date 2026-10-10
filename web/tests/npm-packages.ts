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
  writeFileSync(join(project, 'package.json'), JSON.stringify({ name: 'consumer', private: true, type: 'module' }));
  const tarballs = readdirSync(project).filter((file) => file.endsWith('.tgz'));
  assert.equal(tarballs.length, packages.length);
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

const { create_drc } = await load('@draco-rust/encoder');
const quad = { positions: new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 0]), indices: new Uint32Array([0, 1, 2, 2, 1, 3]) };
const encoded = create_drc(quad, { position_bits: 14 });
assert.ok(encoded.success, encoded.error);
const { parse_drc_bytes } = await import('@draco-rust/decoder/mesh');
const decoded = parse_drc_bytes(encoded.binary_data);
assert.ok(decoded.success, decoded.error);
assert.equal(decoded.meshes[0].indices.length, 6);

const glb = await fixture('bun_zipper.glb');
for (const spec of ['@draco-rust/gltf', '@draco-rust/gltf/validate', '@draco-rust/gltf/writer']) {
  const { GltfAsset } = await load(spec);
  const asset = new GltfAsset(glb, '2.0');
  assert.equal(asset.meshCount(), 1);
  const geometry = asset.readPrimitive(0, 0);
  assert.ok(geometry.attributeCount() > 0 && geometry.indexBytes().length > 0, spec);
  assert.ok(asset.glb(2).length > 0, spec);
}

const fbx = await load('@draco-rust/fbx');
const written = fbx.create_fbx([quad], {});
assert.ok(written.success, written.error);
const read = fbx.parse_fbx(written.binary_data);
assert.ok(read.success, read.error);
assert.equal(read.meshes.length, 1);

// Each format reads a file of its own, then writes what it read and reads that
// back with the same positions.
const formats = [
  ['obj', 'parse_obj_bytes', 'create_obj', 'test_cube_shared.obj', true],
  ['ply', 'parse_ply_bytes', 'create_ply', 'bun_zipper.ply', true],
  ['ply', 'parse_ply_bytes', 'create_ply', 'point_cloud_pos.ply', false],
  ['stl', 'parse_stl_bytes', 'create_stl', 'STL/test_sphere_ascii.stl', true],
  ['stl', 'parse_stl_bytes', 'create_stl', 'STL/bunny.stl', true],
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
  // OBJ comes back as text in data, binary PLY and STL in binary_data.
  const output = written.binary_data ?? new TextEncoder().encode(written.data);
  const again = module[parse](output);
  assert.ok(again.success, name + ' ' + file + ' reread: ' + again.error);
  assert.equal(again.meshes[0].positions.length, mesh.positions.length, name + ' ' + file + ' round trip');
}
console.log('npm-packages: OK (4 decoder entries, encoder, 3 glTF entries, FBX, OBJ, PLY, STL)');
`);
  process.stdout.write(execFileSync(process.execPath, ['consumer.mjs'], { cwd: project }).toString());
} finally {
  rmSync(project, { recursive: true, force: true });
}
