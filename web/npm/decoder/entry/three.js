// three.js's DRACOLoader, decoding on this entry's worker pool. Everything a
// caller sees -- load, parse, decodeDracoFile, GLTFLoader's use of it, the
// geometry it builds -- is three's own class; only the decode is replaced, so
// the result follows whichever three the application has.
import { REVISION } from 'three';
import { DRACOLoader as ThreeDRACOLoader } from 'three/addons/loaders/DRACOLoader.js';
import { createDecoderPool } from './pool.js';

// From r181 DRACOLoader pads an attribute whose items are not a multiple of
// four bytes and builds it interleaved, from the stride its worker reports.
const PADS = Number.parseInt(REVISION, 10) >= 181;

function padded(array, itemSize) {
  const bytes = array.BYTES_PER_ELEMENT;
  const stride = (Math.ceil((itemSize * bytes) / 4) * 4) / bytes;
  if (!PADS || stride === itemSize) return { array, stride: itemSize };
  const count = array.length / itemSize;
  const out = new array.constructor(count * stride);
  for (let i = 0; i < count; i += 1) {
    out.set(array.subarray(i * itemSize, (i + 1) * itemSize), i * stride);
  }
  return { array: out, stride };
}

export class DRACOLoader extends ThreeDRACOLoader {
  #tasks = new WeakMap();

  constructor(manager, options = {}) {
    super(manager);
    const { pool, decode_options = {}, ...poolOptions } = options;
    this.decodeOptions = decode_options;
    this.pool = pool ?? createDecoderPool(poolOptions);
  }

  // The buffer goes to a worker by transfer, which leaves nothing here to
  // decode again; a second request for it gets the first answer, as three's
  // own task cache gives it.
  decodeGeometry(buffer, taskConfig) {
    const key = JSON.stringify(taskConfig);
    const cached = this.#tasks.get(buffer);
    if (cached?.key === key) return cached.promise;
    if (cached && buffer.byteLength === 0) {
      return Promise.reject(new Error('THREE.DRACOLoader: Unable to re-decode a buffer with different settings. Buffer has already been transferred.'));
    }
    const { attributeIDs, attributeTypes, useUniqueIDs, vertexColorSpace } = taskConfig;
    const requests = Object.keys(attributeIDs).map((name) => ({
      name,
      ...(useUniqueIDs ? { id: attributeIDs[name] } : { semantic: attributeIDs[name] }),
      type: attributeTypes[name],
    }));
    const promise = this.pool.decode(buffer, requests, {
      ...this.decodeOptions,
      transfer: buffer instanceof ArrayBuffer,
    }).then((result) => {
      if (!result.success) throw new Error(`THREE.DRACOLoader: Decoding failed: ${result.error}`);
      return this._createGeometry({
        index: result.index ? { array: result.index, itemSize: 1 } : null,
        attributes: result.attributes.map(({ name, array, itemSize }) => ({
          name,
          itemSize,
          ...padded(array, itemSize),
          ...(name === 'color' ? { vertexColorSpace } : {}),
        })),
      });
    });
    if (buffer instanceof ArrayBuffer) this.#tasks.set(buffer, { key, promise });
    return promise;
  }

  setWorkerLimit(workerLimit) {
    super.setWorkerLimit(workerLimit);
    this.pool.setWorkerLimit(workerLimit);
    return this;
  }

  preload() {
    // A failure shows again on the first decode, which is where it is handled.
    this.pool.preload().catch(() => {});
    return this;
  }

  dispose() {
    super.dispose();
    this.pool.dispose();
    return this;
  }
}
