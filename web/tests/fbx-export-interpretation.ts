/**
 * The FBX export route reads glTF the same way everything else does.
 *
 * It used to have its own reader: materials off `pbrMetallicRoughness`,
 * textures off `texture.source`. The visible cost was that an asset whose
 * images come through EXT_texture_webp or KHR_texture_basisu exported with
 * empty texture payloads — the alternate source was simply not looked at —
 * and that nothing told the user which glTF features FBX had left behind.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { stripeNormalMapPng } from './smoke-fixtures.ts';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

const gltfModule = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href);
await gltfModule.default({ module_or_path: await readFile(resolve(pkg, 'gltf_bg.wasm')) });

const { buildFbxSceneFromGltf } = await import(pathToFileURL(resolve(here, '..', 'src', 'gltf-loader.ts')).href);

const geometry = new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]);
const image = `data:image/png;base64,${stripeNormalMapPng().toString('base64')}`;

function asset({ textures, materials, extensionsUsed }: {
  textures: any[];
  materials: any[];
  extensionsUsed?: string[];
}) {
  return new TextEncoder().encode(JSON.stringify({
    asset: { version: '2.0' },
    buffers: [{
      byteLength: geometry.byteLength,
      uri: `data:application/octet-stream;base64,${Buffer.from(geometry.buffer).toString('base64')}`,
    }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }],
    accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] }],
    images: [{ mimeType: 'image/png', uri: image }],
    samplers: [{ wrapS: 33071, wrapT: 33648, minFilter: 9729, magFilter: 9729 }],
    textures,
    materials,
    meshes: [{ primitives: [{ attributes: { POSITION: 0 }, material: 0 }] }],
    nodes: [{ mesh: 0 }],
    scenes: [{ nodes: [0] }],
    scene: 0,
    ...(extensionsUsed ? { extensionsUsed } : {}),
  }));
}

const plainMaterial = { pbrMetallicRoughness: { baseColorTexture: { index: 0 } } };

// An image reached through an alternate-source extension has to be carried.
// The old reader looked only at `texture.source` and wrote an empty payload.
// Every such extension takes the same path, so each is asked rather than one
// standing in for the rest: a name missing from the list reads as an ordinary
// texture with no source, which is silent.
for (const extension of ['EXT_texture_webp', 'EXT_texture_avif', 'KHR_texture_basisu']) {
  const alternate = buildFbxSceneFromGltf(
    asset({
      textures: [{ extensions: { [extension]: { source: 0 } } }],
      materials: [plainMaterial],
      extensionsUsed: [extension],
    }),
    {},
    gltfModule,
  );
  assert.ok(
    alternate.textures[0].content && alternate.textures[0].content.length > 0,
    `a texture whose image comes through ${extension} exported with no payload`,
  );
  assert.ok(
    alternate.warnings.some((warning: string) => /importers cannot decode them/.test(warning)),
    'carrying a codec FBX importers cannot read has to be reported',
  );
}

// Wrap modes, UV transforms and the layered material extensions all have
// nowhere to go in FBX; each has to be named rather than silently dropped.
const rich = buildFbxSceneFromGltf(
  asset({
    textures: [{ source: 0, sampler: 0 }],
    materials: [{
      pbrMetallicRoughness: {
        baseColorTexture: {
          index: 0,
          extensions: { KHR_texture_transform: { offset: [0.5, 0], scale: [2, 2] } },
        },
      },
      extensions: {
        KHR_materials_clearcoat: { clearcoatFactor: 1 },
        KHR_materials_ior: { ior: 1.7 },
      },
    }],
    extensionsUsed: ['KHR_texture_transform', 'KHR_materials_clearcoat', 'KHR_materials_ior'],
  }),
  {},
  gltfModule,
);
assert.ok(rich.warnings.some((warning: string) => /wrap mode/.test(warning)), 'clamp and mirror wraps must be reported');
assert.ok(rich.warnings.some((warning: string) => /UV transform/.test(warning)), 'a dropped texture transform must be reported');
assert.ok(rich.warnings.some((warning: string) => /Phong/.test(warning)), 'dropped material layers must be reported');
assert.ok(
  rich.warnings.some((warning: string) => /FBX writer cannot express/.test(warning)),
  'the extension list FBX cannot express must be reported',
);

// A plain material must stay quiet: warnings that fire on everything are noise.
const plain = buildFbxSceneFromGltf(
  asset({ textures: [{ source: 0 }], materials: [plainMaterial] }),
  {},
  gltfModule,
);
assert.deepEqual(plain.warnings, [], 'an asset FBX can express fully must export without complaint');
assert.equal(plain.materials[0].shadingModel, 'Phong');
assert.deepEqual(plain.materials[0].textures, [{ slot: 'diffuse', textureIndex: 0 }]);

console.log('FBX export interpretation passed');
