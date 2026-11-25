import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { validateBytes } from 'gltf-validator';

import { decodeFirstDracoPrimitive } from './draco-interop.ts';

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');
const api = await import(pathToFileURL(resolve(pkg, 'gltf.js')).href) as any;
const wasm = await readFile(resolve(pkg, 'gltf_bg.wasm'));
await api.default({ module_or_path: wasm });

const values = new Float32Array([
  0, 0, 0,
  1, 0, 0,
  0, 1, 0,
]);
const positions = new Uint8Array(values.buffer.slice(0));
const indices = new Uint8Array(new Uint16Array([0, 1, 2]).buffer);
const geometry = new api.PackedGeometry(4);
geometry.addAttribute('POSITION', 3, 3, 5126, false, positions);
geometry.setIndices(3, 5123, indices);
geometry.validate('2.0');

const options = new api.GeometryWriteOptions();
const draco = process.argv.includes('--draco');
if (draco) {
  if (typeof options.useDraco !== 'function') {
    throw new Error('Draco writer method is missing');
  }
  options.useDraco(5, 5, false);
}
const document = api.GltfAsset.fromGeometry(geometry, '2.0', options);
const glb = document.glb(2);
const validation = await validateBytes(glb, { uri: 'packed-geometry.glb' });
if (validation.issues.numErrors !== 0) {
  throw new Error(`generated GLB is invalid: ${JSON.stringify(validation.issues.messages)}`);
}
const reloaded = new api.GltfAsset(glb, '2.0');
const packed = reloaded.readPrimitive(0, 0);
if (
  packed.attributeSemantic(0) !== 'POSITION'
  || packed.attributeElementCount(0) !== 3
  || packed.attributeBytes(0).length !== 36
  || packed.indexCount() !== 3
) {
  throw new Error('glTF write roundtrip failed');
}
if (draco) {
  const decoded = await decodeFirstDracoPrimitive(glb);
  if (
    decoded.points !== 3
    || decoded.faces !== 1
    || decoded.declaredPoints !== decoded.points
    || decoded.declaredIndices !== decoded.faces * 3
  ) {
    throw new Error(`official draco3d metadata mismatch: ${JSON.stringify(decoded)}`);
  }
}

