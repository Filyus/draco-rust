/**
 * `@draco-rust/decoder/three` against three.js's own DRACOLoader, in GLTFLoader.
 *
 * Every Draco glTF fixture loads twice through three's GLTFLoader, once with
 * its DRACOLoader and once with `createDracoLoader(THREE)`, and every `.drc`
 * fixture goes through each loader's `parse`. The geometries must agree: the
 * index, and each attribute's name, array type, item size, normalization,
 * interleaving and bytes. The page is assembled from files on disk -- three
 * from node_modules, the converter's `drc` build as the entry's `index.js`,
 * and the adapter's modules from `web/npm/decoder/entry/` -- so it runs with
 * what CI builds and needs no `build-tool --npm`.
 */
import { expect, test } from '@playwright/test';
import { readdirSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const web = path.resolve(here, '..');
const testdata = path.resolve(web, '..', 'testdata');

const gltfFixtures = [
  'Box/glTF_Binary/Box_Draco.glb',
  'BoxMetaDraco/glTF/BoxMetaDraco.gltf',
  'bun_zipper.glb',
  'gltf_transform/sphere_edgebreaker_speed0.glb',
  'gltf_transform/sphere_sequential.glb',
  'gltf_transform/two_objects_edgebreaker_speed0.glb',
  'gltf_transform/two_objects_sequential.glb',
];
// The legacy non-manifold streams number their points differently from
// upstream (draco-attribute-parity.ts compares them corner by corner), so a
// geometry-for-geometry comparison leaves them out.
const drcFixtures = [
  ...readdirSync(testdata).filter((name) => name.endsWith('.drc') && !name.startsWith('test_nm')),
  ...readdirSync(path.join(testdata, 'production_draco'))
    .filter((name) => name.endsWith('.drc'))
    .map((name) => `production_draco/${name}`),
];

const files: Record<string, string> = {
  'decoder/index.js': path.join(web, 'www', 'pkg', 'drc.js'),
  'decoder/index_bg.wasm': path.join(web, 'www', 'pkg', 'drc_bg.wasm'),
  'decoder/pool.js': path.join(web, 'npm', 'decoder', 'entry', 'pool.js'),
  'decoder/worker.js': path.join(web, 'npm', 'decoder', 'entry', 'worker.js'),
  'decoder/three.js': path.join(web, 'npm', 'decoder', 'entry', 'three.js'),
};
const page = `<!doctype html><meta charset="utf-8">
<script type="importmap">
{ "imports": { "three": "/__t/three/build/three.module.js", "three/addons/": "/__t/three/examples/jsm/" } }
</script>`;

const types: Record<string, string> = {
  '.js': 'text/javascript',
  '.wasm': 'application/wasm',
  '.html': 'text/html',
  '.json': 'application/json',
};

test('createDracoLoader builds what DRACOLoader builds, in GLTFLoader and for .drc files', async ({ context, page: tab }) => {
  test.setTimeout(120_000);
  await context.route('**/__t/**', async (route) => {
    const at = new URL(route.request().url()).pathname.slice('/__t/'.length);
    let file: string | undefined;
    if (at === 'page.html') return route.fulfill({ body: page, contentType: 'text/html' });
    if (at.startsWith('three/')) file = path.join(web, 'node_modules', at);
    else if (at.startsWith('testdata/')) file = path.join(testdata, at.slice('testdata/'.length));
    else file = files[at];
    if (!file) return route.fulfill({ status: 404 });
    try {
      const body = await readFile(file);
      return route.fulfill({ body, contentType: types[path.extname(file)] ?? 'application/octet-stream' });
    } catch {
      return route.fulfill({ status: 404 });
    }
  });
  const errors: string[] = [];
  tab.on('pageerror', (error) => errors.push(String(error)));
  await tab.goto('/__t/page.html');

  const report = await tab.evaluate(
    async ({ gltfFixtures, drcFixtures }) => {
      const THREE: any = await import('three' as string);
      const { GLTFLoader }: any = await import('three/addons/loaders/GLTFLoader.js' as string);
      const { DRACOLoader }: any = await import('three/addons/loaders/DRACOLoader.js' as string);
      const { createDracoLoader }: any = await import('/__t/decoder/three.js' as string);

      const theirs = new DRACOLoader();
      const ours = createDracoLoader(THREE, { workers: 2 });
      const theirGltf = new GLTFLoader().setDRACOLoader(theirs);
      const ourGltf = new GLTFLoader().setDRACOLoader(ours);

      const bytesOf = (array: ArrayBufferView) => new Uint8Array(array.buffer, array.byteOffset, array.byteLength);
      const sameBytes = (a: ArrayBufferView, b: ArrayBufferView) => {
        const x = bytesOf(a);
        const y = bytesOf(b);
        return x.length === y.length && x.every((byte, i) => byte === y[i]);
      };
      const mismatches: string[] = [];
      const compare = (where: string, a: any, b: any) => {
        if (!!a.index !== !!b.index) mismatches.push(`${where}: index present on one side`);
        else if (a.index && !sameBytes(a.index.array, b.index.array)) mismatches.push(`${where}: index`);
        const names = Object.keys(a.attributes).sort();
        if (names.join() !== Object.keys(b.attributes).sort().join()) {
          mismatches.push(`${where}: attributes ${Object.keys(a.attributes)} against ${Object.keys(b.attributes)}`);
          return 0;
        }
        for (const name of names) {
          const x = a.attributes[name];
          const y = b.attributes[name];
          const shape = (v: any) =>
            [v.array.constructor.name, v.itemSize, v.normalized, !!v.isInterleavedBufferAttribute, v.data?.stride].join();
          if (shape(x) !== shape(y)) mismatches.push(`${where} ${name}: ${shape(x)} against ${shape(y)}`);
          else if (!sameBytes(x.array, y.array)) mismatches.push(`${where} ${name}: values`);
        }
        return names.length;
      };
      const geometries = (scene: any) => {
        const out: any[] = [];
        scene.traverse((object: any) => object.geometry && out.push(object.geometry));
        return out;
      };

      let attributes = 0;
      let interleaved = 0;
      for (const fixture of gltfFixtures) {
        const url = `/__t/testdata/${fixture}`;
        const [a, b] = await Promise.all([ourGltf.loadAsync(url), theirGltf.loadAsync(url)]);
        const mine = geometries(a.scene);
        const reference = geometries(b.scene);
        if (mine.length !== reference.length || mine.length === 0) {
          mismatches.push(`${fixture}: ${mine.length} geometries against ${reference.length}`);
          continue;
        }
        mine.forEach((geometry, i) => (attributes += compare(`${fixture} #${i}`, geometry, reference[i])));
      }
      let refused = 0;
      let unconvertible = 0;
      for (const fixture of drcFixtures) {
        const bytes = await (await fetch(`/__t/testdata/${fixture}`)).arrayBuffer();
        const settle = (loader: any) =>
          new Promise((resolve) => loader.parse(bytes.slice(0), resolve, (error: unknown) => resolve({ error })));
        const [a, b]: any[] = await Promise.all([settle(ours), settle(theirs)]);
        if ('error' in a || 'error' in b) {
          if (!('error' in a && 'error' in b)) {
            mismatches.push(`${fixture}: ${'error' in a ? 'ours' : 'DRACOLoader'} refused: ${a.error ?? b.error}`);
          }
          refused += 1;
          continue;
        }
        attributes += compare(fixture, a, b);

        // GLTFLoader's call, by unique id, in the narrow types a quantized
        // accessor asks for: where a three-component item is padded to four
        // bytes, and where a value the type cannot hold is refused.
        const listed = await ours.pool.decode(bytes.slice(0));
        const ids: Record<string, number> = {};
        for (const attribute of listed.attributes) {
          ids[attribute.semantic === 'COLOR' && !('color' in ids) ? 'color' : `a${attribute.uniqueId}`] ??= attribute.uniqueId;
        }
        for (const type of ['Uint8Array', 'Int16Array', 'Uint16Array']) {
          const typesOf = Object.fromEntries(Object.keys(ids).map((name) => [name, type]));
          const call = (loader: any) =>
            new Promise((resolve) =>
              loader.decodeDracoFile(bytes.slice(0), resolve, ids, typesOf, 'srgb', (error: unknown) => resolve({ error })),
            );
          const [x, y]: any[] = await Promise.all([call(ours), call(theirs)]);
          // Where upstream's conversion fails, DRACOLoader does not look at
          // the result and builds the attribute from whatever the failed
          // conversion left in memory; this loader refuses. That the two
          // refuse the same values is draco-attribute-parity.ts's subject.
          if ('error' in x && !('error' in y) && /does not convert to the requested type/.test(x.error)) {
            unconvertible += 1;
            continue;
          }
          if ('error' in x || 'error' in y) {
            if (!('error' in x && 'error' in y)) {
              mismatches.push(`${fixture} as ${type}: ${'error' in x ? 'ours' : 'DRACOLoader'} refused: ${x.error ?? y.error}`);
            }
            refused += 1;
            continue;
          }
          attributes += compare(`${fixture} as ${type}`, x, y);
          interleaved += Object.values(x.attributes).filter((v: any) => v.isInterleavedBufferAttribute).length;
        }
      }
      ours.dispose();
      theirs.dispose();
      return { mismatches, attributes, interleaved, refused, unconvertible };
    },
    { gltfFixtures, drcFixtures },
  );

  expect(errors).toEqual([]);
  expect(report.mismatches).toEqual([]);
  expect(report.attributes).toBeGreaterThan(100);
  expect(report.interleaved).toBeGreaterThan(0);
  console.log(
    `three-draco-loader: ${gltfFixtures.length} glTF files and ${drcFixtures.length} .drc files, ` +
      `${report.attributes} attributes the same as DRACOLoader's (${report.interleaved} padded), ` +
      `${report.refused} refused by both, ${report.unconvertible} unconvertible requests refused here and left ` +
      'to memory by DRACOLoader',
  );
});
