// Decoding off the calling thread, on the shared pool: decode_draco on a
// worker, with the stream copied over unless the caller hands it away.
import { createPool as createTaskPool } from './pool-core.js';

const bytesOf = (data) =>
  ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : new Uint8Array(data);

export function createDecoderPool(options = {}) {
  const pool = createTaskPool(options);
  return {
    decode(data, attributes, { transfer = false, signal, ...decodeOptions } = {}) {
      const wasmOptions = Object.keys(decodeOptions).length > 0 ? decodeOptions : undefined;
      return pool.run('decode_draco', [bytesOf(data), attributes, wasmOptions], { transfer, signal });
    },
    preload: () => pool.preload(),
    setWorkerLimit: (workers) => pool.setWorkerLimit(workers),
    get workerLimit() {
      return pool.workerLimit;
    },
    dispose: () => pool.dispose(),
  };
}

export { createDecoderPool as createPool };
