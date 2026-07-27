/**
 * Every glTF in the corpus, through both load paths.
 *
 * The preview builds its scene straight from the asset (`gltf-loader.ts`); the
 * portable SceneDocument reaches the same renderer through
 * `scene-document-viewer.ts`. That second path is fully written and covered by
 * hand-built fixtures, but no application route uses it, so nobody knows how
 * much of a real corpus it can carry. This measures exactly that: which files
 * the document cannot be built from, which ones fail strict validation, and
 * where the two adapters disagree about what the file contains.
 *
 * It does not demand that the list be empty. It demands that the list be
 * known: a new failure and a fixed one both fail the gate, so the count only
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

/**
 * Directories that hold Draco bitstreams, encoder timings and C++ reference
 * output rather than scenes. Skipped by name so the count below stays a count
 * of assets someone might actually open.
 */
const SKIPPED_DIRECTORIES = new Set(['fuzz_regressions', 'speed', 'production_draco', 'reference_cpp']);

/**
 * What each known-imperfect file does, one line each.
 *
 * These are findings, not accepted behaviour. Anything here is a file that
 * does not survive the route today, and the value says why.
 */
const KNOWN = new Map(Object.entries({
  // KHR_meshopt_compression is a newer extension than the
  // EXT_meshopt_compression the reader decodes, with its own codec version
  // rather than a rename. It marks its fallback buffer under the KHR name, so
  // the reader sees a URI-less buffer it has no reason to accept and refuses
  // the file outright. Kept in the corpus as the marker for that gap.
  'testdata/KhronosSampleModels/MeshoptCubeTest/glTF_Meshopt/MeshoptCubeTest.gltf':
    'not readable by either path: KHR_meshopt_compression, which the reader does not decode',
}));

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

const { buildSceneFromGltf } = await import(pathToFileURL(resolve(here, '..', 'src', 'gltf-loader.ts')).href);
const { buildSceneDocumentFromGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
);
const { buildViewerSceneFromDocument } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'scene-document-viewer.ts')).href
);
const { assertValidSceneDocument } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'scene-document.ts')).href
);

async function collect(directory) {
  const found = [];
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
async function companions(model, data) {
  const resources = Object.create(null);
  if (!model.toLowerCase().endsWith('.gltf')) return resources;
  let manifest;
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
      // A missing companion is the file's own problem, and both paths see it.
    }
  }
  return resources;
}

/**
 * A cheap order-sensitive fold over accessor bytes.
 *
 * Not a cryptographic digest and not meant to be one: it stands in for the
 * payload in a structural comparison, over a corpus of 73 MB that has to stay
 * inside a minute. Two different payloads colliding here is a missed finding,
 * not a wrong one.
 */
function digest(accessor) {
  if (!accessor) return null;
  const bytes = ArrayBuffer.isView(accessor.bytes)
    ? new Uint8Array(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength)
    : new Uint8Array(0);
  let hash = 0x811c9dc5;
  for (let index = 0; index < bytes.length; index += 1) {
    hash = Math.imul(hash ^ bytes[index], 0x01000193);
  }
  return `${accessor.componentType}:${accessor.components}:${accessor.count}:${bytes.length}:${(hash >>> 0).toString(16)}`;
}

/**
 * What both adapters must agree on.
 *
 * Not the material count: the preview appends one fallback record and
 * addresses material-less primitives through it, while the document leaves
 * them at index -1 for the renderer's own default. So materials are compared
 * where it matters — by the name each primitive actually resolves to.
 *
 * Positions and morph deltas are compared by their bytes, not only by their
 * counts. Counting alone let a real defect through: the document keeps morph
 * deltas quantized, the preview expands them, and both paths still reported
 * one target per primitive while only one of them could be blended.
 *
 * That particular defect stays pinned by `gltf-quantized-morph.mjs` rather than
 * here, because no asset in the corpus carries quantized morph deltas — every
 * morph target in testdata is already float, so the two paths agree on them
 * whether or not the expansion exists. Said out loud so the byte comparison is
 * not mistaken for covering a class it cannot reach.
 *
 * Indices and the remaining attributes stay uncompared for now. A Draco
 * primitive's bytes come out of the codec rather than out of a named accessor,
 * so byte equality there needs an argument before it becomes an assertion.
 */
function observe(scene) {
  const materialName = (primitive) => {
    const material = scene.materials[primitive.materialIndex];
    // The preview's trailing fallback has no name, and neither does an absent
    // material. Both mean "the renderer decides", which is the same thing.
    return material?.name ?? null;
  };
  return {
    nodes: scene.nodes.length,
    roots: scene.rootIndices.length,
    skins: scene.skins.length,
    textures: scene.textures.length,
    renderables: scene.renderables.length,
    clips: scene.animations.map((clip) => clip.channels.length),
    meshes: scene.meshes.map((mesh) => mesh.primitives.map((primitive) => ({
      mode: primitive.mode,
      vertices: primitive.attributes.POSITION?.count ?? 0,
      elements: primitive.indices?.count ?? null,
      attributes: Object.keys(primitive.attributes).sort().join(','),
      positions: digest(primitive.attributes.POSITION),
      morphs: primitive.morphPositions?.length ?? 0,
      morphPositions: (primitive.morphPositions ?? []).map(digest),
      morphNormals: (primitive.morphNormals ?? []).map(digest),
      material: materialName(primitive),
    }))),
  };
}

/** A one-line reason, short enough to sit next to a path in the summary. */
function reason(error) {
  return (error.message || String(error)).split('\n')[0].slice(0, 120);
}

const models = await collect(corpusRoot);
assert.ok(models.length >= 60, `the corpus shrank to ${models.length} files; expected the whole testdata tree`);

/** Every file that did not come through, and what stopped it. */
const problems = new Map();
let carried = 0;
for (const model of models) {
  const name = relative(repoRoot, model).replace(/\\/g, '/');
  const data = new Uint8Array(await readFile(model));
  const resources = await companions(model, data);

  let preview;
  try {
    preview = await buildSceneFromGltf(data, resources, gltfModule);
  } catch (error) {
    // A file the preview itself cannot open is a different finding from one
    // only the document route drops, but it is recorded the same way: skipping
    // it would let a reader regression pass here as a clean run.
    problems.set(name, `not readable by either path: ${reason(error)}`);
    continue;
  }

  try {
    const document = buildSceneDocumentFromGltf(data, resources, gltfModule);
    assertValidSceneDocument(document);
    assert.deepEqual(observe(buildViewerSceneFromDocument(document)), observe(preview));
    carried += 1;
  } catch (error) {
    problems.set(name, reason(error));
  }
}

console.log(`glTF corpus: ${carried}/${models.length} files carried by the portable document`);
for (const [name, note] of problems) console.log(`  ${name}: ${note}`);

const unexpected = [...problems.keys()].filter((name) => !KNOWN.has(name));
const fixed = [...KNOWN.keys()].filter((name) => !problems.has(name));
assert.deepEqual(unexpected, [], 'these files stopped surviving the portable document route');
assert.deepEqual(fixed, [], 'these files now survive; remove them from KNOWN and keep the count honest');

console.log('glTF corpus parity passed');
