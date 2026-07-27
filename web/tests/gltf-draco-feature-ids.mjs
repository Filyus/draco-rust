/**
 * Feature IDs have to mean the same thing after compression.
 *
 * `EXT_mesh_features` names no accessor and no buffer view, so by the letter of
 * the registry it belongs with the other binary-free extensions. But its
 * payload rides on ordinary vertex attributes — `featureIds[].attribute: 2`
 * selects `_FEATURE_ID_2` — and those attributes are integers identifying
 * things. A quantized feature ID is not an approximate feature ID, it is the
 * wrong one, and nothing downstream could tell.
 *
 * So the permission granted in the registry rests on this measurement rather
 * than on reading the specification: the encoder must return every vertex
 * record unchanged. Compared as whole records rather than in file order,
 * because Draco reorders vertices and rewrites the indices to match — a
 * positional diff reports a change that is not one.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');
const pkg = resolve(here, '..', 'www', 'pkg');
const model = resolve(repoRoot, 'testdata', 'BoxMeta', 'glTF', 'BoxMeta.gltf');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

if (typeof gltfModule.GltfAsset?.prototype?.compressPrimitive !== 'function') {
  console.log('gltf-draco-feature-ids: SKIPPED (this WASM profile has no Draco encoder)');
  process.exit(0);
}

const data = new Uint8Array(await readFile(model));
const manifest = JSON.parse(new TextDecoder().decode(data));
const resources = Object.create(null);
for (const entry of [...(manifest.buffers || []), ...(manifest.images || [])]) {
  if (typeof entry.uri !== 'string' || entry.uri.startsWith('data:')) continue;
  resources[entry.uri] = new Uint8Array(await readFile(resolve(dirname(model), decodeURIComponent(entry.uri))));
}

const COMPONENT_WIDTH = { 5120: 1, 5121: 1, 5122: 2, 5123: 2, 5125: 4, 5126: 4 };

/** One string per vertex holding every attribute that vertex carries. */
function vertexRecords(asset) {
  const geometry = asset.readPrimitive(0, 0);
  const attributes = [];
  for (let index = 0; index < geometry.attributeCount(); index += 1) {
    attributes.push({
      semantic: geometry.attributeSemantic(index),
      components: geometry.attributeComponents(index),
      componentType: geometry.attributeComponentType(index),
      bytes: new Uint8Array(geometry.attributeBytes(index)),
    });
  }
  attributes.sort((left, right) => left.semantic.localeCompare(right.semantic));
  const stride = (attribute) => attribute.components * COMPONENT_WIDTH[attribute.componentType];
  const count = attributes[0].bytes.length / stride(attributes[0]);
  const rows = [];
  for (let vertex = 0; vertex < count; vertex += 1) {
    rows.push(attributes.map((attribute) => {
      const width = stride(attribute);
      const slice = attribute.bytes.subarray(vertex * width, vertex * width + width);
      return `${attribute.semantic}:${attribute.componentType}=${[...slice].join('.')}`;
    }).join('|'));
  }
  return { rows: rows.sort(), semantics: attributes.map((attribute) => attribute.semantic) };
}

const plainAsset = gltfModule.GltfAsset.withResources(data, resources, '2.1');
const plain = vertexRecords(plainAsset);
plainAsset.free();

const compressedAsset = gltfModule.GltfAsset.withResources(data, resources, '2.1');
for (let primitive = 0; primitive < compressedAsset.primitiveCount(0); primitive += 1) {
  compressedAsset.compressPrimitive(0, primitive, 5, 5);
}
const compressed = vertexRecords(compressedAsset);
const output = compressedAsset.glb(2);
compressedAsset.free();

// Three feature-ID attributes of three different component types, plus the two
// property attributes the metadata addresses by name. If the fixture stops
// carrying them the measurement below stops meaning anything.
for (const semantic of ['_FEATURE_ID_0', '_FEATURE_ID_1', '_FEATURE_ID_2', '_DIRECTION', '_MAGNITUDE']) {
  assert.ok(plain.semantics.includes(semantic), `BoxMeta must still carry ${semantic}`);
  assert.ok(compressed.semantics.includes(semantic), `${semantic} did not survive compression`);
}

assert.equal(compressed.rows.length, plain.rows.length, 'compression must not add or drop vertices');
assert.deepEqual(
  compressed.rows,
  plain.rows,
  'every vertex must come back with the same values, component types and pairing',
);

// The JSON still has to resolve: `attribute: N` selects `_FEATURE_ID_N`, so a
// surviving extension pointing at a vanished attribute is no better than a
// dropped one.
const bytes = new Uint8Array(output);
const jsonLength = new DataView(bytes.buffer, bytes.byteOffset, 20).getUint32(12, true);
const written = JSON.parse(new TextDecoder().decode(bytes.subarray(20, 20 + jsonLength)));
const primitive = written.meshes[0].primitives[0];
// Without this the record comparison above would pass on an asset that was
// never compressed, which is the one way it could be vacuously true.
assert.ok(
  primitive.extensions?.KHR_draco_mesh_compression,
  'the primitive must actually have been compressed',
);
const featureIds = primitive.extensions?.EXT_mesh_features?.featureIds;
assert.ok(Array.isArray(featureIds) && featureIds.length > 0, 'the extension must survive the write');
const named = new Set([
  ...Object.keys(primitive.attributes || {}),
  ...Object.keys(primitive.extensions?.KHR_draco_mesh_compression?.attributes || {}),
]);
for (const featureId of featureIds) {
  if (featureId.attribute === undefined) continue;
  assert.ok(
    named.has(`_FEATURE_ID_${featureId.attribute}`),
    `featureIds names _FEATURE_ID_${featureId.attribute}, which the compressed primitive no longer has`,
  );
}

console.log(`gltf-draco-feature-ids: OK (${plain.rows.length} vertex records unchanged)`);
