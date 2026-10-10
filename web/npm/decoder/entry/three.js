// What three.js's GLTFLoader and DRACOLoader users call, over the decoder pool.
// three.js is not imported: the caller hands in the classes, so the package
// works with whichever copy of three the application bundles.
import { createDecoderPool } from './pool.js';

const SRGB = 'srgb';
const LINEAR_SRGB = 'srgb-linear';

// DRACOLoader's defaults for a `.drc` file, read by attribute type.
const defaultAttributes = {
  position: 'POSITION',
  normal: 'NORMAL',
  color: 'COLOR',
  uv: 'TEX_COORD',
};

export function createDracoLoader(three, options = {}) {
  const { BufferGeometry, BufferAttribute, Color, ColorManagement } = three;
  if (typeof BufferGeometry !== 'function' || typeof BufferAttribute !== 'function') {
    throw new TypeError("createDracoLoader needs three.js's BufferGeometry and BufferAttribute");
  }
  const pool = options.pool ?? createDecoderPool(options);
  // The buffer goes to the worker by transfer, which leaves nothing here to
  // decode a second time; GLTFLoader asks again for a primitive it shares, and
  // gets the first answer, as DRACOLoader gives it.
  const tasks = new WeakMap();

  // Vertex colours a `.drc` file stores in sRGB become linear, as DRACOLoader
  // makes them; without Color and ColorManagement they are left as stored.
  function toWorkingColorSpace(attribute) {
    const convert = ColorManagement?.colorSpaceToWorking ?? ColorManagement?.toWorkingColorSpace;
    if (!Color || !convert) return;
    const color = new Color();
    for (let i = 0; i < attribute.count; i += 1) {
      color.fromBufferAttribute(attribute, i);
      convert.call(ColorManagement, color, SRGB);
      attribute.setXYZ(i, color.r, color.g, color.b);
    }
  }

  async function decode(buffer, attributeIDs, attributeTypes, vertexColorSpace) {
    const byId = !!attributeIDs;
    const requests = Object.entries(attributeIDs ?? defaultAttributes).map(([name, id]) => ({
      name,
      ...(byId ? { id } : { semantic: id }),
      type: attributeTypes?.[name] ?? 'Float32Array',
    }));
    const result = await pool.decode(buffer, requests, { transfer: buffer instanceof ArrayBuffer });
    if (!result.success) throw new Error(result.error);

    const geometry = new BufferGeometry();
    if (result.index) geometry.setIndex(new BufferAttribute(result.index, 1));
    for (const { name, array, itemSize } of result.attributes) {
      const attribute = new BufferAttribute(array, itemSize);
      if (name === 'color') {
        if (vertexColorSpace === SRGB) toWorkingColorSpace(attribute);
        attribute.normalized = !(array instanceof Float32Array);
      }
      geometry.setAttribute(name, attribute);
    }
    return geometry;
  }

  function decodeGeometry(buffer, { attributeIDs, attributeTypes, vertexColorSpace = LINEAR_SRGB } = {}) {
    const key = JSON.stringify([attributeIDs ?? null, attributeTypes ?? null, vertexColorSpace]);
    const cached = buffer instanceof ArrayBuffer ? tasks.get(buffer) : undefined;
    if (cached?.key === key) return cached.promise;
    if (cached && buffer.byteLength === 0) {
      return Promise.reject(new Error('this buffer was already decoded with other settings and handed to a worker'));
    }
    const promise = decode(buffer, attributeIDs, attributeTypes, vertexColorSpace);
    if (buffer instanceof ArrayBuffer) tasks.set(buffer, { key, promise });
    return promise;
  }

  const loader = {
    pool,
    decodeGeometry,
    /** DRACOLoader's entry for GLTFLoader's `KHR_draco_mesh_compression`. */
    decodeDracoFile(buffer, callback, attributeIDs, attributeTypes, vertexColorSpace = LINEAR_SRGB, onError = () => {}) {
      return decodeGeometry(buffer, { attributeIDs, attributeTypes, vertexColorSpace }).then(callback).catch(onError);
    },
    /** A `.drc` file's contents, as DRACOLoader's `parse`. */
    parse(buffer, onLoad, onError = () => {}) {
      loader.decodeDracoFile(buffer, onLoad, null, null, SRGB, onError);
    },
    parseAsync(buffer) {
      return decodeGeometry(buffer, { vertexColorSpace: SRGB });
    },
    preload() {
      // A failure shows again on the first decode, which is where it is handled.
      pool.preload().catch(() => {});
      return loader;
    },
    setWorkerLimit(workers) {
      pool.setWorkerLimit(workers);
      return loader;
    },
    dispose() {
      pool.dispose();
      return loader;
    },
  };
  return loader;
}