if (draco) {
  const advanced = new api.GeometryWriteOptions();
  advanced.useDraco(5, 5, false, {
    position_bits: 14,
    position_quantization_grid: { origin: [0, 0, 0], range: 1 },
    max_output_bytes: 1024,
  });
  const advancedAsset = api.GltfAsset.fromGeometry(geometry, '2.0', advanced);
  const advancedGeometry = advancedAsset.readPrimitive(0, 0);
  if (advancedGeometry.attributeElementCount(0) !== 3) {
    throw new Error('advanced GeometryWriteOptions Draco roundtrip failed');
  }

  const capped = new api.GeometryWriteOptions();
  capped.useDraco(5, 5, false, { max_output_bytes: 1 });
  let capRefused = false;
  try {
    api.GltfAsset.fromGeometry(geometry, '2.0', capped);
  } catch (error) {
    capRefused = /limit/i.test(String(error));
  }
  if (!capRefused) throw new Error('max_output_bytes did not reject an oversized glTF result');

  const raw = api.GltfAsset.fromGeometry(geometry, '2.0', new api.GeometryWriteOptions());
  raw.compressPrimitive(0, 0, 5, 5, 14, 0, 0, 0, 0, 0, {
    position_quantization_grid: { origin: [0, 0, 0], range: 1 },
    max_output_bytes: 1024,
  });
  const direct = raw.readPrimitive(0, 0);
  if (direct.attributeElementCount(0) !== 3) {
    throw new Error('advanced compressPrimitive Draco roundtrip failed');
  }

  const points = new api.PackedGeometry(0);
  points.addAttribute('POSITION', 3, 3, 5126, false, positions);
  points.validate('2.0');
  const pointAsset = api.GltfAsset.fromGeometry(
    points,
    '2.0',
    new api.GeometryWriteOptions(),
  );
  let pointsRefused = false;
  try {
    pointAsset.compressPrimitive(0, 0, 5, 5, 14, 0, 0, 0, 0, 0);
  } catch (error) {
    pointsRefused = /allow_points/i.test(String(error));
  }
  if (!pointsRefused) throw new Error('POINTS compression was not opt-in');
  const pointReport = pointAsset.compressPrimitive(0, 0, 5, 5, 14, 0, 0, 0, 0, 0, {
    allow_points: true,
    prediction_search: true,
    point_order_search: true,
  });
  if (pointReport.method !== 'kd-tree') {
    throw new Error(`Expected KD-tree compression report, got ${pointReport.method}`);
  }
  const pointGeometry = pointAsset.readPrimitive(0, 0);
  if (pointGeometry.mode() !== 0 || pointGeometry.attributeElementCount(0) !== 3) {
    throw new Error('opt-in Draco point-cloud roundtrip failed');
  }

  const splat = new api.PackedGeometry(0);
  const prefix = 'KHR_gaussian_splatting:';
  const rotation = `${prefix}ROTATION`;
  const opacity = `${prefix}OPACITY`;
  const sh = `${prefix}SH_DEGREE_0_COEF_0`;
  const rotations = new Float32Array([Math.SQRT1_2, 0, 0, Math.SQRT1_2, 0, 0, 0, 1, 1, 0, 0, 0]);
  const shValues = new Float32Array([-0.22, -0.22, -0.22, 0, 0, 0, 0.25, 0.25, 0.25]);
  splat.addAttribute('POSITION', 3, 3, 5126, false, positions);
  splat.addAttribute(rotation, 3, 4, 5126, false, new Uint8Array(rotations.buffer));
  splat.addAttribute(`${prefix}SCALE`, 3, 3, 5121, false, new Uint8Array(9).fill(1));
  splat.addAttribute(opacity, 3, 1, 5126, false, new Uint8Array(new Float32Array([0.0013, 0.5, 1]).buffer));
  splat.addAttribute(sh, 3, 3, 5126, false, new Uint8Array(shValues.buffer));
  const splatOptions = {
    allow_points: true,
    generic_bits: 8,
    gaussian_splat_quantization: {
      scale_bits: 8,
      sh_degrees: [{ bits: 3, max_abs: 0.25 }],
    },
  };
  const splatWriter = new api.GeometryWriteOptions();
  splatWriter.useDraco(5, 5, false, splatOptions);
  const writtenSplat = api.GltfAsset.fromGeometry(splat, '2.0', splatWriter);
  const compressedSplat = api.GltfAsset.fromGeometry(splat, '2.0', new api.GeometryWriteOptions());
  compressedSplat.compressPrimitive(0, 0, 5, 5, 14, 0, 0, 0, 0, 0, {
    ...splatOptions,
    gaussian_splat_quantization: {
      ...splatOptions.gaussian_splat_quantization,
      sh_coefficients: { [sh]: 'preserve' },
    },
  });
  for (const asset of [writtenSplat, compressedSplat]) {
    const decoded = new api.GltfAsset(asset.glb(2), '2.0').readPrimitive(0, 0);
    const readFloats = (semantic: string) => {
      for (let i = 0; i < decoded.attributeCount(); i++) {
        if (decoded.attributeSemantic(i) === semantic) {
          return new Float32Array(Uint8Array.from(decoded.attributeBytes(i)).buffer);
        }
      }
      throw new Error(`Missing splat attribute ${semantic}`);
    };
    const coefficients = readFloats(sh);
    if (asset === writtenSplat && (coefficients[0] === shValues[0] || coefficients[3] !== 0)) {
      throw new Error('SH grid did not quantize while preserving zero');
    }
    if (asset === compressedSplat && coefficients.some((value, i) => value !== shValues[i])) {
      throw new Error('SH coefficient override did not preserve the original data');
    }
    if (readFloats(rotation).some((value, i) => value !== rotations[i])) {
      throw new Error('Splat rotation did not retain Float32 precision');
    }
    if (Math.max(...readFloats(opacity)) !== 1) {
      throw new Error('Quantized splat opacity did not preserve the upper endpoint');
    }
  }
}
const bundle = document.gltfBundle();
if (bundle.resourceCount() !== 1 || bundle.resourceBytes(0).length === 0) {
  throw new Error('glTF JSON bundle is incomplete');
}
console.log(`glTF ${draco ? 'Draco' : 'raw'} writer smoke passed`);
