import type { DracoArrayType } from './index.js';
import type { DecoderPool, DecoderPoolOptions } from './pool.js';

/**
 * The three.js classes the loader builds with, passed in so the package uses
 * the application's own copy of three: `import * as THREE from "three"` will
 * do. With `Color` and `ColorManagement`, a `.drc` file's sRGB vertex colours
 * are made linear as DRACOLoader makes them.
 */
export interface ThreeClasses<Geometry> {
  BufferGeometry: new () => Geometry;
  BufferAttribute: new (array: any, itemSize: number, normalized?: boolean) => any;
  Color?: new () => any;
  /** `colorSpaceToWorking` since three r177, `toWorkingColorSpace` before. */
  ColorManagement?:
    | { colorSpaceToWorking(color: any, colorSpace: string): any }
    | { toWorkingColorSpace(color: any, colorSpace: string): any };
}

export interface DracoLoaderOptions extends DecoderPoolOptions {
  /** A pool to share with other loaders, instead of one of the loader's own. */
  pool?: DecoderPool;
}

export interface DecodeGeometryOptions {
  /** three.js attribute name to Draco unique id, as glTF names them. Without
   * it, `position`, `normal`, `color` and `uv` are read by type. */
  attributeIDs?: Record<string, number>;
  /** three.js attribute name to the typed array it comes back in. */
  attributeTypes?: Record<string, DracoArrayType>;
  /** `"srgb"` makes vertex colours linear; the default leaves them as stored. */
  vertexColorSpace?: string;
}

/** The part of three.js's DRACOLoader that GLTFLoader and `.drc` users call. */
export interface DracoLoader<Geometry> {
  readonly pool: DecoderPool;
  decodeGeometry(buffer: ArrayBuffer | ArrayBufferView, options?: DecodeGeometryOptions): Promise<Geometry>;
  decodeDracoFile(
    buffer: ArrayBuffer,
    callback: (geometry: Geometry) => void,
    attributeIDs?: Record<string, number> | null,
    attributeTypes?: Record<string, DracoArrayType> | null,
    vertexColorSpace?: string,
    onError?: (error: unknown) => void,
  ): Promise<void>;
  parse(buffer: ArrayBuffer, onLoad: (geometry: Geometry) => void, onError?: (error: unknown) => void): void;
  parseAsync(buffer: ArrayBuffer | ArrayBufferView): Promise<Geometry>;
  preload(): this;
  setWorkerLimit(workers: number): this;
  dispose(): this;
}

/**
 * A stand-in for three.js's DRACOLoader on the decoder pool:
 * `gltfLoader.setDRACOLoader(createDracoLoader(THREE))`.
 */
export function createDracoLoader<Geometry>(
  three: ThreeClasses<Geometry>,
  options?: DracoLoaderOptions,
): DracoLoader<Geometry>;
