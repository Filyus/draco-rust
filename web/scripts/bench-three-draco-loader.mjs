/**
 * `@draco-rust/decoder/three` against three.js's own DRACOLoader, in headless
 * Chromium, both on four workers. Each sample is a fresh page loading its own
 * module; the two arms alternate in order, after one sacrificial page each.
 *
 *   cold         a new loader to the first bun_zipper.glb through GLTFLoader
 *   gltf         bun_zipper.glb through GLTFLoader again, warm
 *   edgebreaker  16 bunny_gltf.drc parses in flight at once
 *   sequential   16 bunny_cpp_standard.drc parses in flight at once
 *
 * It measures what `build-tool --npm` left in web/npm/dist, so build that
 * first. `control` runs this loader against itself, the harness's floor.
 *
 *   cargo run --manifest-path build-tool/Cargo.toml -- --npm --package decoder
 *   node scripts/bench-three-draco-loader.mjs [control]
 */
import { chromium } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const web = fileURLToPath(new URL('..', import.meta.url));
const testdata = path.join(web, '..', 'testdata');
const ORIGIN = 'http://bench.localhost:4999';
const types = { '.js': 'text/javascript', '.wasm': 'application/wasm', '.html': 'text/html' };
const page = `<!doctype html><meta charset="utf-8"><script type="importmap">
{ "imports": { "three": "/three/build/three.module.js", "three/addons/": "/three/examples/jsm/" } }
</script>`;

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext();
await context.route(`${ORIGIN}/**`, async (route) => {
  const at = new URL(route.request().url()).pathname.slice(1);
  if (at === 'page.html') return route.fulfill({ body: page, contentType: 'text/html' });
  const file = at.startsWith('three/')
    ? path.join(web, 'node_modules', at)
    : at.startsWith('decoder/')
      ? path.join(web, 'npm', 'dist', at)
      : path.join(testdata, at.slice('testdata/'.length));
  try {
    return route.fulfill({ body: await readFile(file), contentType: types[path.extname(file)] ?? 'application/octet-stream' });
  } catch {
    return route.fulfill({ status: 404 });
  }
});

// Runs in the page: builds the arm's loader and returns timings.
async function arm({ which, scenario, rounds }) {
  const THREE = await import('three');
  const { GLTFLoader } = await import('three/addons/loaders/GLTFLoader.js');
  const fetchBytes = async (p) => (await fetch(p)).arrayBuffer();
  const glb = await fetchBytes('/testdata/bun_zipper.glb');
  const drcs = {
    edgebreaker: await fetchBytes('/testdata/bunny_gltf.drc'),
    sequential: await fetchBytes('/testdata/bunny_cpp_standard.drc'),
  };
  const t0 = performance.now();
  let loader;
  if (which.startsWith('ours')) {
    const { createDracoLoader } = await import('/decoder/mesh/three.js');
    loader = createDracoLoader(THREE, { workers: 4 });
  } else {
    const { DRACOLoader } = await import('three/addons/loaders/DRACOLoader.js');
    loader = new DRACOLoader().setWorkerLimit(4);
  }
  const gltf = new GLTFLoader().setDRACOLoader(loader);
  const out = [];
  if (scenario === 'cold') {
    await gltf.parseAsync(glb.slice(0), '');
    out.push(performance.now() - t0);
  } else if (scenario === 'gltf') {
    await gltf.parseAsync(glb.slice(0), '');
    for (let i = 0; i < rounds; i++) {
      const t = performance.now();
      await gltf.parseAsync(glb.slice(0), '');
      out.push(performance.now() - t);
    }
  } else {
    const bytes = drcs[scenario];
    const parse = (b) => new Promise((res, rej) => loader.parse(b, res, rej));
    await Promise.all(Array.from({ length: 16 }, () => parse(bytes.slice(0))));
    for (let i = 0; i < rounds; i++) {
      const copies = Array.from({ length: 16 }, () => bytes.slice(0));
      const t = performance.now();
      await Promise.all(copies.map(parse));
      out.push(performance.now() - t);
    }
  }
  loader.dispose();
  return out;
}

const median = (xs) => [...xs].sort((a, b) => a - b)[xs.length >> 1];
const run = async (which, scenario, rounds) => {
  const tab = await context.newPage();
  await tab.goto(`${ORIGIN}/page.html`);
  const result = await tab.evaluate(arm, { which, scenario, rounds });
  await tab.close();
  return result;
};

const [A, B] = process.argv[2] === 'control' ? ['ours', 'ours2'] : ['ours', 'theirs'];
// A sacrificial run of each arm first, so neither pays the first-instance cost.
await run('ours', 'cold', 1);
await run('theirs', 'cold', 1);
for (const [scenario, outer, inner] of [['cold', 11, 1], ['gltf', 4, 9], ['edgebreaker', 4, 7], ['sequential', 4, 7]]) {
  const samples = { [A]: [], [B]: [] };
  for (let r = 0; r < outer; r++) {
    const order = r % 2 ? [A, B] : [B, A];
    for (const which of order) samples[which].push(...(await run(which, scenario, inner)));
  }
  const a = median(samples[A]);
  const b = median(samples[B]);
  console.log(`${scenario.padEnd(12)} ours ${a.toFixed(1).padStart(7)} ms   DRACOLoader ${b.toFixed(1).padStart(7)} ms   ${(b / a).toFixed(2)}x  (n=${samples[A].length}) ${A} vs ${B}`);
}
await browser.close();
