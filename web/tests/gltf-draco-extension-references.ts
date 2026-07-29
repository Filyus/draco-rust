/**
 * References an extension owns have to survive compaction.
 *
 * Draco-only output detaches the accessors the codec replaced and then drops
 * every accessor and buffer view nothing points at any more, renumbering what
 * is left. Core glTF references are walked by the transformer itself, but an
 * extension's are not: it never inspects JSON nobody has read.
 *
 * Two extensions in the corpus own such references, and each fails a different
 * way if only half the handler is right:
 *
 * - `EXT_mesh_gpu_instancing` names accessors that no primitive names, so
 *   without `collect` they look unreferenced and the instances vanish;
 * - `EXT_structural_metadata` names buffer views holding property-table
 *   columns, and one of the fixtures points at a zero-length view, which is
 *   exactly the kind of thing a compactor drops as empty.
 *
 * Both then need `remap`, or the surviving reference points at whatever moved
 * into that index. So the assertions are not "the extension is still there" —
 * a stale index is still there too — but "it still resolves to the same data".
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');
const pkg = resolve(here, '..', 'www', 'pkg');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

if (typeof gltfModule.GltfAsset?.prototype?.compressPrimitive !== 'function') {
  console.log('gltf-draco-extension-references: SKIPPED (this WASM profile has no Draco encoder)');
  process.exit(0);
}

async function companions(model: string, manifest: any): Promise<Record<string, Uint8Array>> {
  const resources: Record<string, Uint8Array> = Object.create(null);
  for (const entry of [...(manifest.buffers || []), ...(manifest.images || [])]) {
    if (typeof entry.uri !== 'string' || entry.uri.startsWith('data:')) continue;
    resources[entry.uri] = new Uint8Array(await readFile(resolve(dirname(model), decodeURIComponent(entry.uri))));
  }
  return resources;
}

/** Compress every primitive and return the GLB's JSON and its binary chunk. */
async function compressed(relativePath: string) {
  const model = resolve(repoRoot, relativePath);
  const data = new Uint8Array(await readFile(model));
  const manifest = JSON.parse(new TextDecoder().decode(data));
  const asset = gltfModule.GltfAsset.withResources(data, await companions(model, manifest), '2.1');
  try {
    for (let mesh = 0; mesh < asset.meshCount(); mesh += 1) {
      for (let primitive = 0; primitive < asset.primitiveCount(mesh); primitive += 1) {
        asset.compressPrimitive(mesh, primitive, 5, 5);
      }
    }
    const bytes = new Uint8Array(asset.glb(2));
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const jsonLength = view.getUint32(12, true);
    const json = JSON.parse(new TextDecoder().decode(bytes.subarray(20, 20 + jsonLength)));
    // The BIN chunk follows the JSON chunk and its own eight-byte header.
    const binaryStart = 20 + jsonLength + 8;
    return { source: manifest, json, binary: bytes.subarray(binaryStart) };
  } finally {
    asset.free();
  }
}

// EXT_mesh_gpu_instancing: four copies of one quad, placed by three accessors
// that no primitive refers to.
{
  const { source, json } = await compressed('testdata/InstancedQuads.gltf');
  const before = source.nodes.find((node: any) => node.extensions?.EXT_mesh_gpu_instancing);
  const after = json.nodes.find((node: any) => node.extensions?.EXT_mesh_gpu_instancing);
  assert.ok(before && after, 'the instancing extension must survive compression');
  const attributes = after.extensions.EXT_mesh_gpu_instancing.attributes;
  assert.deepEqual(
    Object.keys(attributes).sort(),
    Object.keys(before.extensions.EXT_mesh_gpu_instancing.attributes).sort(),
    'no instance attribute may be dropped',
  );
  for (const [semantic, index] of Object.entries(attributes) as [string, number][]) {
    const accessor = json.accessors[index];
    assert.ok(accessor, `instancing ${semantic} points at accessor ${index}, which does not exist`);
    const original = source.accessors[before.extensions.EXT_mesh_gpu_instancing.attributes[semantic]];
    // The index moved; what it addresses must not have.
    assert.equal(accessor.type, original.type, `instancing ${semantic} now addresses a different accessor`);
    assert.equal(accessor.count, original.count, `instancing ${semantic} now addresses a different accessor`);
    assert.equal(accessor.componentType, original.componentType, `instancing ${semantic} now addresses a different accessor`);
  }
}

// EXT_structural_metadata: property tables whose columns are buffer views,
// including one of zero length.
for (const relativePath of [
  'testdata/ZeroLengthBufferView/ZeroLengthBufferView.gltf',
  'testdata/BoxMeta/glTF/BoxMeta.gltf',
]) {
  const { source, json, binary } = await compressed(relativePath);
  const columns = (manifest: any) => (manifest.extensions?.EXT_structural_metadata?.propertyTables || [])
    .flatMap((table: any) => Object.entries(table.properties || {})
      .flatMap(([property, slots]: [string, any]) => ['values', 'arrayOffsets', 'stringOffsets']
        .filter((slot) => slots[slot] !== undefined)
        .map((slot) => ({ key: `${table.class}.${property}.${slot}`, view: slots[slot] }))));

  const before = columns(source);
  const after = columns(json);
  assert.ok(before.length > 0, `${relativePath} must carry property-table columns to be worth testing`);
  assert.deepEqual(
    after.map((column: any) => column.key),
    before.map((column: any) => column.key),
    `${relativePath}: no property-table column may be dropped`,
  );

  for (let index = 0; index < after.length; index += 1) {
    const view = json.bufferViews[after[index].view];
    assert.ok(view, `${relativePath}: ${after[index].key} points at buffer view ${after[index].view}, which does not exist`);
    const length = source.bufferViews[before[index].view].byteLength;
    assert.equal(view.byteLength, length, `${relativePath}: ${after[index].key} now addresses a different buffer view`);
    // And the bytes are reachable: a view kept alive but left outside the
    // written buffer is no better than a dropped one.
    assert.ok(
      (view.byteOffset || 0) + view.byteLength <= binary.length,
      `${relativePath}: ${after[index].key} addresses bytes past the end of the buffer`,
    );
  }
}

console.log('gltf-draco-extension-references: OK');
