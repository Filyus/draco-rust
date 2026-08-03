/**
 * What the flat writers cost, on a mesh big enough for the cost to show.
 *
 * Committed rather than run once and quoted in a commit message, because a
 * number nobody can reproduce is not a measurement. It reports the median and
 * the spread beside the minimum on purpose: a best-of-N alone hides how noisy
 * the run was, and this workspace builds with `lto = true` and
 * `codegen-units = 1`, where a rebuild can move a figure two or three percent
 * on code layout alone. A difference smaller than the spread printed here is
 * not a difference.
 *
 *   node tests/export-benchmark.ts [--runs 7] [--side 513]
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
  const median = sorted[Math.floor(sorted.length / 2)];
  return { min: sorted[0], median, spread: sorted[sorted.length - 1] - sorted[0] };
}

const [ply, stl, drc, fbx, obj] = await Promise.all(['ply', 'stl', 'drc', 'fbx', 'obj'].map(load));
const mesh = grid(side);
const vertices = side * side;
const triangles = (side - 1) * (side - 1) * 2;

const cases: [string, () => { success: boolean; error?: string }][] = [
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
    encoding_speed: 5, position_bits: 14, normal_bits: 10, texcoord_bits: 12,
    include_normals: true, include_uvs: true, include_colors: true,
  })],
];

console.log(`grid ${side}x${side}: ${vertices} vertices, ${triangles} triangles, best of ${runs}\n`);
console.log('format     min      median   spread');
for (const [name, write] of cases) {
  // One warm-up outside the samples: the first call through a module pays for
  // wasm instantiation and first-touch page faults, which is not what is
  // being measured.
  const warm = write();
  if (!warm.success) {
    console.log(`${name.padEnd(10)} failed: ${warm.error}`);
    continue;
  }
  const samples: number[] = [];
  for (let run = 0; run < runs; run += 1) {
    const started = performance.now();
    const result = write();
    samples.push(performance.now() - started);
    if (!result.success) throw new Error(`${name}: ${result.error}`);
  }
  const { min, median, spread } = summarize(samples);
  console.log(
    `${name.padEnd(10)} ${min.toFixed(1).padStart(6)}ms ${median.toFixed(1).padStart(6)}ms `
    + `${spread.toFixed(1).padStart(6)}ms`,
  );
}
