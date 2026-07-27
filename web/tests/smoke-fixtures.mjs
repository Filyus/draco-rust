import { deflateSync } from 'node:zlib';

export const TRIANGLE_BASE64 = 'AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA';

export function triangleBytes() {
  const binary = atob(TRIANGLE_BASE64);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

export function embeddedTriangle() {
  return JSON.stringify({
    asset: { version: '2.0' },
    buffers: [{
      byteLength: 36,
      uri: `data:application/octet-stream;base64,${TRIANGLE_BASE64}`,
    }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }],
    accessors: [{
      bufferView: 0,
      componentType: 5126,
      count: 3,
      type: 'VEC3',
      min: [0, 0, 0],
      max: [1, 1, 0],
    }],
    meshes: [{
      name: 'Triangle',
      primitives: [{ attributes: { POSITION: 0 } }],
    }],
    nodes: [{ mesh: 0 }],
    scenes: [{ nodes: [0] }],
    scene: 0,
  });
}

export function externalTriangle() {
  return JSON.stringify({
    asset: { version: '2.0' },
    buffers: [{ uri: 'missing.bin', byteLength: 36 }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }],
    accessors: [{
      bufferView: 0,
      componentType: 5126,
      count: 3,
      type: 'VEC3',
      min: [0, 0, 0],
      max: [1, 1, 0],
    }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }],
  });
}

/** CRC-32, the one checksum every PNG chunk carries. */
function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngChunk(type, data) {
  const header = Buffer.alloc(8);
  header.writeUInt32BE(data.length, 0);
  header.write(type, 4, 'ascii');
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([header.subarray(4), data])), 0);
  return Buffer.concat([header, data, crc]);
}

/**
 * An 8x1 truecolour PNG of alternating tangent-space normals.
 *
 * Written by hand rather than pulled from a fixture file so the stripe pattern
 * the assertions depend on is visible right here: columns tilt about 75 degrees
 * either way around the V axis. The preview lights surfaces from a smooth
 * environment map, which flattens gentle perturbations, so the fixture leans on
 * a steep tilt to make "the map is applied" separable from "it is not".
 */
export function stripeNormalMapPng() {
  const width = 8;
  const tilted = [251, 128, 161];
  const counterTilted = [5, 128, 161];
  const row = Buffer.alloc(1 + width * 3);
  for (let x = 0; x < width; x += 1) {
    const [r, g, b] = x % 2 === 0 ? tilted : counterTilted;
    row[1 + x * 3] = r;
    row[2 + x * 3] = g;
    row[3 + x * 3] = b;
  }
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(1, 4);
  header[8] = 8; // bit depth
  header[9] = 2; // truecolour
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk('IHDR', header),
    pngChunk('IDAT', deflateSync(row)),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}

/**
 * A textured quad, optionally normal-mapped, scaled down to `scale`.
 *
 * The scale is the point of the fixture: a tangent frame derived from
 * screen-space derivatives works in world units per pixel, so a centimetre-
 * sized model produces gradients orders of magnitude smaller than a
 * metre-sized one. Rendering the same quad with and without the normal map
 * shows whether the map reaches the surface at all.
 */
export function normalMappedQuad({ normalMap = true, scale = 0.02 } = {}) {
  const positions = new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]);
  const normals = new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]);
  const uvs = new Float32Array([0, 1, 1, 1, 1, 0, 0, 0]);
  const indices = new Uint16Array([0, 1, 2, 0, 2, 3]);
  const binary = Buffer.concat([
    Buffer.from(positions.buffer),
    Buffer.from(normals.buffer),
    Buffer.from(uvs.buffer),
    Buffer.from(indices.buffer),
  ]);
  const material = {
    pbrMetallicRoughness: { baseColorFactor: [0.8, 0.8, 0.8, 1], metallicFactor: 0, roughnessFactor: 0.3 },
    ...(normalMap ? { normalTexture: { index: 0 } } : {}),
  };
  return JSON.stringify({
    asset: { version: '2.0' },
    buffers: [{ byteLength: binary.length, uri: `data:application/octet-stream;base64,${binary.toString('base64')}` }],
    bufferViews: [
      { buffer: 0, byteOffset: 0, byteLength: 48 },
      { buffer: 0, byteOffset: 48, byteLength: 48 },
      { buffer: 0, byteOffset: 96, byteLength: 32 },
      { buffer: 0, byteOffset: 128, byteLength: 12 },
    ],
    accessors: [
      { bufferView: 0, componentType: 5126, count: 4, type: 'VEC3', min: [-1, -1, 0], max: [1, 1, 0] },
      { bufferView: 1, componentType: 5126, count: 4, type: 'VEC3' },
      { bufferView: 2, componentType: 5126, count: 4, type: 'VEC2' },
      { bufferView: 3, componentType: 5123, count: 6, type: 'SCALAR' },
    ],
    images: [{
      mimeType: 'image/png',
      uri: `data:image/png;base64,${stripeNormalMapPng().toString('base64')}`,
    }],
    textures: [{ source: 0, sampler: 0 }],
    samplers: [{ wrapS: 10497, wrapT: 10497, minFilter: 9729, magFilter: 9729 }],
    materials: [material],
    meshes: [{
      primitives: [{ attributes: { POSITION: 0, NORMAL: 1, TEXCOORD_0: 2 }, indices: 3, material: 0 }],
    }],
    nodes: [{ mesh: 0, scale: [scale, scale, scale] }],
    scenes: [{ nodes: [0] }],
    scene: 0,
  });
}

