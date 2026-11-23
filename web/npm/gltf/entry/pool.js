// glTF reading off the calling thread, on the shared pool.
import { createPool as createTaskPool } from './pool-core.js';

const bytesOf = (data) =>
  ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : new Uint8Array(data);

export function createPool(options = {}) {
  const pool = createTaskPool(options);
  return {
    read: (data, { resources, profile = '2.0', decode_options, transfer = false, signal } = {}) =>
      pool.run('read_gltf', [bytesOf(data), resources, profile, decode_options], { transfer, signal }),
    inspect: (data, { transfer = false, signal } = {}) => pool.run('inspect_gltf', [bytesOf(data)], { transfer, signal }),
    preload: () => pool.preload(),
    setWorkerLimit: (workers) => pool.setWorkerLimit(workers),
    get workerLimit() {
      return pool.workerLimit;
    },
    dispose: () => pool.dispose(),
  };
}
