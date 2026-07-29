import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  animatedTranslation, embeddedTriangle, externalTriangle, triangleBytes,
} from './smoke-fixtures.ts';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

async function isReleaseProfile() {
  const stamp = JSON.parse(await readFile(resolve(pkg, 'gltf.build-stamp.json'), 'utf8'));
  return stamp.config_key.includes('features=;');
}

async function load(name: string) {
  const module = await import(pathToFileURL(resolve(pkg, `${name}.js`)).href);
  const wasm = await readFile(resolve(pkg, `${name}_bg.wasm`));
  await module.default({ module_or_path: wasm });
  return module;
}

const api = await load('gltf');
const input = new TextEncoder().encode(embeddedTriangle());
const asset = new api.GltfAsset(input, '2.0');
const releaseProfile = await isReleaseProfile();
if (releaseProfile && (
  typeof asset.decompress === 'function'
  || typeof asset.compressPrimitive === 'function'
  || typeof asset.readAccessor === 'function'
  || typeof asset.bufferViewBytes === 'function'
  || typeof asset.previewManifest === 'function'
  || api.GltfAsset.prototype.decompress
  || api.GltfAsset.prototype.compressPrimitive
  || api.GltfAsset.prototype.readAccessor
  || api.GltfAsset.prototype.bufferViewBytes
  || api.GltfAsset.prototype.previewManifest
)) {
  throw new Error('default glTF artifact unexpectedly includes renderer or writer API');
}
const types = await readFile(resolve(pkg, 'gltf.d.ts'), 'utf8');
if (releaseProfile && (
  types.includes('decompress')
  || types.includes('compressPrimitive')
  || types.includes('readAccessor')
  || types.includes('bufferViewBytes')
  || types.includes('previewManifest')
)) {
  throw new Error('default glTF declarations unexpectedly include renderer or writer API');
}
const summary = asset.summary();
if (!summary.success || summary.meshCount !== 1 || summary.primitiveCount !== 1) {
  throw new Error(`glTF asset smoke failed: ${JSON.stringify(summary)}`);
}

const primitive = asset.readPrimitive(0, 0);
if (releaseProfile && ('GeometryWriteOptions' in api || typeof api.GltfAsset.fromGeometry === 'function')) {
  throw new Error('default glTF artifact unexpectedly includes writer API');
}
if (
  asset.meshCount() !== 1
  || asset.primitiveCount(0) !== 1
  || primitive.attributeCount() !== 1
  || primitive.attributeSemantic(0) !== 'POSITION'
  || primitive.attributeBytes(0).length !== 36
) {
  throw new Error('glTF geometry smoke failed');
}

const glb = asset.glb(2);
const roundtrip = new api.GltfAsset(glb, '2.0').summary();
if (!roundtrip.success || roundtrip.meshCount !== 1) {
  throw new Error(`GLB roundtrip failed: ${JSON.stringify(roundtrip)}`);
}

let missingFailed = false;
try {
  new api.GltfAsset(new TextEncoder().encode(externalTriangle()), '2.0');
} catch (error) {
  missingFailed = String(error).includes('missing.bin');
}
if (!missingFailed) {
  throw new Error('missing-resource smoke failed');
}
const resolved = api.GltfAsset.withResources(
  new TextEncoder().encode(externalTriangle()),
  { 'missing.bin': triangleBytes() },
  '2.0',
).summary();
if (!resolved.success || resolved.meshCount !== 1) {
    throw new Error(`resource-map smoke failed: ${JSON.stringify(resolved)}`);
}

const fbx = await load('fbx');
const fbxScene = {
  rootNodes: [{
    name: 'AnimatedTriangle',
    meshes: [{
      name: 'Triangle',
      positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
      indices: [0, 1, 2],
      uvs: [0, 0, 1, 0, 0, 1],
      materialIndices: [1],
    }],
    children: [],
  }],
  materials: [{
    name: 'Red',
    shadingModel: 'Phong',
    diffuse: [1, 0, 0],
    shininess: 20,
  }, {
    name: 'Blue',
    shadingModel: 'Phong',
    diffuse: [0, 0, 1],
  }],
  animations: [{
    name: 'Move',
    duration: 1,
    channels: [{
      nodeName: 'AnimatedTriangle',
      path: 'translation',
      sampler: {
        input: [0, 1],
        output: [0, 0, 0, 1, 0, 0],
        interpolation: 'linear',
      },
    }],
  }],
};
const fbxExport = fbx.create_fbx_scene(fbxScene, {});
if (!fbxExport.success || !fbxExport.binary_data?.length) {
  throw new Error(`FBX scene export smoke failed: ${fbxExport.error}`);
}
const fbxRoundtrip = fbx.parse_fbx(fbxExport.binary_data);
if (
  !fbxRoundtrip.success
  || fbxRoundtrip.materials?.[1]?.name !== 'Blue'
  || fbxRoundtrip.scene?.rootNodes?.[0]?.meshes?.[0]?.materialIndices?.[0] !== 1
  || fbxRoundtrip.scene?.rootNodes?.[0]?.meshes?.[0]?.uvs?.length !== 6
  || fbxRoundtrip.animations?.[0]?.channels?.length !== 1
) {
  throw new Error(`FBX material/animation smoke failed: ${JSON.stringify(fbxRoundtrip)}`);
}

// FBX export from glTF must carry the document's material assignments and
// node animation, rather than only the hierarchy and triangle buffers.
(globalThis as any).WebGL2RenderingContext = {};
const { buildFbxSceneFromGltf } = await import('../src/gltf-loader.ts');
const animatedDocument = JSON.parse(animatedTranslation());
animatedDocument.nodes[0] = { name: 'AnimatedTriangle' };
const animatedFbxScene = buildFbxSceneFromGltf(
  new TextEncoder().encode(JSON.stringify(animatedDocument)),
  {},
  api,
);
const materialDocument = JSON.parse(embeddedTriangle());
materialDocument.materials = [{
  name: 'Blue',
  pbrMetallicRoughness: { baseColorFactor: [0, 0, 1, 1] },
}];
materialDocument.meshes[0].primitives[0].material = 0;
const materialFbxScene = buildFbxSceneFromGltf(
  new TextEncoder().encode(JSON.stringify(materialDocument)),
  {},
  api,
);
if (
  materialFbxScene.materials?.[0]?.name !== 'Blue'
  || materialFbxScene.rootNodes?.[0]?.meshes?.[0]?.materialIndices?.[0] !== 0
  || animatedFbxScene.animations?.[0]?.channels?.[0]?.path !== 'translation'
) {
  throw new Error('glTF to FBX scene conversion smoke failed');
}
const materialFbxRoundtrip = fbx.parse_fbx(
  fbx.create_fbx_scene(materialFbxScene, {}).binary_data,
);
const animatedFbxRoundtrip = fbx.parse_fbx(
  fbx.create_fbx_scene(animatedFbxScene, {}).binary_data,
);
if (
  materialFbxRoundtrip.materials?.[0]?.name !== 'Blue'
  || materialFbxRoundtrip.scene?.rootNodes?.[0]?.meshes?.[0]?.materialIndices?.[0] !== 0
  || animatedFbxRoundtrip.animations?.[0]?.channels?.[0]?.path !== 'translation'
) {
  throw new Error('glTF to FBX exported data smoke failed');
}
console.log('Node WASM smoke passed');
