/**
 * What an export route costs has to reach the caller.
 *
 * Five of the six routes computed warnings and threw them away — most
 * structurally, in `prepareFbxSceneForExport`, which rebuilds the scene object
 * from named fields and simply did not name `warnings`. The user converting a
 * lit, textured glTF to FBX was told nothing.
 *
 * These gates are on the pure route helpers rather than on `exportFile`, which
 * needs a browser; the Playwright suite covers the wiring end to end.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import type {
  exportSceneDocumentToGlb as ExportSceneDocumentToGlb,
  mergeMeshes as MergeMeshes,
  prepareFbxAnimationForExport as PrepareFbxAnimationForExport,
  prepareFbxSceneForExport as PrepareFbxSceneForExport,
  prepareMeshesForExport as PrepareMeshesForExport,
  runExport as RunExport,
} from '../src/app/export-branches.ts';
import type { createSceneDocument as CreateSceneDocument } from '../src/scene-document.ts';
import type { modules as ModulesState, state as AppState } from '../src/app/state.ts';

const here = dirname(fileURLToPath(import.meta.url));
const {
  exportSceneDocumentToGlb,
  runExport,
  mergeMeshes,
  prepareFbxAnimationForExport,
  prepareFbxSceneForExport,
  prepareMeshesForExport,
} = await import(pathToFileURL(resolve(here, '..', 'src', 'app', 'export-branches.ts')).href) as {
  exportSceneDocumentToGlb: typeof ExportSceneDocumentToGlb;
  runExport: typeof RunExport;
  mergeMeshes: typeof MergeMeshes;
  prepareFbxAnimationForExport: typeof PrepareFbxAnimationForExport;
  prepareFbxSceneForExport: typeof PrepareFbxSceneForExport;
  prepareMeshesForExport: typeof PrepareMeshesForExport;
};

const settings = { includeNormals: true, includeUvs: true };

// The regression this file exists for.
const prepared = prepareFbxSceneForExport({
  rootNodes: [],
  materials: [],
  textures: [],
  animations: [],
  warnings: ['Skin 0 has unsupported inverse bind matrices'],
}, settings);
assert.deepEqual(
  prepared.warnings,
  ['Skin 0 has unsupported inverse bind matrices'],
  'preparing a scene for FBX export must not drop what the builder reported',
);
assert.deepEqual(prepareFbxSceneForExport({}, settings).warnings, [], 'a scene without warnings yields an empty list');

// GlobalSettings carry the source unit scale; rebuilding the object dropped
// those too, which silently rescaled a re-exported FBX.
assert.deepEqual(
  prepareFbxSceneForExport({ globalSettings: { unitScaleFactor: 2.54 } }, settings).globalSettings,
  { unitScaleFactor: 2.54 },
);

// Preparation is driven by explicit settings now, not by reading checkboxes.
const mesh = {
  name: 'quad',
  positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
  indices: [0, 1, 2],
  normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
  uvs: [0, 0, 1, 0, 0, 1],
  uvSets: [{ name: 'UV1', mapping: 'byPolygonVertex', reference: 'direct', values: [0, 0], indices: [] }],
};
const [withAll] = prepareMeshesForExport([mesh], { includeNormals: true, includeUvs: true });
assert.equal(withAll.normals!.length, 9);
assert.equal(withAll.uvs!.length, 6);
assert.equal(withAll.uvSets[0].name, 'UV1');
const [without] = prepareMeshesForExport([mesh], { includeNormals: false, includeUvs: false });
assert.equal(without.normals, null);
assert.equal(without.uvs, null);
assert.equal(without.uvSets.length, 1, 'extra layer sets are not what the normals/UV toggles control');

// Legacy FBX importers mishandle cubic tangents; the flag has to flatten them.
const clip = {
  name: 'Take 001',
  duration: 1,
  channels: [{
    nodeName: 'Cube',
    nodeId: 1,
    path: 'translation',
    sampler: { input: [0, 1], output: [0, 0, 0, 1, 1, 1], interpolation: 'cubic', inTangents: [1], outTangents: [1] },
  }],
};
assert.equal(prepareFbxAnimationForExport(clip, false).channels[0].sampler.interpolation, 'cubic');
const legacy = prepareFbxAnimationForExport(clip, true).channels[0].sampler;
assert.equal(legacy.interpolation, 'linear');
assert.equal(legacy.inTangents, null);
assert.equal(legacy.outTangents, null);

// Morph weight channels address a target; losing that index collapses every
// shape onto the first one.
const morphClip = prepareFbxAnimationForExport({
  name: 'Morph',
  duration: 1,
  channels: [
    { nodeName: 'Cube', nodeId: 1, path: 'morphweight', morphTargetIndex: 1, sampler: { input: [0], output: [0] } },
  ],
}, false);
assert.equal(morphClip.channels[0].morphTargetIndex, 1);

// Merging rebases indices; PLY is the only consumer and it takes one mesh.
const merged = mergeMeshes([
  { positions: [0, 0, 0, 1, 0, 0, 0, 1, 0], indices: [0, 1, 2] },
  { positions: [2, 0, 0, 3, 0, 0, 2, 1, 0], indices: [0, 1, 2] },
] as any);
assert.deepEqual(merged.indices, [0, 1, 2, 3, 4, 5]);
assert.equal(merged.positions.length, 18);

// The element-at-a-time append in mergeMeshes is load-bearing: spreading a
// buffer this size into push() overflows the call stack.
const big = { positions: new Array(300000).fill(1), indices: [0, 1, 2] };
assert.equal(mergeMeshes([big, big] as any).positions.length, 600000);

/**
 * The Draco checkbox has to reach the document route.
 *
 * It did not: the route read the export settings and never looked at
 * `useDraco`, so an FBX exported as a "compressed" GLB came out uncompressed
 * and said nothing about it. The document itself has no way to express Draco
 * and does not need one — compression is a second pass over the GLB this route
 * already produces — which is exactly why the omission was invisible.
 */
