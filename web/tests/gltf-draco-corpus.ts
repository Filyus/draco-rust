/**
 * Which files in the corpus the Draco encoder will accept.
 *
 * The safety check that guards compression is whole-document: one extension
 * anywhere that has not been read and registered refuses the file entirely.
 * That is the right default — silently rewriting accessor indices inside JSON
 * nobody has interpreted produces a broken file rather than an honest error —
 * but it is only as good as the list of extensions someone has actually read.
 * With just the Draco handler registered it refused 31 of these 70 assets, 21
 * of them over extensions that describe how a surface is lit and name no
 * binary data at all.
 *
 * So this gate is not "compression works". It is the list of files that cannot
 * be compressed and the reason for each, in the same spirit as the corpus
 * parity gate: a new refusal and a quietly fixed one both fail, so the set only
 * moves when someone means it to.
 */
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');
const pkg = resolve(here, '..', 'www', 'pkg');
const corpusRoot = resolve(repoRoot, 'testdata');

/** Draco bitstreams, encoder timings and C++ reference output, not scenes. */
const SKIPPED_DIRECTORIES = new Set(['fuzz_regressions', 'speed', 'production_draco', 'reference_cpp']);

/**
 * Every file the encoder refuses, and why.
 *
 * Each value is a substring of the error, chosen to name the cause rather than
 * to match the whole message. Three causes appear:
 *
 * - the asset already carries Draco geometry, so there is nothing to encode;
 * - the primitive is not a triangle mesh, which this encoder does not accept;
 * - the file carries an extension that owns binary references nobody has
 *   taught the registry to follow. Those are findings, not settled behaviour.
 */
const KNOWN = new Map(Object.entries({
  'testdata/Box/glTF_Binary/Box_Draco.glb': 'primitive uses Draco compression',
  'testdata/BoxMetaDraco/glTF/BoxMetaDraco.gltf': 'primitive uses Draco compression',
  'testdata/bun_zipper.glb': 'primitive uses Draco compression',
  'testdata/SphereTwoMaterials/sphere_two_materials_mesh_and_point_cloud.gltf': 'only TRIANGLES',
  'testdata/SphereTwoMaterials/sphere_two_materials_point_cloud.gltf': 'only TRIANGLES',
  // Two compressions describing the same bytes. Import decodes meshopt into
  // the fallback buffers, but the compressed ranges stay in the document and
  // the writer rebases them, so a re-export comes out compressed again -- the
  // extension object is live, not stale. What it addresses is a range inside a
  // *buffer*, and the maps a handler is handed cover accessors and buffer
  // views. There is nothing to remap it with, so this refusal is correct until
  // meshopt is decompressed on the way in rather than carried through.
  'testdata/KhronosSampleModels/MeshoptCubeTest/glTF_Meshopt/MeshoptCubeTest.gltf': 'meshopt_compression',
  // KHR_animation_pointer addresses arbitrary JSON by pointer, and a Draco
  // pass rewrites accessors and buffer views under it. A pointer into
  // `/meshes/0/primitives/0/attributes/POSITION` would be left naming
  // something that moved, and this crate cannot tell that pointer from one
  // into a material factor without implementing the extension. Refusing the
  // whole file is the conservative answer and, for now, the right one.
  'testdata/KhronosSampleModels/AnimatedColorsCube/glTF_Binary/AnimatedColorsCube.glb': 'animation_pointer',
}));

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

if (typeof gltfModule.GltfAsset?.prototype?.compressPrimitive !== 'function') {
  console.log('gltf-draco-corpus: SKIPPED (this WASM profile has no Draco encoder)');
  process.exit(0);
}

async function collect(directory: string): Promise<string[]> {
  const found: string[] = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) {
      if (!SKIPPED_DIRECTORIES.has(entry.name)) found.push(...await collect(path));
    } else if (/\.(gltf|glb)$/i.test(entry.name)) {
      found.push(path);
    }
  }
  return found.sort();
}

/** The companion buffers and images a .gltf names, as the browser would supply them. */
async function companions(model: string, data: Uint8Array): Promise<Record<string, Uint8Array>> {
  const resources: Record<string, Uint8Array> = Object.create(null);
  if (!model.toLowerCase().endsWith('.gltf')) return resources;
  let manifest: any;
  try {
    manifest = JSON.parse(new TextDecoder().decode(data));
  } catch {
    return resources;
  }
  for (const entry of [...(manifest.buffers || []), ...(manifest.images || [])]) {
    if (typeof entry.uri !== 'string' || entry.uri.startsWith('data:')) continue;
    try {
      resources[entry.uri] = new Uint8Array(await readFile(resolve(dirname(model), decodeURIComponent(entry.uri))));
    } catch {
      // A missing companion is the file's own problem, and the reader reports it.
    }
  }
  return resources;
}

/** Compress every primitive and package the result, exactly as the app does. */
function compress(data: Uint8Array, resources: Record<string, Uint8Array>) {
  const asset = gltfModule.GltfAsset.withResources(data, resources, '2.1');
  try {
    let primitives = 0;
    for (let mesh = 0; mesh < asset.meshCount(); mesh += 1) {
      const count = asset.primitiveCount(mesh);
      for (let primitive = 0; primitive < count; primitive += 1) {
        asset.compressPrimitive(mesh, primitive, 5, 5);
        primitives += 1;
      }
    }
    return { bytes: asset.glb(2).length, primitives };
  } finally {
    asset.free();
  }
}

const models = await collect(corpusRoot);
const refused = new Map<string, string>();
let compressed = 0;

for (const model of models) {
  const name = relative(repoRoot, model).replace(/\\/g, '/');
  const data = new Uint8Array(await readFile(model));
  try {
    const result = compress(data, await companions(model, data));
    assert.ok(result.bytes > 0, `${name} produced an empty GLB`);
    compressed += 1;
  } catch (error: any) {
    refused.set(name, String(error?.message ?? error));
  }
}

assert.ok(models.length > 0, 'the corpus must not be empty');

// Named files first: a count alone says how many, never which, and "which" is
// the part that carries a finding.
const unexpected = [...refused.keys()].filter((name) => !KNOWN.has(name));
assert.deepEqual(unexpected, [], `these files stopped compressing:\n  ${unexpected.map((name) => `${name}: ${refused.get(name)}`).join('\n  ')}`);

const fixed = [...KNOWN.keys()].filter((name) => !refused.has(name));
assert.deepEqual(fixed, [], `these files now compress; remove them from KNOWN:\n  ${fixed.join('\n  ')}`);

for (const [name, reason] of KNOWN) {
  assert.ok(
    refused.get(name)!.includes(reason),
    `${name} was refused for a different reason than recorded: ${refused.get(name)}`,
  );
}

assert.equal(compressed + refused.size, models.length);
console.log(`gltf-draco-corpus: OK (${compressed}/${models.length} compressed, ${refused.size} refused for recorded reasons)`);
