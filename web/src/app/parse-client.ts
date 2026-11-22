import type { ResourceMap } from '../scene-resources.ts';
import { executeParseTask } from './parse-tasks.ts';
import type { ParseFormat, ParseOutcome } from './parse-tasks.ts';
import type { WorkerParseRequest, WorkerParseResponse } from './parse-worker.ts';

/**
 * Host controller for the converter's off-main-thread parse worker.
 *
 * Mirrors the pool design used by the `@draco-rust/*` npm packages: compiled
 * `WebAssembly.Module` objects are shared with a module Web Worker, buffers
 * cross by transfer (and input buffers come right back with the result so zero
 * copies happen on large files), and an environment without workers falls back
 * transparently to the same `executeParseTask` on the calling thread.
 *
 * DOM-free so Node tests can exercise the real worker lifecycle directly.
 */

export interface ParseInput {
  format: ParseFormat;
  extension: string;
  data: Uint8Array;
  resources: ResourceMap;
  /** Compiled WebAssembly module and stamped glue URL for the worker. */
  glueUrl: string | null;
  wasmModule: WebAssembly.Module | null;
  wasmUrl?: string | null;
  /** Main-thread module instance used only if workers are unavailable. */
  fallbackModule?: any;
}

export interface ParseCallOptions {
  /**
   * Move `data` and `resources` buffers into the worker without copying; the
   * worker transfers them back on completion so the caller receives owned
   * buffers again at `ParseResult.data` / `ParseResult.resources`.
   */
  transfer?: boolean;
  signal?: AbortSignal;
}

export interface ParseCallResult {
  outcome: ParseOutcome;
  data: Uint8Array;
  resources: ResourceMap;
  /** True when the parse executed on a background worker thread. */
  offMainThread: boolean;
}

export interface ParseClientOptions {
  /** Cache-busting query string (e.g. `?v=<build-id>`) appended to the worker URL in browsers. */
  cacheBust?: string;
  /** Set `false` to force main-thread fallback (used in tests). */
  workers?: boolean;
  /** Invoked when a trapped worker is terminated so it starts fresh next time. */
  onWorkerRestart?: (format: ParseFormat) => void;
}

export interface ParseClient {
  parse(input: ParseInput, options?: ParseCallOptions): Promise<ParseCallResult>;
  /** Cancel any in-flight parse and terminate a busy worker immediately. */
  cancel(): void;
  /** Terminate the worker and prevent further use. */
  dispose(): void;
}

interface WorkerAdapter {
  post(message: WorkerParseRequest, transfer: Transferable[]): void;
  listen(onMessage: (response: WorkerParseResponse) => void, onError: (error: Error) => void): void;
  busy(active: boolean): void;
  terminate(): void;
}

function nodeBuiltin(name: string): any {
  const proc = (globalThis as { process?: { getBuiltinModule?: (id: string) => any } }).process;
  return typeof proc?.getBuiltinModule === 'function' ? proc.getBuiltinModule(name) : undefined;
}

function workersAvailable(): boolean {
  return typeof Worker === 'function' || nodeBuiltin('node:worker_threads') !== undefined;
}

function resolveWorkerUrl(cacheBust: string): URL {
  // Node tests import this module directly as `.ts` via native type stripping,
  // while the built browser bundle loads `.js` from `www/app/`.
  const script = import.meta.url.endsWith('.ts') ? './parse-worker.ts' : `./parse-worker.js${cacheBust}`;
  return new URL(script, import.meta.url);
}

function spawnWorker(cacheBust: string): WorkerAdapter | null {
  const url = resolveWorkerUrl(cacheBust);
  if (typeof Worker === 'function') {
    const worker = new Worker(url, { type: 'module' });
    return {
      post: (message, transfer) => worker.postMessage(message, transfer),
      listen: (onMessage, onError) => {
        worker.addEventListener('message', (event) => onMessage(event.data as WorkerParseResponse));
        worker.addEventListener('error', (event) => {
          event.preventDefault();
          onError(new Error(event.message || 'Parse worker failed'));
        });
        worker.addEventListener('messageerror', () => onError(new Error('Parse worker response could not be read')));
      },
      busy: () => {},
      terminate: () => worker.terminate(),
    };
  }
  const threads = nodeBuiltin('node:worker_threads');
  if (!threads) return null;
  const worker = new threads.Worker(url);
  // An idle worker must not keep Node alive once a test finishes.
  worker.unref();
  return {
    post: (message, transfer) => worker.postMessage(message, transfer),
    listen: (onMessage, onError) => {
      worker.on('message', onMessage);
      worker.on('error', onError);
      worker.on('exit', (code: number) => {
        if (code !== 0) onError(new Error(`Parse worker exited with code ${code}`));
      });
    },
    busy: (active) => (active ? worker.ref() : worker.unref()),
    terminate: () => worker.terminate(),
  };
}

function abortError(signal?: AbortSignal): Error {
  if (signal?.reason instanceof Error) return signal.reason;
  if (typeof DOMException === 'function') return new DOMException('The parse was aborted', 'AbortError');
  const error = new Error('The parse was aborted');
  error.name = 'AbortError';
  return error;
}

/**
 * Prepare `data` and `resources` for `postMessage`.
 *
 * When `transfer` is true, full-buffer views move directly with zero copies
 * (views sharing a backing buffer — like FBX's aliased texture entries — are
 * deduplicated in `buffers`). A subview over a larger buffer is sliced first
 * so unrelated bytes are not detached. When `transfer` is false, every typed
 * array is copied into its own buffer and that copy is transferred.
 */
