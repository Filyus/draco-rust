/**
 * What the format modules cost in both directions, on a mesh big enough for the
 * cost to show.
 *
 * Committed rather than run once and quoted in a commit message, because a
 * number nobody can reproduce is not a measurement. It reports the median and
 * the spread beside the minimum on purpose: a best-of-N alone says nothing
 * about how noisy the run was, and this workspace builds with `lto = true` and
 * `codegen-units = 1`, where a rebuild can move a figure two or three percent
 * on code layout alone. A difference smaller than the spread printed here is
 * not a difference.
 *
 * Reading is measured against what this same run wrote, so the two halves
 * always describe the same geometry and no fixture has to be carried.
 *
 *   node tests/format-benchmark.ts [--runs 7] [--side 513]
 */
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

function flag(name: string, fallback: number) {
  const at = process.argv.indexOf(`--${name}`);
  return at === -1 ? fallback : Number(process.argv[at + 1]);
}

const runs = flag('runs', 7);
const side = flag('side', 513);

async function load(name: string) {
  const module = await import(pathToFileURL(resolve(pkg, `${name}.js`)).href);
  await module.default({ module_or_path: await readFile(resolve(pkg, `${name}_bg.wasm`)) });
  return module;
}

/**
 * A `side` x `side` grid of vertices with a full attribute set.
 *
 * Displaced rather than flat: a plane compresses to almost nothing, so a Draco
 * timing taken on one measures the encoder's best case rather than its usual.
 */
function grid(n: number) {
  const vertices = n * n;
  const positions = new Float32Array(vertices * 3);
  const normals = new Float32Array(vertices * 3);
  const uvs = new Float32Array(vertices * 2);
  const colors = new Uint8Array(vertices * 4);
  for (let y = 0; y < n; y += 1) {
    for (let x = 0; x < n; x += 1) {
      const at = y * n + x;
      const height = Math.sin(x * 0.11) * Math.cos(y * 0.13) * 4;
      positions.set([x * 0.05, height, y * 0.05], at * 3);
      normals.set([0, 1, 0], at * 3);
      uvs.set([x / (n - 1), y / (n - 1)], at * 2);
      colors.set([x & 0xff, y & 0xff, (x ^ y) & 0xff, 255], at * 4);
    }
  }
  const quads = (n - 1) * (n - 1);
  const indices = new Uint32Array(quads * 6);
  let at = 0;
  for (let y = 0; y < n - 1; y += 1) {
    for (let x = 0; x < n - 1; x += 1) {
      const corner = y * n + x;
      indices.set([corner, corner + 1, corner + n], at);
      indices.set([corner + 1, corner + n + 1, corner + n], at + 3);
      at += 6;
    }
  }
  return { name: 'grid', positions, indices, normals, uvs, colors };
}

function summarize(samples: number[]) {
  const sorted = [...samples].sort((a, b) => a - b);
  return {
    min: sorted[0],
    median: sorted[Math.floor(sorted.length / 2)],
    spread: sorted[sorted.length - 1] - sorted[0],
  };
}

/**
 * Time one call, discarding several warm-up runs.
 *
 * More than one because the first calls through a module pay for wasm
 * instantiation and first-touch page faults, which is not what is measured.
 *
 * Read the spread column, and do not compare across invocations. The spread
 * only describes variance *inside* one process, and it stays tight while the
 * level moves: the same build measured 163ms and 228ms for the same read in
 * two runs minutes apart, each with a spread under 11ms, because something
 * else on the machine was busy. So a figure is only comparable against one
 * taken back to back with it on an otherwise idle machine, and a difference
 * under roughly ten percent between separate runs is unproven either way.
 */
const WARMUP_RUNS = 3;

// Generic over what `call` returns rather than narrowing it to the two fields
// this function reads: the caller wants the payload back, and a fixed parameter
// type erases it, so `result.binary_data` below stops existing.
function measure<T extends { success?: boolean; error?: string }>(
  label: string,
  call: () => T,
  note = '',
): T | null {
  let warm = call();
  for (let run = 1; run < WARMUP_RUNS; run += 1) warm = call();
  if (warm && warm.success === false) {
    console.log(`${label.padEnd(10)} failed: ${warm.error}`);
    return null;
  }
  const samples: number[] = [];
  for (let run = 0; run < runs; run += 1) {
    const started = performance.now();
    call();
    samples.push(performance.now() - started);
  }
  const { min, median, spread } = summarize(samples);
  console.log(
    `${label.padEnd(10)} ${min.toFixed(1).padStart(7)}ms ${median.toFixed(1).padStart(7)}ms `
    + `${spread.toFixed(1).padStart(6)}ms   ${note}`,
  );
  return warm;
}

const [ply, stl, drc, fbx, obj] = await Promise.all(['ply', 'stl', 'drc', 'fbx', 'obj'].map(load));
const mesh = grid(side);
const vertices = side * side;
const triangles = (side - 1) * (side - 1) * 2;

const writers: [string, () => any][] = [
  ['ply', () => ply.create_ply(mesh, {
    include_normals: true,
    include_uvs: true,
    include_colors: true,
    precision: 6,
    format: 'binary_little_endian',
  })],
  ['stl', () => stl.create_stl(
    { positions: mesh.positions, indices: mesh.indices }, { format: 'binary', name: 'grid' },
  )],
  ['obj', () => obj.create_obj(mesh, { include_normals: true, include_uvs: true, precision: 6 })],
  ['fbx', () => fbx.create_fbx([mesh], { version: 7500, compression: true })],
  ['drc', () => drc.create_drc(mesh, {
    encoding_speed: 5,
    position_bits: 14,
    normal_bits: 10,
    texcoord_bits: 12,
    include_normals: true,
    include_uvs: true,
    include_colors: true,
  })],
];

console.log(`grid ${side}x${side}: ${vertices} vertices, ${triangles} triangles, best of ${runs}\n`);
console.log('writing    min       median   spread');
const written = new Map<string, Uint8Array>();
for (const [name, write] of writers) {
  const result = measure(name, write);
  if (!result) continue;
  const payload = result.binary_data
    ? new Uint8Array(result.binary_data)
    : new TextEncoder().encode(result.data);
  written.set(name, payload);
}

const readers: [string, (bytes: Uint8Array) => any][] = [
  ['ply', (bytes) => ply.parse_ply_bytes(bytes)],
  ['stl', (bytes) => stl.parse_stl_bytes(bytes)],
  ['obj', (bytes) => obj.parse_obj_bytes(bytes)],
  ['fbx', (bytes) => fbx.parse_fbx(bytes)],
  ['drc', (bytes) => drc.parse_drc_bytes(bytes)],
];

console.log('\nreading    min       median   spread    source');
for (const [name, read] of readers) {
  const bytes = written.get(name);
  if (!bytes) continue;
  measure(name, () => read(bytes), `${(bytes.length / 1024 / 1024).toFixed(1)} MB`);
}
