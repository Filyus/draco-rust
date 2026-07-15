import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';

// Resolve CI-only validator/decoder dependencies from the single web package.
const require = createRequire(new URL('../../web/package.json', import.meta.url));
const draco3d = require('draco3d');
const validator = require('gltf-validator');

const [outputPath, sourcePath] = process.argv.slice(2);
if (!outputPath || !sourcePath) {
  throw new Error('usage: node gltf-interop.mjs <rust-output.glb> <source.gltf>');
}

const JSON_CHUNK = 0x4e4f534a;
const BIN_CHUNK = 0x004e4942;
const GLB_MAGIC = 0x46546c67;

function checkedRange(bytes, offset, length, label) {
  if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(length) || offset < 0 || length < 0 || offset + length > bytes.length) {
    throw new Error(`${label} range ${offset}..${offset + length} exceeds ${bytes.length} bytes`);
  }
  return bytes.subarray(offset, offset + length);
}

function parseGlb(bytes) {
  if (bytes.length < 12) throw new Error('GLB header is truncated');
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (view.getUint32(0, true) !== GLB_MAGIC) throw new Error('Rust output is not GLB');
  if (view.getUint32(4, true) !== 2) throw new Error('Rust output is not GLB 2.0');
  if (view.getUint32(8, true) !== bytes.length) throw new Error('GLB declared length mismatch');

  let offset = 12;
  let json;
  let bin;
  while (offset < bytes.length) {
    if (offset + 8 > bytes.length) throw new Error('GLB chunk header is truncated');
    const length = view.getUint32(offset, true);
    const type = view.getUint32(offset + 4, true);
    const chunk = checkedRange(bytes, offset + 8, length, 'GLB chunk');
    if (type === JSON_CHUNK) {
      if (json !== undefined) throw new Error('GLB has multiple JSON chunks');
      const text = new TextDecoder().decode(chunk).replace(/[\u0000 ]+$/u, '');
      json = JSON.parse(text);
    } else if (type === BIN_CHUNK) {
      if (bin !== undefined) throw new Error('GLB has multiple BIN chunks');
      bin = chunk;
    }
    offset += 8 + length;
  }
  if (offset !== bytes.length || json === undefined) throw new Error('GLB chunk layout is invalid');
  return { json, buffers: [bin ?? new Uint8Array()] };
}

async function loadSource(filename) {
  const json = JSON.parse(await fs.readFile(filename, 'utf8'));
  const base = path.dirname(filename);
  const buffers = await Promise.all((json.buffers ?? []).map(async (buffer, index) => {
    if (typeof buffer.uri !== 'string' || buffer.uri.startsWith('data:')) {
      throw new Error(`source buffer ${index} must be an external fixture file`);
    }
    const decoded = decodeURIComponent(buffer.uri);
    const resolved = path.resolve(base, decoded);
    const data = new Uint8Array(await fs.readFile(resolved));
    if (data.length < buffer.byteLength) throw new Error(`source buffer ${index} is shorter than byteLength`);
    return data;
  }));
  return { json, buffers };
}

const COMPONENTS = { SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4, MAT2: 4, MAT3: 9, MAT4: 16 };
const COMPONENT_TYPES = {
  5121: [1, 'getUint8'],
  5123: [2, 'getUint16'],
  5125: [4, 'getUint32'],
  5126: [4, 'getFloat32'],
};

function readAccessor(document, buffers, accessorIndex) {
  const accessor = document.accessors?.[accessorIndex];
  if (!accessor || !Number.isSafeInteger(accessor.count) || accessor.bufferView === undefined) {
    throw new Error(`accessor ${accessorIndex} is missing or has no bufferView`);
  }
  const componentCount = COMPONENTS[accessor.type];
  const component = COMPONENT_TYPES[accessor.componentType];
  const bufferView = document.bufferViews?.[accessor.bufferView];
  if (!componentCount || !component || !bufferView) throw new Error(`unsupported accessor ${accessorIndex}`);
  const [componentSize, getter] = component;
  const rowSize = componentSize * componentCount;
  const stride = bufferView.byteStride ?? rowSize;
  if (stride < rowSize) throw new Error(`accessor ${accessorIndex} has a short stride`);
  const buffer = buffers[bufferView.buffer ?? 0];
  if (!buffer) throw new Error(`accessor ${accessorIndex} references a missing buffer`);
  const start = (bufferView.byteOffset ?? 0) + (accessor.byteOffset ?? 0);
  const finalEnd = accessor.count === 0 ? start : start + (accessor.count - 1) * stride + rowSize;
  checkedRange(buffer, start, finalEnd - start, `accessor ${accessorIndex}`);
  const data = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength);
  const rows = [];
  for (let row = 0; row < accessor.count; row += 1) {
    const values = [];
    for (let componentIndex = 0; componentIndex < componentCount; componentIndex += 1) {
      const offset = start + row * stride + componentIndex * componentSize;
      values.push(data[getter](offset, true));
    }
    rows.push(values);
  }
  return rows;
}

function coordinateKey(position) {
  return position.map((value) => {
    const rounded = Math.round(value * 1000) / 1000;
    return Object.is(rounded, -0) ? '0.000' : rounded.toFixed(3);
  }).join(',');
}