function prepareOutgoing(
  data: Uint8Array,
  resources: ResourceMap,
  transfer: boolean,
): { data: Uint8Array; resources: ResourceMap; buffers: Transferable[] } {
  const buffers = new Set<ArrayBuffer>();
  const clonedBySource = new Map<Uint8Array, Uint8Array>();

  const packView = (view: Uint8Array): Uint8Array => {
    const existing = clonedBySource.get(view);
    if (existing) return existing;
    const isFullBuffer = view.buffer instanceof ArrayBuffer
      && view.byteOffset === 0
      && view.byteLength === view.buffer.byteLength;
    const packed = transfer && isFullBuffer ? view : view.slice();
    if (packed.buffer instanceof ArrayBuffer) buffers.add(packed.buffer);
    clonedBySource.set(view, packed);
    return packed;
  };

  const outData = packView(data);
  const outResources: ResourceMap = Object.create(null);
  for (const [key, value] of Object.entries(resources)) {
    if (value instanceof Uint8Array) {
      outResources[key] = packView(value);
    } else if (value instanceof ArrayBuffer) {
      const buf = transfer ? value : value.slice(0);
      buffers.add(buf);
      outResources[key] = buf;
    } else if (value != null) {
      outResources[key] = value;
    }
  }

  return { data: outData, resources: outResources, buffers: [...buffers] };
}

function restoreNullProto<T extends Record<string, unknown>>(obj: T | undefined): T {
  return Object.assign(Object.create(null), obj ?? {});
}

interface PendingTask {
  id: number;
  format: ParseFormat;
  resolve: (value: ParseCallResult) => void;
  reject: (error: Error) => void;
  cleanup: () => void;
}

export function createParseClient(options: ParseClientOptions = {}): ParseClient {
  const cacheBust = options.cacheBust ?? '';
  const useWorkers = options.workers !== false;
  let worker: WorkerAdapter | null = null;
  let pending: PendingTask | null = null;
  let nextId = 1;
  let disposed = false;

  function terminateWorker(reason?: Error) {
    const currentWorker = worker;
    const currentTask = pending;
    worker = null;
    pending = null;
    if (currentWorker) {
      currentWorker.busy(false);
      currentWorker.terminate();
    }
    if (currentTask) {
      currentTask.cleanup();
      if (reason) currentTask.reject(reason);
    }
  }

  function ensureWorker(): WorkerAdapter | null {
    if (worker) return worker;
    const spawned = spawnWorker(cacheBust);
    if (!spawned) return null;
    worker = spawned;
    spawned.listen(
      (response) => {
        if (!pending || pending.id !== response.id) return;
        const task = pending;
        pending = null;
        task.cleanup();
        spawned.busy(false);

        const resources = restoreNullProto(response.resources);
        if (response.outcome?.result?.materials) {
          response.outcome.result.materials = restoreNullProto(response.outcome.result.materials);
        }

        if (response.trapped) {
          // Drop the trapped worker immediately so a 4 GiB linear memory is
          // freed right away instead of sitting until GC.
          terminateWorker();
          options.onWorkerRestart?.(task.format);
        }

        if (response.error !== undefined) {
          task.reject(new Error(response.error));
        } else {
          task.resolve({
            outcome: response.outcome!,
            data: response.data,
            resources,
            offMainThread: true,
          });
        }
      },
      (error) => {
        if (worker === spawned) terminateWorker(error);
      },
    );
    return spawned;
  }

  async function parse(input: ParseInput, callOptions: ParseCallOptions = {}): Promise<ParseCallResult> {
    if (disposed) throw new Error('Parse client was disposed');
    const { transfer = false, signal } = callOptions;
    if (signal?.aborted) throw abortError(signal);

    // If a newer parse starts while a previous one is still running, abort and
    // terminate the busy worker so the new file does not queue behind it.
    if (pending) terminateWorker(abortError());

    const canUseWorker = useWorkers && workersAvailable() && Boolean(input.glueUrl);
    if (canUseWorker) {
      const active = ensureWorker();
      if (active) {
        const id = nextId++;
        const outgoing = prepareOutgoing(input.data, input.resources, transfer);
        return new Promise<ParseCallResult>((resolve, reject) => {
          const onAbort = () => {
            if (pending?.id === id) {
              terminateWorker(abortError(signal));
            }
          };
          signal?.addEventListener('abort', onAbort, { once: true });
          const cleanup = () => {
            signal?.removeEventListener('abort', onAbort);
          };
          pending = { id, format: input.format, resolve, reject, cleanup };
          active.busy(true);
          active.post(
            {
              id,
              format: input.format,
              extension: input.extension,
              glueUrl: input.glueUrl!,
              wasmModule: input.wasmModule,
              wasmUrl: input.wasmUrl,
              data: outgoing.data,
              resources: outgoing.resources,
            },
            outgoing.buffers,
          );
        });
      }
    }

    // Fallback when workers are unavailable or disabled: run the exact same
    // task logic on the calling thread against the already-loaded module.
    if (!input.fallbackModule) {
      throw new Error(`No ${input.format} module available`);
    }
    const outcome = executeParseTask(
      input.format,
      input.data,
      input.resources,
      input.extension,
      input.fallbackModule,
    );
    return {
      outcome,
      data: input.data,
      resources: input.resources,
      offMainThread: false,
    };
  }

  return {
    parse,
    cancel() {
      if (pending) terminateWorker(abortError());
    },
    dispose() {
      disposed = true;
      terminateWorker(new Error('Parse client was disposed'));
    },
  };
}