const { createSceneDocument } = await import(pathToFileURL(resolve(here, '..', 'src', 'scene-document.ts')).href) as {
  createSceneDocument: typeof CreateSceneDocument;
};
const gltfModule = await import(pathToFileURL(resolve(here, '..', 'www', 'pkg', 'gltf.js')).href);
await gltfModule.default({
  module_or_path: await readFile(resolve(here, '..', 'www', 'pkg', 'gltf_bg.wasm')),
});
const { modules } = await import(pathToFileURL(resolve(here, '..', 'src', 'app', 'state.ts')).href) as {
  modules: typeof ModulesState;
};
modules.gltf.loaded = true;
modules.gltf.module = gltfModule;

const triangle = createSceneDocument({
  accessors: [{
    bytes: new Uint8Array(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]).buffer),
    componentType: 5126,
    components: 3,
    count: 3,
    min: [0, 0, 0],
    max: [1, 1, 0],
  }] as any,
  meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }] as any,
  nodes: [{ name: 'triangle', mesh: 0 }] as any,
  rootNodes: [0],
});

/** The JSON chunk of a GLB, which is where the extension declarations live. */
const glbManifest = (binary: ArrayBuffer | Uint8Array | ArrayLike<number>) => {
  const bytes = new Uint8Array(binary);
  const length = new DataView(bytes.buffer, bytes.byteOffset, 20).getUint32(12, true);
  return JSON.parse(new TextDecoder().decode(bytes.subarray(20, 20 + length)));
};

const plain = exportSceneDocumentToGlb(triangle, { useDraco: false, encodingSpeed: 5 });
assert.equal(plain.result.success, true);
assert.equal(plain.result.draco_stats, null, 'an uncompressed export reports no compression');
assert.equal(
  glbManifest(plain.result.binary_data!).extensionsRequired,
  undefined,
  'without the checkbox the GLB must not require Draco',
);

