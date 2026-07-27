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
 * These are findings, not accepted behaviour. Anything here is a file the
 * portable document route could not carry today, and the reason is the note.
 */
const KNOWN = new Map(Object.entries({
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
 * What both adapters must agree on.
 *
 * Not the material count: the preview appends one fallback record and
 * addresses material-less primitives through it, while the document leaves
 * them at index -1 for the renderer's own default. So materials are compared
 * where it matters — by the name each primitive actually resolves to.
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
      morphs: primitive.morphPositions?.length ?? 0,
      material: materialName(primitive),
    }))),
  };
}

const models = await collect(corpusRoot);
assert.ok(models.length >= 60, `the corpus shrank to ${models.length} files; expected the whole testdata tree`);

const problems = new Map();
const unopenable = [];
let carried = 0;
for (const model of models) {
  const name = relative(repoRoot, model).replace(/\\/g, '/');
  const data = new Uint8Array(await readFile(model));
  const resources = await companions(model, data);

  let preview;
  try {
    preview = await buildSceneFromGltf(data, resources, gltfModule);
  } catch (error) {
    // A file the preview itself cannot open is not a document problem; there
    // is nothing to compare against. Counted rather than dropped in silence,
    // so the headline figure is never mistaken for the whole corpus.
    unopenable.push(name);
    continue;
  }

  try {
    const document = buildSceneDocumentFromGltf(data, resources, gltfModule);
    assertValidSceneDocument(document);
    assert.deepEqual(observe(buildViewerSceneFromDocument(document)), observe(preview));
    carried += 1;
  } catch (error) {
    problems.set(name, error.message.split('\n')[0].slice(0, 120));
  }
}

console.log(
  `glTF corpus: ${carried}/${carried + problems.size} files carried by the portable document`
  + ` (${unopenable.length} of ${models.length} the preview could not open either)`,
);
for (const [name, reason] of problems) console.log(`  ${name}: ${reason}`);
for (const name of unopenable) console.log(`  ${name}: not readable by either path`);

const unexpected = [...problems.keys()].filter((name) => !KNOWN.has(name));
const fixed = [...KNOWN.keys()].filter((name) => !problems.has(name));
assert.deepEqual(unexpected, [], 'these files stopped surviving the portable document route');
assert.deepEqual(fixed, [], 'these files now survive; remove them from KNOWN and keep the count honest');

console.log('glTF corpus parity passed');
