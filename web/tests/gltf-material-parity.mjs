/**
 * The two producers of a viewer material must agree.
 *
 * `gltf-loader.ts` builds the preview scene straight from glTF; the portable
 * SceneDocument reaches the same renderer through `scene-document-viewer.ts`.
 * Both feed one `applyMaterial`, so a field that reaches one and not the other
 * shades the same asset differently depending on which path loaded it — which
 * is exactly what happened when the preview grew clearcoat, ior, specular and
 * emissive strength and the document did not.
 *
 * Comparing the whole record rather than a list of interesting fields is the
 * point: a new material feature added to one producer alone fails here without
 * anyone remembering to extend the test.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { stripeNormalMapPng } from './smoke-fixtures.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

const { buildMaterials } = await import(pathToFileURL(resolve(here, '..', 'src', 'gltf-loader.ts')).href);
const { buildSceneDocumentFromGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'gltf-scene-document.ts')).href
);
const { buildViewerSceneFromDocument } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'scene-document-viewer.ts')).href
);
const { lowerSceneDocumentToGltf } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'scene-document-gltf.ts')).href
);

const geometry = new Float32Array([
  0, 0, 0, 1, 0, 0, 0, 1, 0, // POSITION
  0, 0, 1, 0, 0, 1, 0, 0, 0, // TEXCOORD_0, padded to the same view length
]);
const image = `data:image/png;base64,${stripeNormalMapPng().toString('base64')}`;

/**
 * One material exercising every slot and factor the renderer reads, so the
 * comparison below covers the whole record rather than the common case.
 */
const material = {
  name: 'Everything',
  doubleSided: true,
  alphaMode: 'MASK',
  alphaCutoff: 0.25,
  emissiveFactor: [0.1, 0.2, 0.3],
  pbrMetallicRoughness: {
    baseColorFactor: [0.5, 0.4, 0.3, 1],
    metallicFactor: 0.25,
    roughnessFactor: 0.75,
    baseColorTexture: {
      index: 0,
      texCoord: 0,
      extensions: { KHR_texture_transform: { offset: [0.1, 0.2], scale: [2, 3], rotation: 0.5 } },
    },
    metallicRoughnessTexture: { index: 1 },
  },
  normalTexture: { index: 2, scale: 0.4 },
  occlusionTexture: { index: 3, strength: 0.6 },
  emissiveTexture: {
    index: 4,
    extensions: { KHR_texture_transform: { offset: [0, 0], scale: [4, 4], rotation: 0, texCoord: 0 } },
  },
  extensions: {
    KHR_materials_unlit: {},
    KHR_materials_emissive_strength: { emissiveStrength: 3 },
    KHR_materials_ior: { ior: 1.7 },
    KHR_materials_specular: {
      specularFactor: 0.5,
      specularColorFactor: [1, 0.5, 0.25],
      specularTexture: { index: 5 },
      specularColorTexture: { index: 6 },
    },
    KHR_materials_clearcoat: {
      clearcoatFactor: 0.9,
      clearcoatRoughnessFactor: 0.05,
      clearcoatTexture: { index: 7 },
      clearcoatRoughnessTexture: { index: 8 },
      clearcoatNormalTexture: { index: 9, scale: 0.8 },
    },
  },
};

const manifest = {
  asset: { version: '2.0' },
  buffers: [{
    byteLength: geometry.byteLength,
    uri: `data:application/octet-stream;base64,${Buffer.from(geometry.buffer).toString('base64')}`,
  }],
  bufferViews: [
    { buffer: 0, byteOffset: 0, byteLength: 36 },
    { buffer: 0, byteOffset: 36, byteLength: 24 },
  ],
  accessors: [
    { bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] },
    { bufferView: 1, componentType: 5126, count: 3, type: 'VEC2' },
  ],
  images: [{ mimeType: 'image/png', uri: image }],
  samplers: [{ wrapS: 33071, wrapT: 33071, minFilter: 9729, magFilter: 9729 }],
  // Ten textures over one image: the two producers index textures in different
  // spaces, and only a one-to-one mapping proves the slots line up rather than
  // happening to collide on index 0.
  textures: Array.from({ length: 10 }, () => ({ source: 0, sampler: 0 })),
  materials: [material],
  meshes: [{ primitives: [{ attributes: { POSITION: 0, TEXCOORD_0: 1 }, material: 0 }] }],
  nodes: [{ mesh: 0 }],
  scenes: [{ nodes: [0] }],
  scene: 0,
  extensionsUsed: [
    'KHR_texture_transform',
    'KHR_materials_unlit',
    'KHR_materials_emissive_strength',
    'KHR_materials_ior',
    'KHR_materials_specular',
    'KHR_materials_clearcoat',
  ],
};

const source = new TextEncoder().encode(JSON.stringify(manifest));

const fromLoader = buildMaterials(manifest.materials)[0];
const document = buildSceneDocumentFromGltf(source, {}, gltfModule);
const fromDocument = buildViewerSceneFromDocument(document).materials[0];

assert.deepEqual(
  Object.keys(fromDocument).sort(),
  Object.keys(fromLoader).sort(),
  'the two viewer material producers disagree on which fields exist',
);
assert.deepEqual(fromDocument, fromLoader, 'the two viewer material producers disagree on a value');

// Everything the document carries has to come back out again, or the preview
// shows a coated surface that the exported file no longer describes.
const lowered = JSON.parse(new TextDecoder().decode(lowerSceneDocumentToGltf(document).json));
const exported = lowered.materials[0];
assert.deepEqual(exported.extensions.KHR_materials_clearcoat, {
  clearcoatFactor: 0.9,
  clearcoatRoughnessFactor: 0.05,
  clearcoatTexture: { index: 7, texCoord: 0 },
  clearcoatRoughnessTexture: { index: 8, texCoord: 0 },
  clearcoatNormalTexture: { index: 9, texCoord: 0, scale: 0.8 },
});
assert.deepEqual(exported.extensions.KHR_materials_specular, {
  specularFactor: 0.5,
  specularColorFactor: [1, 0.5, 0.25],
  specularTexture: { index: 5, texCoord: 0 },
  specularColorTexture: { index: 6, texCoord: 0 },
});
assert.deepEqual(exported.extensions.KHR_materials_ior, { ior: 1.7 });
assert.deepEqual(exported.extensions.KHR_materials_emissive_strength, { emissiveStrength: 3 });
// Merged, not overwritten: an unlit coated material keeps both.
assert.deepEqual(exported.extensions.KHR_materials_unlit, {});

for (const extension of manifest.extensionsUsed) {
  assert.ok(
    lowered.extensionsUsed.includes(extension),
    `${extension} was written into materials but not declared in extensionsUsed`,
  );
}

// A material with no extensions must stay one: emitting KHR_materials_ior with
// the default 1.5 on every material would declare extensions nothing needs.
const plain = buildSceneDocumentFromGltf(
  new TextEncoder().encode(JSON.stringify({
    ...manifest,
    materials: [{ name: 'Plain', pbrMetallicRoughness: {} }],
    extensionsUsed: undefined,
  })),
  {},
  gltfModule,
);
assert.deepEqual(Object.keys(plain.materials[0]).filter((key) => /clearcoat|specular|ior|emissiveStrength/i.test(key)), []);
const loweredPlain = JSON.parse(new TextDecoder().decode(lowerSceneDocumentToGltf(plain).json));
assert.equal(loweredPlain.materials[0].extensions, undefined);
assert.equal(loweredPlain.extensionsUsed, undefined);

console.log('glTF material parity passed');
