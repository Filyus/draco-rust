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
    }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }],
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
