/**
 * The extension lists must describe what the code actually does.
 *
 * `GLTF_INTERPRETED_EXTENSIONS`, `GLTF_READER_RESOLVED_EXTENSIONS` and
 * `GLTF_TEXTURE_SOURCE_EXTENSIONS` are hand-written sets of strings, and every
 * consumer decides what to report as ignored by asking them. Nothing tied them
 * to the code that does the interpreting, so both mistakes were silent: adding
 * a name without reading it means the preview says nothing about an extension
 * it does not understand, and reading one without adding it means the preview
 * applies the extension *and* reports it as ignored in the same breath.
 *
 * So each interpreted extension needs a case here that shows its effect on the
 * reading, and a name with no case fails the gate. The corresponding claim -
 * that neither consumer reports it - is checked against both consumers, since
 * "ignored by the preview" and "omitted from the portable document" are
 * separate predicates over the same list.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

const {
  GLTF_INTERPRETED_EXTENSIONS,
  GLTF_READER_RESOLVED_EXTENSIONS,
  GLTF_TEXTURE_SOURCE_EXTENSIONS,
  readGltfMaterial,
} = await import(pathToFileURL(resolve(here, '..', 'src', 'gltf-interpretation.ts')).href);
const { extensionWarnings } = await import(pathToFileURL(resolve(here, '..', 'src', 'gltf-loader.ts')).href);
const { buildSceneDocumentFromGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
);

/**
 * What each interpreted extension does to the reading, stated as the material
 * JSON that carries it and the difference it must make.
 *
 * `KHR_texture_transform` rides on a texture binding rather than on the
 * material, so its case reads the binding instead.
 */
const INTERPRETED = {
  KHR_materials_unlit: {
    material: { extensions: { KHR_materials_unlit: {} } },
    effect: (read) => assert.equal(read.unlit, true, 'KHR_materials_unlit must reach the reading'),
  },
  KHR_materials_ior: {
    material: { extensions: { KHR_materials_ior: { ior: 1.7 } } },
    effect: (read) => assert.equal(read.ior, 1.7),
  },
  KHR_materials_specular: {
    material: {
      extensions: { KHR_materials_specular: { specularFactor: 0.3, specularColorFactor: [1, 0.5, 0.25] } },
    },
    effect: (read) => {
      assert.equal(read.specularFactor, 0.3);
      assert.deepEqual(read.specularColorFactor, [1, 0.5, 0.25]);
    },
  },
  KHR_materials_emissive_strength: {
    material: { extensions: { KHR_materials_emissive_strength: { emissiveStrength: 4 } } },
    effect: (read) => assert.equal(read.emissiveStrength, 4),
  },
  KHR_materials_clearcoat: {
    material: {
      extensions: { KHR_materials_clearcoat: { clearcoatFactor: 1, clearcoatRoughnessFactor: 0.2 } },
    },
    effect: (read) => {
      assert.equal(read.clearcoatFactor, 1);
      assert.equal(read.clearcoatRoughnessFactor, 0.2);
    },
  },
  KHR_texture_transform: {
    material: {
      pbrMetallicRoughness: {
        baseColorTexture: {
          index: 0,
          extensions: { KHR_texture_transform: { offset: [0.25, 0.5], scale: [2, 2], rotation: 0.5 } },
        },
      },
    },
    effect: (read) => assert.deepEqual(
      read.baseColorTexture.transform,
      { offset: [0.25, 0.5], scale: [2, 2], rotation: 0.5 },
    ),
  },
};

// A name in the set with no case here is the failure this gate exists for: it
// claims an interpretation nobody demonstrated.
assert.deepEqual(
  new Set(Object.keys(INTERPRETED)),
  new Set(GLTF_INTERPRETED_EXTENSIONS),
  'every interpreted extension needs a case showing what it does to the reading',
);

/** A triangle carrying `material`, declared as using `extensions`. */
function assetWith(extensions, material) {
  return new TextEncoder().encode(JSON.stringify({
    asset: { version: '2.0' },
    extensionsUsed: extensions,
    buffers: [{
      byteLength: 36,
      uri: 'data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA',
    }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }],
    accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] }],
    ...(material ? { materials: [material] } : {}),
    meshes: [{ primitives: [{ attributes: { POSITION: 0 }, ...(material ? { material: 0 } : {}) }] }],
    nodes: [{ mesh: 0 }],
    scenes: [{ nodes: [0] }],
    scene: 0,
  }));
}

/** Whether either consumer names `extension` in what it could not act on. */
function reported(extension, material = null) {
  const bytes = assetWith([extension], material);
  const manifest = JSON.parse(new TextDecoder().decode(bytes));
  const preview = extensionWarnings(manifest, new Map());
  const document = buildSceneDocumentFromGltf(bytes, Object.create(null), gltfModule);
  const names = (warnings) => warnings.some((warning) => warning.includes(extension));
  return { preview: names(preview), document: names(document.warnings) };
}

for (const [extension, { material, effect }] of Object.entries(INTERPRETED)) {
  // The material a texture-transform case needs a texture for is read without
  // one: readGltfMaterial works in glTF index space and never resolves it.
  effect(readGltfMaterial(material, 0));
  const seen = reported(extension, null);
  assert.equal(seen.preview, false, `${extension} is interpreted, so the preview must not report it as ignored`);
  assert.equal(seen.document, false, `${extension} is interpreted, so the document must not report it as omitted`);
}

// The reader resolves these before either consumer runs - decoded payloads and
// attributes in their storage type - so neither may report them either. The
// claim is about the declaration alone; that the payloads decode is what the
// corpus gate measures.
for (const extension of GLTF_READER_RESOLVED_EXTENSIONS) {
  const seen = reported(extension);
  assert.equal(seen.preview, false, `${extension} is resolved by the reader, so the preview must not report it`);
  assert.equal(seen.document, false, `${extension} is resolved by the reader, so the document must not report it`);
}

// An alternate image source is the one class where the two consumers are
// entitled to disagree: the document carries the bytes whatever the codec, the
// preview can only claim the source once the browser decoded it.
for (const extension of GLTF_TEXTURE_SOURCE_EXTENSIONS) {
  const manifest = { extensionsUsed: [extension] };
  assert.deepEqual(
    extensionWarnings(manifest, new Map([[extension, true]])),
    [],
    `${extension} must be honored by the preview once every image through it decoded`,
  );
  assert.equal(
    extensionWarnings(manifest, new Map([[extension, false]])).some((warning) => warning.includes(extension)),
    true,
    `${extension} must be reported by the preview when an image through it did not decode`,
  );
  assert.equal(reported(extension).document, false, `${extension} carries bytes, so the document must not report it`);
}

// The control: an extension nobody claims must be named by both, or the
// assertions above would hold trivially for a predicate that honors everything.
const unclaimed = 'KHR_materials_sheen';
assert.equal(GLTF_INTERPRETED_EXTENSIONS.has(unclaimed), false, 'pick a control the code really does not read');
const control = reported(unclaimed);
assert.equal(control.preview, true, 'an uninterpreted extension must be reported by the preview');
assert.equal(control.document, true, 'an uninterpreted extension must be reported by the document');

console.log('gltf-extension-support: OK');
