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
const {
  MATERIAL_EXTENSION_DEFAULTS, MATERIAL_EXTENSION_TEXTURE_SLOTS, writeMaterialExtensions,
} = await import(pathToFileURL(resolve(here, '..', 'src', 'material-extensions.ts')).href);
const { MATERIAL_TEXTURE_SLOTS } = await import(pathToFileURL(resolve(here, '..', 'src', 'scene-document.ts')).href);
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
  KHR_materials_iridescence: {
    material: {
      extensions: {
        KHR_materials_iridescence: {
          iridescenceFactor: 1, iridescenceIor: 1.8, iridescenceThicknessMaximum: 550,
        },
      },
    },
    effect: (read) => {
      assert.equal(read.iridescenceFactor, 1);
      assert.equal(read.iridescenceIor, 1.8);
      assert.equal(read.iridescenceThicknessMaximum, 550);
      assert.equal(read.iridescenceThicknessMinimum, 100, 'an unstated thickness keeps the extension default');
    },
  },
  KHR_materials_sheen: {
    material: {
      extensions: { KHR_materials_sheen: { sheenColorFactor: [0.8, 0.6, 0.4], sheenRoughnessFactor: 0.6 } },
    },
    effect: (read) => {
      assert.deepEqual(read.sheenColorFactor, [0.8, 0.6, 0.4]);
      assert.equal(read.sheenRoughnessFactor, 0.6);
    },
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

// The slot list validation and the writer walk is the core slots plus whatever
// the extension table brings; the extension half needs no checking because it
// is spliced in rather than repeated, but the core half is written out by hand
// and a slot dropped from it silently stops being validated.
assert.deepEqual(
  MATERIAL_TEXTURE_SLOTS.filter((slot) => !MATERIAL_EXTENSION_TEXTURE_SLOTS.includes(slot)),
  [
    'baseColorTexture',
    'metallicRoughnessTexture',
    'normalTexture',
    'emissiveTexture',
    'occlusionTexture',
  ],
  'the core metallic-roughness slots must all be in MATERIAL_TEXTURE_SLOTS',
);

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

// Reading and writing are both derived from the table now, so a wrong default
// no longer shows up as a difference between two paths - both are wrong
// together. What the values must actually be is stated in `gltf-materials`,
// against literals; what is checked here is that a material at those values
// reads back as them and declares nothing.
const bare = readGltfMaterial({}, 0);
for (const [property, fallback] of Object.entries(MATERIAL_EXTENSION_DEFAULTS)) {
  assert.deepEqual(
    bare[property],
    fallback,
    `a material without extensions must read ${property} as what the core model implies`,
  );
}
assert.deepEqual(
  writeMaterialExtensions(bare, () => null),
  {},
  'a material at its defaults must declare no extension at all',
);

// Read and write are two directions of one table, and a field that survives one
// but not the other loses data on every export. Round-tripping distinctive
// values through both is what says they agree.
const stated = {
  extensions: {
    KHR_materials_unlit: {},
    KHR_materials_emissive_strength: { emissiveStrength: 3.5 },
    KHR_materials_ior: { ior: 1.9 },
    KHR_materials_specular: { specularFactor: 0.4, specularColorFactor: [0.5, 0.6, 0.7] },
    KHR_materials_sheen: { sheenColorFactor: [0.2, 0.3, 0.4], sheenRoughnessFactor: 0.6 },
    KHR_materials_iridescence: {
      iridescenceFactor: 0.7, iridescenceIor: 1.8,
      iridescenceThicknessMinimum: 200, iridescenceThicknessMaximum: 550,
    },
    KHR_materials_clearcoat: { clearcoatFactor: 0.8, clearcoatRoughnessFactor: 0.1 },
  },
};
const read = readGltfMaterial(stated, 0);
const written = writeMaterialExtensions(read, () => null);
assert.deepEqual(written, stated.extensions, 'the table must write back exactly what it read');
for (const [property, fallback] of Object.entries(MATERIAL_EXTENSION_DEFAULTS)) {
  assert.notDeepEqual(read[property], fallback, `${property} must be exercised away from its default here`);
  assert.deepEqual(readGltfMaterial({ extensions: written }, 0)[property], read[property]);
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
// Archived by Khronos and deliberately out of scope, so it will not quietly
// become interpreted and turn this check into a tautology.
const unclaimed = 'KHR_materials_pbrSpecularGlossiness';
assert.equal(GLTF_INTERPRETED_EXTENSIONS.has(unclaimed), false, 'pick a control the code really does not read');
const control = reported(unclaimed);
assert.equal(control.preview, true, 'an uninterpreted extension must be reported by the preview');
assert.equal(control.document, true, 'an uninterpreted extension must be reported by the document');

console.log('gltf-extension-support: OK');