/**
 * A 1x1 truecolour PNG of one flat colour, default pure green.
 *
 * One texel is enough for what it is used for: telling a decoded image apart
 * from the opaque white texel every texture is uploaded with before its bitmap
 * arrives. Green, because that placeholder is neutral and a channel that only
 * the decoded image can raise is what makes the two separable.
 */
export function solidColorPng([red, green, blue] = [0, 255, 0]) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(1, 0);
  header.writeUInt32BE(1, 4);
  header[8] = 8; // bit depth
  header[9] = 2; // truecolour
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk('IHDR', header),
    pngChunk('IDAT', deflateSync(Buffer.from([0, red, green, blue]))),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}

/** A 2x1 truecolour PNG: red on the left half, green on the right. */
function splitColorPng() {
  const row = Buffer.from([0, 255, 0, 0, 0, 255, 0]);
  const header = Buffer.alloc(13);
  header.writeUInt32BE(2, 0);
  header.writeUInt32BE(1, 4);
  header[8] = 8; // bit depth
  header[9] = 2; // truecolour
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    pngChunk('IHDR', header),
    pngChunk('IDAT', deflateSync(row)),
    pngChunk('IEND', Buffer.alloc(0)),
  ]);
}

/**
 * A quad whose emissive texture is squeezed onto one half of a two-colour map
 * by `KHR_texture_transform`.
 *
 * The emissive slot is chosen because it is additive and independent of the
 * environment: whatever the lighting does, the surface reads as the colour the
 * transform selected. Both variants scale U by 0.5, so each samples exactly one
 * half of the map and the frame is a single flat colour — red at offset 0,
 * green at offset 0.5. A renderer that ignores the transform on this slot
 * produces the same half-red half-green frame for both, which is what makes the
 * pair separable rather than merely different.
 */
export function emissiveTransformQuad({ offset = 0 } = {}) {
  const positions = new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]);
  const normals = new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]);
  const uvs = new Float32Array([0, 1, 1, 1, 1, 0, 0, 0]);
  const indices = new Uint16Array([0, 1, 2, 0, 2, 3]);
  const binary = Buffer.concat([
    Buffer.from(positions.buffer),
    Buffer.from(normals.buffer),
    Buffer.from(uvs.buffer),
    Buffer.from(indices.buffer),
  ]);
  return JSON.stringify({
    asset: { version: '2.0' },
    extensionsUsed: ['KHR_texture_transform'],
    buffers: [{ byteLength: binary.length, uri: `data:application/octet-stream;base64,${binary.toString('base64')}` }],
    bufferViews: [
      { buffer: 0, byteOffset: 0, byteLength: 48 },
      { buffer: 0, byteOffset: 48, byteLength: 48 },
      { buffer: 0, byteOffset: 96, byteLength: 32 },
      { buffer: 0, byteOffset: 128, byteLength: 12 },
    ],
    accessors: [
      { bufferView: 0, componentType: 5126, count: 4, type: 'VEC3', min: [-1, -1, 0], max: [1, 1, 0] },
      { bufferView: 1, componentType: 5126, count: 4, type: 'VEC3' },
      { bufferView: 2, componentType: 5126, count: 4, type: 'VEC2' },
      { bufferView: 3, componentType: 5123, count: 6, type: 'SCALAR' },
    ],
    images: [{
      mimeType: 'image/png',
      uri: `data:image/png;base64,${splitColorPng().toString('base64')}`,
    }],
    textures: [{ source: 0, sampler: 0 }],
    // Nearest and clamped: the assertion is about which half is sampled, not
    // about how the seam between them filters.
    samplers: [{ wrapS: 33071, wrapT: 33071, minFilter: 9728, magFilter: 9728 }],
    materials: [{
      pbrMetallicRoughness: { baseColorFactor: [0, 0, 0, 1], metallicFactor: 0, roughnessFactor: 1 },
      emissiveFactor: [1, 1, 1],
      emissiveTexture: {
        index: 0,
        extensions: { KHR_texture_transform: { offset: [offset, 0], scale: [0.5, 1] } },
      },
    }],
    meshes: [{
      primitives: [{ attributes: { POSITION: 0, NORMAL: 1, TEXCOORD_0: 2 }, indices: 3, material: 0 }],
    }],
    nodes: [{ mesh: 0 }],
    scenes: [{ nodes: [0] }],
    scene: 0,
  });
}

export function animatedTranslation() {
  return JSON.stringify({
    asset: { version: '2.0' },
    buffers: [{
      byteLength: 32,
      uri: 'data:application/octet-stream;base64,AAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAQAAAQEA=',
    }],
    bufferViews: [
      { buffer: 0, byteOffset: 0, byteLength: 8 },
      { buffer: 0, byteOffset: 8, byteLength: 24 },
    ],
    accessors: [
      { bufferView: 0, componentType: 5126, count: 2, type: 'SCALAR' },
      { bufferView: 1, componentType: 5126, count: 2, type: 'VEC3' },
    ],
    nodes: [{}],
    scenes: [{ nodes: [0] }],
    scene: 0,
    animations: [{
      samplers: [{ input: 0, output: 1, interpolation: 'LINEAR' }],
      channels: [{ sampler: 0, target: { node: 0, path: 'translation' } }],
    }],
  });
}
