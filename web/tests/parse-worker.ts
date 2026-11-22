/**
 * Off-main-thread file parsing for the converter: every format runs inside the
 * parse worker, transfers its input buffers across and back without copying,
 * returns plain transferable geometry/SceneDocument structures (never wasm
 * handles), handles malformed input without killing the worker, and supports
 * immediate cancellation via AbortSignal.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { createParseClient } from '../src/app/parse-client.ts';
import type { ParseFormat } from '../src/app/parse-tasks.ts';
import { tinySplatPly } from './smoke-fixtures.ts';

const here = dirname(fileURLToPath(import.meta.url));
const webRoot = resolve(here, '..');
const repoRoot = resolve(webRoot, '..');
const pkgRoot = resolve(webRoot, 'www', 'pkg');

async function loadPkg(format: ParseFormat) {
  const glueUrl = pathToFileURL(resolve(pkgRoot, `${format}.js`)).href;
  const wasmBytes = await readFile(resolve(pkgRoot, `${format}_bg.wasm`));
  const wasmModule = await WebAssembly.compile(wasmBytes);
  const fallbackModule = await import(glueUrl);
  await fallbackModule.default({ module_or_path: wasmModule });
  return { glueUrl, wasmModule, fallbackModule };
}

const [plyPkg, drcPkg, objPkg, stlPkg, gltfPkg, fbxPkg] = await Promise.all([
  loadPkg('ply'),
  loadPkg('drc'),
  loadPkg('obj'),
  loadPkg('stl'),
  loadPkg('gltf'),
  loadPkg('fbx'),
]);

const client = createParseClient();

try {
  // ---- PLY + zero-copy transfer round-trip ----------------------------------
  {
    const raw = await readFile(resolve(repoRoot, 'testdata', 'Box.ply'));
    const data = new Uint8Array(raw.buffer.slice(raw.byteOffset, raw.byteOffset + raw.byteLength));
    const expected = new Uint8Array(data);

    const parsed = await client.parse(
      {
        format: 'ply',
        extension: 'ply',
        data,
        resources: Object.create(null),
        glueUrl: plyPkg.glueUrl,
        wasmModule: plyPkg.wasmModule,
      },
      { transfer: true },
    );

    assert.equal(parsed.offMainThread, true, 'PLY runs on the worker thread');
    assert.equal(data.byteLength, 0, 'original view is detached after zero-copy transfer');
    assert.deepEqual(parsed.data, expected, 'worker transfers input bytes back intact');
    assert.equal(parsed.outcome.result.success, true);
    assert.ok((parsed.outcome.result.meshes?.length ?? 0) > 0);
  }

  // ---- Splat PLY extraction off-thread --------------------------------------
  {
    const raw = tinySplatPly().bytes;
    const splatBytes = new Uint8Array(raw.buffer.slice(raw.byteOffset, raw.byteOffset + raw.byteLength));
    const parsed = await client.parse({
      format: 'ply',
      extension: 'ply',
      data: splatBytes,
      resources: Object.create(null),
      glueUrl: plyPkg.glueUrl,
      wasmModule: plyPkg.wasmModule,
    });

    assert.equal(parsed.offMainThread, true);
    assert.equal(parsed.outcome.result.success, true);
    assert.ok(parsed.outcome.result.splats, 'splat PLY extracts SplatCloud inside worker');
    assert.ok(parsed.outcome.result.splats.positions instanceof Float32Array);
    assert.ok(parsed.outcome.result.splats.count > 0);
  }

  // ---- DRC decode + malformed payload handling ------------------------------
  {
    const raw = await readFile(resolve(repoRoot, 'testdata', 'car.drc'));
    const data = new Uint8Array(raw.buffer.slice(raw.byteOffset, raw.byteOffset + raw.byteLength));

    const parsed = await client.parse(
      {
        format: 'drc',
        extension: 'drc',
        data,
        resources: Object.create(null),
        glueUrl: drcPkg.glueUrl,
        wasmModule: drcPkg.wasmModule,
      },
      { transfer: true },
    );

    assert.equal(parsed.offMainThread, true);
    assert.equal(parsed.outcome.result.success, true);
    assert.ok((parsed.outcome.result.meshes?.[0]?.positions?.length ?? 0) > 0);

    // Worker-vs-fallback parity check on the same bytes.
    const fallbackClient = createParseClient({ workers: false });
    const direct = await fallbackClient.parse({
      format: 'drc',
      extension: 'drc',
      data: parsed.data,
      resources: Object.create(null),
      glueUrl: null,
      wasmModule: null,
      fallbackModule: drcPkg.fallbackModule,
    });
    assert.equal(direct.offMainThread, false);
    assert.equal(
      direct.outcome.result.meshes?.[0]?.positions?.length,
      parsed.outcome.result.meshes?.[0]?.positions?.length,
      'worker and fallback decode the same geometry',
    );
    fallbackClient.dispose();

    // Malformed Draco returns a structured error and keeps the worker alive.
    const bad = await client.parse({
      format: 'drc',
      extension: 'drc',
      data: new Uint8Array([0x44, 0x52, 0x41, 0x43, 0x4f, 0xff, 0xff]),
      resources: Object.create(null),
      glueUrl: drcPkg.glueUrl,
      wasmModule: drcPkg.wasmModule,
    });
    assert.equal(bad.outcome.result.success, false);
    assert.ok(bad.outcome.result.error, 'malformed DRC reports an error');
  }

  // ---- OBJ + companion MTL library off-thread -------------------------------
  {
    const objRaw = await readFile(resolve(repoRoot, 'testdata', 'cube_att_material.obj'));
    const mtlRaw = await readFile(resolve(repoRoot, 'testdata', 'cube_att_material.mtl'));
    const objData = new Uint8Array(objRaw.buffer.slice(objRaw.byteOffset, objRaw.byteOffset + objRaw.byteLength));
    const mtlData = new Uint8Array(mtlRaw.buffer.slice(mtlRaw.byteOffset, mtlRaw.byteOffset + mtlRaw.byteLength));
    const resources: Record<string, Uint8Array> = Object.create(null);
    resources['cube_att_material.mtl'] = mtlData;

    const parsed = await client.parse(
      {
        format: 'obj',
        extension: 'obj',
        data: objData,
        resources,
        glueUrl: objPkg.glueUrl,
        wasmModule: objPkg.wasmModule,
      },
      { transfer: true },
    );

    assert.equal(parsed.offMainThread, true);
    assert.equal(parsed.outcome.result.success, true);
    assert.equal(Object.getPrototypeOf(parsed.resources), null, 'returned resources keep null prototype');
    assert.ok(parsed.resources['cube_att_material.mtl'] instanceof Uint8Array);
    assert.ok(Object.keys(parsed.outcome.result.materials ?? {}).length > 0, 'MTL parsed inside worker');
  }

  // ---- STL off-thread -------------------------------------------------------
  {
    const stlRaw = await readFile(resolve(repoRoot, 'testdata', 'STL', 'test_sphere.stl'));
    const data = new Uint8Array(stlRaw.buffer.slice(stlRaw.byteOffset, stlRaw.byteOffset + stlRaw.byteLength));
    const parsed = await client.parse({
      format: 'stl',
      extension: 'stl',
      data,
      resources: Object.create(null),
      glueUrl: stlPkg.glueUrl,
      wasmModule: stlPkg.wasmModule,
    });
    assert.equal(parsed.offMainThread, true);
    assert.equal(parsed.outcome.result.success, true);
    assert.ok((parsed.outcome.result.meshes?.[0]?.positions?.length ?? 0) > 0);
  }

  // ---- glTF / GLB full SceneDocument + provenance off-thread ----------------
  {
    const glbRaw = await readFile(resolve(repoRoot, 'testdata', 'bun_zipper.glb'));
    const data = new Uint8Array(glbRaw.buffer.slice(glbRaw.byteOffset, glbRaw.byteOffset + glbRaw.byteLength));
    const parsed = await client.parse(
      {
        format: 'gltf',
        extension: 'glb',
        data,
        resources: Object.create(null),
        glueUrl: gltfPkg.glueUrl,
        wasmModule: gltfPkg.wasmModule,
      },
      { transfer: true },
    );

    assert.equal(parsed.offMainThread, true);
    assert.equal(parsed.outcome.result.success, true);
    assert.equal(parsed.outcome.result.document, true);
    assert.ok(parsed.outcome.sceneDocument, 'glTF builds portable SceneDocument inside worker');
    assert.ok(parsed.outcome.gltfProvenance, 'glTF returns extension provenance');
    assert.ok((parsed.outcome.result.vertexCount ?? 0) > 0, 'geometry summary computed from SceneDocument');
  }

  // ---- FBX + aliased resource buffer transfer -------------------------------
  {
    const written = fbxPkg.fallbackModule.write_fbx(
      [{
        name: 'Tri',
        positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
        indices: [0, 1, 2],
      }],
      { version: 7500, compression: false },
    );
    assert.equal(written.success, true);
    const fbxBytes = new Uint8Array(written.bytes);
    // Simulate FBX's aliased texture map (two keys pointing to the same buffer),
    // which would throw DataCloneError if transferables were not deduplicated.
    const texBytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47]);
    const resources: Record<string, Uint8Array> = Object.create(null);
    resources['tex/albedo.png'] = texBytes;
    resources['albedo.png'] = texBytes;

    const parsed = await client.parse(
      {
        format: 'fbx',
        extension: 'fbx',
        data: fbxBytes,
        resources,
        glueUrl: fbxPkg.glueUrl,
        wasmModule: fbxPkg.wasmModule,
      },
      { transfer: true },
    );

    assert.equal(parsed.offMainThread, true);
    assert.equal(parsed.outcome.result.success, true);
    assert.ok(parsed.outcome.sceneDocument, 'FBX builds SceneDocument inside worker');
    assert.ok(parsed.outcome.fbxProvenance, 'FBX returns provenance');
    assert.deepEqual(parsed.resources['albedo.png'], new Uint8Array([0x89, 0x50, 0x4e, 0x47]));
  }

  // ---- AbortSignal cancellation + recovery on next parse --------------------
  {
    const raw = await readFile(resolve(repoRoot, 'testdata', 'bun_zipper.ply'));
    const data = new Uint8Array(raw.buffer.slice(raw.byteOffset, raw.byteOffset + raw.byteLength));
    const controller = new AbortController();

    const inflight = client.parse(
      {
        format: 'ply',
        extension: 'ply',
        data,
        resources: Object.create(null),
        glueUrl: plyPkg.glueUrl,
        wasmModule: plyPkg.wasmModule,
      },
      { signal: controller.signal },
    );
    controller.abort();
    await assert.rejects(inflight, (err: Error) => err.name === 'AbortError');

    // A subsequent parse must spawn a fresh worker and succeed cleanly.
    const smallRaw = await readFile(resolve(repoRoot, 'testdata', 'Box.ply'));
    const smallData = new Uint8Array(smallRaw.buffer.slice(smallRaw.byteOffset, smallRaw.byteOffset + smallRaw.byteLength));
    const afterAbort = await client.parse({
      format: 'ply',
      extension: 'ply',
      data: smallData,
      resources: Object.create(null),
      glueUrl: plyPkg.glueUrl,
      wasmModule: plyPkg.wasmModule,
    });
    assert.equal(afterAbort.offMainThread, true);
    assert.equal(afterAbort.outcome.result.success, true);
  }

  console.log('parse-worker: OK');
} finally {
  client.dispose();
}
