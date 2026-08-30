/**
 * One mesh drawn under several materials uploads its vertices once.
 *
 * Splitting a mesh into a primitive per material makes primitives that are the
 * same vertices with different index buffers. Each of them used to upload its
 * own copy of every attribute: one character became thirteen primitives and put
 * 224 MiB on the GPU for 24 MiB of vertices.
 */
import assert from 'node:assert/strict';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const { uploadPrimitive } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'viewer', 'primitive-upload.ts')).href
);

const LOCATIONS = {
  position: 0, normal: 1, texCoord: 2, texCoord1: 6,
  color: 3, joints: 4, weights: 5, smoothNormal: 15,
};

/** Just enough context to record what a primitive asks the GPU to hold. */
function stubGl() {
  const uploads: { target: number; bytes: number; buffer: number }[] = [];
  let nextBuffer = 1;
  let bound = 0;
  const buffersByTarget = new Map<number, number>();
  return {
    uploads,
    gl: {
      ARRAY_BUFFER: 34962,
      ELEMENT_ARRAY_BUFFER: 34963,
      STATIC_DRAW: 35044,
      createVertexArray: () => ({}),
      bindVertexArray: () => {},
      createBuffer: () => nextBuffer++,
      bindBuffer(target: number, buffer: number) { bound = target; buffersByTarget.set(target, buffer); },
      bufferData(target: number, data: ArrayBufferView) {
        uploads.push({ target, bytes: data.byteLength, buffer: buffersByTarget.get(target) ?? 0 });
        void bound;
      },
      enableVertexAttribArray: () => {},
      disableVertexAttribArray: () => {},
      vertexAttribPointer: () => {},
    } as any,
  };
}

const vertexCount = 4;
const shared = {
  POSITION: {
    bytes: new Float32Array(vertexCount * 3), componentType: 5126, components: 3,
    normalized: false, count: vertexCount,
  },
  NORMAL: {
    bytes: new Float32Array(vertexCount * 3), componentType: 5126, components: 3,
    normalized: false, count: vertexCount,
  },
  TEXCOORD_0: {
    bytes: new Float32Array(vertexCount * 2), componentType: 5126, components: 2,
    normalized: false, count: vertexCount,
  },
};

/** Two halves of one quad: the same vertices, an index buffer each. */
function primitiveWith(indices: number[]) {
  return {
    attributes: shared,
    mode: 4,
    materialIndex: 0,
    indices: {
      bytes: Uint32Array.from(indices), componentType: 5125, components: 1,
      normalized: false, count: indices.length,
    },
  };
}

/** Vertex and index uploads two primitives make, with the cache or without. */
function uploadPair(cache: Map<object, unknown> | undefined) {
  const { gl, uploads } = stubGl();
  uploadPrimitive(gl, primitiveWith([0, 1, 2]), LOCATIONS, 256, cache);
  uploadPrimitive(gl, primitiveWith([0, 2, 3]), LOCATIONS, 256, cache);
  return {
    vertex: uploads.filter((u) => u.target === gl.ARRAY_BUFFER),
    index: uploads.filter((u) => u.target === gl.ELEMENT_ARRAY_BUFFER),
  };
}

const alone = uploadPair(undefined);
const together = uploadPair(new Map());

// Stated as the difference rather than a count: the primitive also derives
// attributes of its own -- smooth normals, a renumbered joint palette -- and
// those depend on its indices, so they are its own however the source
// attributes are shared. What the cache removes is the second copy of each
// attribute the two primitives were handed.
assert.equal(
  alone.vertex.length - together.vertex.length,
  Object.keys(shared).length,
  'one copy fewer for each attribute the two primitives share',
);
assert.equal(
  new Set(together.vertex.map((u) => u.buffer)).size,
  together.vertex.length,
  'every upload is its own buffer, so nothing was rewritten in place',
);
assert.equal(together.index.length, 2, 'the index buffers differ, so each is still its own');
assert.deepEqual(
  together.index.map((u) => u.bytes),
  alone.index.map((u) => u.bytes),
  'and they carry what they carried',
);

console.log('viewer-shared-buffers: OK');
