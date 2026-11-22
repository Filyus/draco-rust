import type { ResourceMap } from '../scene-resources.ts';
import { executeParseTask, formatTrapMessage } from './parse-tasks.ts';
import type { ParseFormat, ParseOutcome } from './parse-tasks.ts';

/**
 * Off-main-thread file parser for the Draco 3D Studio converter.
 *
 * Each format's wasm module is instantiated lazily inside this worker from the
 * already-compiled `WebAssembly.Module` handed over by the main thread (zero
 * re-fetch and zero re-compile). Input bytes arrive by transfer and are
 * transferred straight back alongside the parsed typed arrays, so a hundred-
 * megabyte PLY or Draco point cloud neither blocks the UI nor doubles memory.
 *
 * Runs as a browser module Web Worker and as a Node `worker_threads` worker
 * (for direct unit tests without a browser).
 */

interface WorkerPort {
  postMessage(message: unknown, transfer?: Transferable[]): void;
  addEventListener?(type: string, listener: (event: { data: unknown }) => void): void;
  on?(type: string, listener: (data: unknown) => void): void;
}

export interface WorkerParseRequest {
  id: number;
  format: ParseFormat;
  extension: string;
  glueUrl: string;
  wasmModule?: WebAssembly.Module | null;
  wasmUrl?: string | null;
  data: Uint8Array;
  resources: ResourceMap;
}

export interface WorkerParseResponse {
  id: number;
  outcome?: ParseOutcome;
  error?: string;
  trapped?: boolean;
  data: Uint8Array;
  resources: ResourceMap;
}

interface LoadedWorkerModule {
  module: any;
  memory: WebAssembly.Memory;
}

const isWebWorker = typeof self !== 'undefined' && typeof (self as unknown as WorkerPort).postMessage === 'function';

function getPort(): WorkerPort {
  if (isWebWorker) return self as unknown as WorkerPort;
  const proc = (globalThis as { process?: { getBuiltinModule?: (name: string) => any } }).process;
  const threads = proc?.getBuiltinModule?.('node:worker_threads');
  if (!threads?.parentPort) {
    throw new Error('parse-worker must run inside a Web Worker or Node worker_threads');
  }
  return threads.parentPort as WorkerPort;
}

const port = getPort();
const listen = (handler: (message: WorkerParseRequest) => void) => {
  if (isWebWorker) {
    port.addEventListener!('message', (event) => handler(event.data as WorkerParseRequest));
  } else {
    port.on!('message', (data) => handler(data as WorkerParseRequest));
  }
};

const loadedModules = new Map<ParseFormat, Promise<LoadedWorkerModule>>();
const activeMemories = new Set<WebAssembly.Memory>();

async function ensureModule(
  format: ParseFormat,
  glueUrl: string,
  wasmModule?: WebAssembly.Module | null,
  wasmUrl?: string | null,
): Promise<LoadedWorkerModule> {
  let pending = loadedModules.get(format);
  if (!pending) {
    pending = (async () => {
      const module = await import(glueUrl);
      const source = wasmModule ?? wasmUrl ?? undefined;
      const exports = source !== undefined
        ? await module.default({ module_or_path: source })
        : await module.default();
      const memory = exports.memory as WebAssembly.Memory;
      activeMemories.add(memory);
      return { module, memory };
    })();
    // If initialization fails, drop the rejected promise so a retry can succeed.
    pending.catch(() => {
      if (loadedModules.get(format) === pending) loadedModules.delete(format);
    });
    loadedModules.set(format, pending);
  }
  return pending;
}

/**
 * Every `ArrayBuffer` reachable from `value`, excluding live wasm linear
 * memories (which must stay attached to this worker) and deduplicated so
 * aliased views (e.g. FBX resource aliases) never trigger a DataCloneError.
 */
function collectTransferables(
  value: unknown,
  excluded: Set<ArrayBuffer>,
  found: Set<ArrayBuffer> = new Set(),
  seen: WeakSet<object> = new WeakSet(),
): Set<ArrayBuffer> {
  if (value === null || typeof value !== 'object') return found;
  if (ArrayBuffer.isView(value)) {
    if (value.buffer instanceof ArrayBuffer && !excluded.has(value.buffer)) {
      found.add(value.buffer);
    }
    return found;
  }
  if (value instanceof ArrayBuffer) {
    if (!excluded.has(value)) found.add(value);
    return found;
  }
  if (seen.has(value)) return found;
  seen.add(value);
  if (Array.isArray(value)) {
    for (const item of value) collectTransferables(item, excluded, found, seen);
  } else {
    for (const item of Object.values(value)) collectTransferables(item, excluded, found, seen);
  }
  return found;
}

/** Restore a null-prototype dictionary after structured cloning across postMessage. */
function nullProtoMap<T extends Record<string, unknown>>(source: T): T {
  return Object.assign(Object.create(null), source);
}

listen(async (request) => {
  const { id, format, extension, glueUrl, wasmModule, wasmUrl, data } = request;
  const resources = nullProtoMap(request.resources ?? {});
  let memory: WebAssembly.Memory | null = null;

  try {
    const loaded = await ensureModule(format, glueUrl, wasmModule, wasmUrl);
    memory = loaded.memory;
    const outcome = executeParseTask(format, data, resources, extension, loaded.module, memory);
    const excluded = new Set<ArrayBuffer>(Array.from(activeMemories, (mem) => mem.buffer));
    const response: WorkerParseResponse = {
      id,
      outcome,
      trapped: outcome.trapped,
      data,
      resources,
    };
    const transfer = [...collectTransferables(response, excluded)];
    port.postMessage(response, transfer);
  } catch (error) {
    const trapped = error instanceof WebAssembly.RuntimeError;
    const message = trapped
      ? formatTrapMessage(format, memory, error)
      : error instanceof Error ? error.message : String(error);
    const excluded = new Set<ArrayBuffer>(Array.from(activeMemories, (mem) => mem.buffer));
    const response: WorkerParseResponse = {
      id,
      error: message,
      trapped,
      data,
      resources,
    };
    const transfer = [...collectTransferables(response, excluded)];
    port.postMessage(response, transfer);
  }
});
