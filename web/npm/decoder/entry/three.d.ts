import type { LoadingManager } from 'three';
import { DRACOLoader as ThreeDRACOLoader } from 'three/addons/loaders/DRACOLoader.js';
import type { DecoderPool, DecoderPoolOptions } from './pool.js';
import type { DracoDecodeOptions } from './index.js';

export interface DRACOLoaderOptions extends DecoderPoolOptions {
  /** A pool to share with other loaders, instead of one of the loader's own. */
  pool?: DecoderPool;
  /** Per-stream output ceilings; WebAssembly currently decodes on one thread. */
  decode_options?: DracoDecodeOptions;
}

/**
 * three.js's `DRACOLoader`, decoding on this entry's worker pool instead of
 * the Draco decoder three fetches from `setDecoderPath`. It is three's class,
 * so it goes wherever one is expected:
 * `gltfLoader.setDRACOLoader(new DRACOLoader())`.
 */
export class DRACOLoader extends ThreeDRACOLoader {
  constructor(manager?: LoadingManager, options?: DRACOLoaderOptions);
  readonly pool: DecoderPool;
}