if (typeof gltfModule.GltfAsset?.prototype?.compressPrimitive === 'function') {
  const compressed = exportSceneDocumentToGlb(triangle, { useDraco: true, encodingSpeed: 5 });
  assert.equal(compressed.result.success, true);
  assert.deepEqual(compressed.warnings, [], 'a document this route wrote itself must compress cleanly');
  assert.ok(
    (glbManifest(compressed.result.binary_data!).extensionsRequired ?? []).includes('KHR_draco_mesh_compression'),
    'the checkbox must reach the writer, not just the settings object',
  );
  assert.equal(compressed.result.draco_stats!.primitives, 1);
  assert.ok(compressed.result.draco_stats!.compressed_size > 0);
  assert.ok(compressed.result.draco_stats!.source_bytes > 0);
  assert.ok(compressed.result.draco_stats!.output_bytes > 0);
  assert.equal(
    compressed.result.draco_stats!.method,
    'edgebreaker',
    'GLB compression statistics must report the method Draco selected',
  );
  assert.equal(
    compressed.result.draco_stats!.speed,
    5,
    'GLB compression statistics must report the resolved encoder speed',
  );

  // And through the route that actually runs, not only the helper it calls:
  // the settings object reached `runExport` all along, and the loss was one
  // call site inside it that never passed the flag on.
  const { state } = await import(pathToFileURL(resolve(here, '..', 'src', 'app', 'state.ts')).href) as {
    state: typeof AppState;
  };
  state.currentMeshData = { document: null, scene: null, meshes: [] } as any;
  state.currentFileType = 'fbx';
  state.currentSceneDocument = triangle;
  const routed = await runExport({
    format: 'glb', includeNormals: true, includeUvs: true, useDraco: true, encodingSpeed: 5,
  });
  assert.equal(routed.result.draco_stats?.primitives, 1, 'the FBX-to-GLB route must honour the checkbox');

  // The slider has to reach the encoder across its whole range, not just above
  // the middle. Draco resolves its two speeds as `max(encoding, decoding)`, so
  // while this route pinned the decoding speed at 5, every position from 0 to 5
  // arrived as 5 and the compression half of the control did nothing.
  //
  // Asserted on the arguments rather than the bytes, because a triangle is too
  // small for the speeds to separate; the payload comparison lives in the
  // browser suite where a real model is loaded.
  const seen: number[][] = [];
  const realCompress = gltfModule.GltfAsset.prototype.compressPrimitive;
  gltfModule.GltfAsset.prototype.compressPrimitive = function (this: unknown, ...args: number[]) {
    seen.push(args.slice(2));
    return realCompress.apply(this, args);
  };
  try {
    exportSceneDocumentToGlb(triangle, {
      useDraco: true, encodingSpeed: 0,
      positionBits: 14, normalBits: 10, texcoordBits: 12, colorBits: 10, genericBits: 12,
    });
  } finally {
    gltfModule.GltfAsset.prototype.compressPrimitive = realCompress;
  }
  assert.deepEqual(
    seen,
    [[0, 0, 14, 10, 12, 10, 12]],
    'both speeds must carry the slider, and every quantization control must reach the encoder',
  );

  // Quantization is the dominant term in the size of a compressed glTF: without
  // it an attribute never reaches Draco's integer coder, so no prediction scheme
  // runs and the encoding speed changes nothing either. This route used to pass
  // none at all.
  const gridSize = 40;
  const gridPositions: number[] = [];
  for (let y = 0; y < gridSize; y += 1) {
    for (let x = 0; x < gridSize; x += 1) {
      gridPositions.push(x, y, Math.sin(x * 0.3) * Math.cos(y * 0.3));
    }
  }
  const gridIndices: number[] = [];
  for (let y = 0; y < gridSize - 1; y += 1) {
    for (let x = 0; x < gridSize - 1; x += 1) {
      const i = y * gridSize + x;
      gridIndices.push(i, i + 1, i + gridSize, i + 1, i + gridSize + 1, i + gridSize);
    }
  }
  const grid = createSceneDocument({
    accessors: [
      {
        bytes: new Uint8Array(new Float32Array(gridPositions).buffer),
        componentType: 5126,
        components: 3,
        count: gridPositions.length / 3,
        min: [0, 0, -1],
        max: [gridSize, gridSize, 1],
      },
      {
        bytes: new Uint8Array(new Uint32Array(gridIndices).buffer),
        componentType: 5125,
        components: 1,
        count: gridIndices.length,
      },
    ] as any,
    meshes: [{ primitives: [{ attributes: { POSITION: 0 }, indices: 1 }] }] as any,
    nodes: [{ name: 'grid', mesh: 0 }] as any,
    rootNodes: [0],
  });

  const quantized = exportSceneDocumentToGlb(grid, {
    useDraco: true, encodingSpeed: 4,
    positionBits: 11, normalBits: 8, texcoordBits: 10, colorBits: 8, genericBits: 8,
  });
  const unquantized = exportSceneDocumentToGlb(grid, {
    useDraco: true, encodingSpeed: 4,
    positionBits: 0, normalBits: 0, texcoordBits: 0, colorBits: 0, genericBits: 0,
  });
  assert.ok(
    quantized.result.draco_stats!.compressed_size * 2 < unquantized.result.draco_stats!.compressed_size,
    'quantization must reach the glTF Draco pass: '
      + `${quantized.result.draco_stats!.compressed_size} vs ${unquantized.result.draco_stats!.compressed_size}`,
  );
} else {
  console.log('export-branches: Draco leg skipped (this WASM profile has no encoder)');
}

console.log('export branch helpers passed');