function orientedTriangleKey(positions, face) {
  const points = face.map((index) => {
    if (!Number.isInteger(index) || index < 0 || index >= positions.length) {
      throw new Error(`face index ${index} is outside ${positions.length} positions`);
    }
    return coordinateKey(positions[index]);
  });
  const rotations = [
    `${points[0]}|${points[1]}|${points[2]}`,
    `${points[1]}|${points[2]}|${points[0]}`,
    `${points[2]}|${points[0]}|${points[1]}`,
  ];
  return rotations.sort()[0];
}

function sourceTopology(source) {
  const result = [];
  for (const mesh of source.json.meshes ?? []) {
    for (const primitive of mesh.primitives ?? []) {
      if ((primitive.mode ?? 4) !== 4) throw new Error('interop source must use TRIANGLES');
      const positions = readAccessor(source.json, source.buffers, primitive.attributes.POSITION);
      const indices = primitive.indices === undefined
        ? positions.map((_, index) => index)
        : readAccessor(source.json, source.buffers, primitive.indices).map(([index]) => index);
      if (indices.length === 0 || indices.length % 3 !== 0) throw new Error('interop source has invalid triangle indices');
      const keys = [];
      for (let i = 0; i < indices.length; i += 3) {
        keys.push(orientedTriangleKey(positions, indices.slice(i, i + 3)));
      }
      result.push(keys.sort());
    }
  }
  return result;
}

async function decodedTopology(output, module) {
  const result = [];
  for (const meshDef of output.json.meshes ?? []) {
    for (const primitive of meshDef.primitives ?? []) {
      const extension = primitive.extensions?.KHR_draco_mesh_compression;
      if (!extension) throw new Error('Rust output contains an uncompressed primitive');
      const bufferView = output.json.bufferViews?.[extension.bufferView];
      const buffer = output.buffers[bufferView?.buffer ?? 0];
      if (!bufferView || !buffer) throw new Error('Draco bufferView is missing');
      const data = checkedRange(buffer, bufferView.byteOffset ?? 0, bufferView.byteLength, 'Draco bufferView');

      const decoderBuffer = new module.DecoderBuffer();
      const decoder = new module.Decoder();
      const decodedMesh = new module.Mesh();
      try {
        decoderBuffer.Init(new Int8Array(data.buffer, data.byteOffset, data.byteLength), data.byteLength);
        if (decoder.GetEncodedGeometryType(decoderBuffer) !== module.TRIANGULAR_MESH) {
          throw new Error('official decoder reports a non-mesh Draco payload');
        }
        const status = decoder.DecodeBufferToMesh(decoderBuffer, decodedMesh);
        if (!status.ok()) throw new Error(`official Draco decode failed: ${status.error_msg()}`);
        if (decodedMesh.num_faces() === 0 || decodedMesh.num_points() === 0) {
          throw new Error('official Draco decoder produced zero faces or points');
        }

        const positionId = extension.attributes?.POSITION;
        if (!Number.isInteger(positionId) || positionId < 0) throw new Error('POSITION unique ID is invalid');
        const attribute = decoder.GetAttributeByUniqueId(decodedMesh, positionId);
        if (!attribute || attribute.ptr === 0) throw new Error(`official decoder cannot find POSITION unique ID ${positionId}`);
        const values = new module.DracoFloat32Array();
        const face = new module.DracoInt32Array();
        try {
          if (!decoder.GetAttributeFloatForAllPoints(decodedMesh, attribute, values)) {
            throw new Error('official decoder could not materialize POSITION values');
          }
          const positions = [];
          for (let point = 0; point < decodedMesh.num_points(); point += 1) {
            positions.push([
              values.GetValue(point * 3),
              values.GetValue(point * 3 + 1),
              values.GetValue(point * 3 + 2),
            ]);
          }
          const keys = [];
          for (let index = 0; index < decodedMesh.num_faces(); index += 1) {
            if (!decoder.GetFaceFromMesh(decodedMesh, index, face)) throw new Error(`official decoder could not read face ${index}`);
            keys.push(orientedTriangleKey(positions, [face.GetValue(0), face.GetValue(1), face.GetValue(2)]));
          }
          result.push(keys.sort());
        } finally {
          module.destroy(face);
          module.destroy(values);
        }
      } finally {
        module.destroy(decodedMesh);
        module.destroy(decoder);
        module.destroy(decoderBuffer);
      }
    }
  }
  return result;
}

const outputBytes = new Uint8Array(await fs.readFile(outputPath));
const validation = await validator.validateBytes(outputBytes, {
  uri: path.basename(outputPath),
  format: 'glb',
  maxIssues: 0,
  writeTimestamp: false,
});
if (validation.issues.numErrors !== 0) {
  throw new Error(`Khronos Validator reported ${validation.issues.numErrors} error(s):\n${JSON.stringify(validation.issues.messages, null, 2)}`);
}

const source = await loadSource(sourcePath);
const output = parseGlb(outputBytes);
const module = await draco3d.createDecoderModule({});
const expected = sourceTopology(source);
const actual = await decodedTopology(output, module);
if (actual.length === 0) throw new Error('Rust output contains zero Draco primitives');
if (JSON.stringify(actual) !== JSON.stringify(expected)) {
  throw new Error(`official decoder topology differs from source (${actual.length} vs ${expected.length} primitives)`);
}

const faces = actual.reduce((count, primitive) => count + primitive.length, 0);
console.log(`Validated ${actual.length} Draco primitive(s), ${faces} oriented triangles with Khronos Validator and Google draco3d.`);
