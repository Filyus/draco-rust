import type { PoolOptions, TaskOptions, Pool } from './pool-core.js';

export type { PoolOptions, TaskOptions };

/** One attribute of a primitive, its bytes in the accessor's component type. */
export interface GltfAttribute {
  semantic: string;
  count: number;
  components: number;
  /** The glTF component type: 5120 to 5126. */
  componentType: number;
  normalized: boolean;
  bytes: Uint8Array;
}

export interface GltfPrimitive {
  /** The glTF primitive mode, 4 for triangles. */
  mode: number;
  attributes: GltfAttribute[];
  index: { count: number; componentType: number; bytes: Uint8Array } | null;
}

export interface GltfRead {
  /** The document's JSON, as `GltfAsset.json()` writes it. */
  json: Uint8Array;
  /** Every primitive of every mesh, Draco-compressed or not, in document order. */
  meshes: GltfPrimitive[][];
}

export interface ReadOptions extends TaskOptions {
  /** A `.gltf`'s external files by URI, as `GltfAsset.withResources` takes them. */
  resources?: Record<string, Uint8Array>;
  /** `"2.0"`, the default, or `"2.1"`. */
  profile?: string;
  /** Draco output ceilings; this WebAssembly build decodes on one thread. */
  decode_options?: GltfDracoDecodeOptions;
}

export interface GltfDracoDecodeOptions {
  /** Maximum points in one Draco point-cloud primitive. */
  max_points?: number;
  /** Maximum decoded faces in one Draco primitive. */
  max_faces?: number;
  /** Maximum total decoded attribute bytes in one Draco primitive. */
  max_decoded_bytes?: number;
  /** Accepted for API parity; this WebAssembly build decodes on one thread. */
  threads?: number;
}

export interface GltfPool extends Pool {
  /** Reads a `.glb`, or a `.gltf` with its `resources`, on a worker. */
  read(data: ArrayBuffer | ArrayBufferView, options?: ReadOptions): Promise<GltfRead>;
  /** `inspect_gltf` on a worker. */
  inspect(data: ArrayBuffer | ArrayBufferView, options?: TaskOptions): Promise<any>;
}

export function createPool(options?: PoolOptions): GltfPool;
