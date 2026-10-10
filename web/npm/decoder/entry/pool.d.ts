import type { DracoAttributeRequest, DracoDecodeResult } from './index.js';

export interface DecoderPoolOptions {
  /**
   * How many workers to run at most; they start as work arrives. Defaults to
   * one less than the cores, between one and four. `0` decodes on the calling
   * thread, as does an environment without workers.
   */
  workers?: number;
  /**
   * The entry's wasm, when it is not next to this module where the pool looks
   * for it: a URL, a `Response`, the bytes, or a compiled module.
   */
  wasm?: string | URL | Response | BufferSource | WebAssembly.Module;
}

export interface DecodeOptions {
  /**
   * Hand the stream's buffer to the worker instead of copying it, leaving it
   * detached here. For a buffer nothing else reads afterwards.
   */
  transfer?: boolean;
}

export interface DecoderPool {
  /**
   * `decode_draco` on a worker. A stream that does not decode resolves with
   * `success: false`, as `decode_draco` returns it; a worker that fails rejects
   * the tasks it held.
   */
  decode(
    data: ArrayBuffer | ArrayBufferView,
    attributes?: DracoAttributeRequest[],
    options?: DecodeOptions,
  ): Promise<DracoDecodeResult>;
  /** Compiles the wasm and starts one worker ahead of the first decode. */
  preload(): Promise<void>;
  /** Changes the worker limit; idle workers past it stop now, busy ones when done. */
  setWorkerLimit(workers: number): void;
  readonly workerLimit: number;
  /** Stops every worker and rejects what they held. */
  dispose(): void;
}

export function createDecoderPool(options?: DecoderPoolOptions): DecoderPool;
