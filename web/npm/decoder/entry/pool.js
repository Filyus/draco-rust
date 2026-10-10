// Decoding off the calling thread: a pool of workers, each running this entry's
// own module. The wasm is compiled once here and handed to every worker, the
// stream goes over by transfer and the arrays come back the same way, so a
// decode copies nothing it does not have to.
import init, { decode_draco } from './index.js';

/** Leaves a core for the page, and stops at four, past which a page rarely has
 * that many streams in flight at once. */
const defaultWorkers = () =>
  Math.max(1, Math.min(4, (globalThis.navigator?.hardwareConcurrency ?? 4) - 1));

const builtin = (name) =>
  typeof process !== 'undefined' && typeof process.getBuiltinModule === 'function'
    ? process.getBuiltinModule(name)
    : undefined;

async function compile(wasm) {
  if (wasm instanceof WebAssembly.Module) return wasm;
  let source = wasm ?? new URL('./index_bg.wasm', import.meta.url);
  if (typeof source === 'string' || source instanceof URL) {
    const url = new URL(source, import.meta.url);
    // Node's fetch does not read files.
    const fs = url.protocol === 'file:' ? builtin('node:fs/promises') : undefined;
    if (fs) return WebAssembly.compile(await fs.readFile(url));
    source = await fetch(url);
  }
  if (typeof Response !== 'undefined' && source instanceof Response) {
    if (!source.ok) throw new Error(`failed to fetch ${source.url}: ${source.status}`);
    if (source.headers.get('content-type') === 'application/wasm') {
      return WebAssembly.compileStreaming(source);
    }
    source = await source.arrayBuffer();
  }
  return WebAssembly.compile(source);
}

/** What the pool needs of a worker, the same over a Web Worker and a Node
 * worker thread. */
function spawn(blobUrl) {
  if (typeof Worker === 'function') {
    const url = new URL('./worker.js', import.meta.url);
    // A bundler recognises this exact form and emits the worker as a chunk.
    const worker =
      typeof location === 'undefined' || url.origin === location.origin
        ? new Worker(new URL('./worker.js', import.meta.url), { type: 'module' })
        : new Worker(blobUrl(url), { type: 'module' });
    return {
      post: (message, transfer) => worker.postMessage(message, transfer),
      listen: (onMessage, onError) => {
        worker.addEventListener('message', (event) => onMessage(event.data));
        worker.addEventListener('error', (event) => {
          event.preventDefault();
          onError(new Error(event.message || 'the decoder worker failed'));
        });
        worker.addEventListener('messageerror', () => onError(new Error('a decode result could not be read')));
      },
      busy: () => {},
      terminate: () => worker.terminate(),
    };
  }
  const threads = builtin('node:worker_threads');
  if (!threads) return null;
  const worker = new threads.Worker(new URL('./worker.js', import.meta.url));
  // An idle worker does not keep Node running; one with work does.
  worker.unref();
  return {
    post: (message, transfer) => worker.postMessage(message, transfer),
    listen: (onMessage, onError) => {
      worker.on('message', onMessage);
      worker.on('error', onError);
      worker.on('exit', (code) => onError(new Error(`the decoder worker exited with code ${code}`)));
    },
    busy: (busy) => (busy ? worker.ref() : worker.unref()),
    terminate: () => worker.terminate(),
  };
}

const workersAvailable = () =>
  typeof Worker === 'function' || builtin('node:worker_threads') !== undefined;

export function createDecoderPool(options = {}) {
  let limit = options.workers ?? defaultWorkers();
  let compiled = null;
  let local = null;
  let blob = null;
  let disposed = false;
  let nextId = 0;
  const slots = [];

  const load = () => (compiled ??= compile(options.wasm));
  // A worker script must share the page's origin. From a CDN, a worker made
  // here imports it instead, which a module worker may do across origins.
  const blobUrl = (url) =>
    (blob ??= URL.createObjectURL(
      new Blob([`import ${JSON.stringify(url.href)};`], { type: 'text/javascript' }),
    ));

  function retire(slot, error) {
    const at = slots.indexOf(slot);
    if (at >= 0) slots.splice(at, 1);
    slot.worker.terminate();
    for (const { reject } of slot.pending.values()) reject(error);
    slot.pending.clear();
  }

  function open(module) {
    const worker = spawn(blobUrl);
    if (!worker) return null;
    const slot = { worker, pending: new Map() };
    worker.listen(
      (message) => {
        const task = slot.pending.get(message.id);
        if (!task) return;
        slot.pending.delete(message.id);
        if (slot.pending.size === 0) {
          worker.busy(false);
          if (slots.indexOf(slot) >= limit) retire(slot);
        }
        if ('error' in message) task.reject(new Error(message.error));
        else task.resolve(message.result);
      },
      // A worker that fails takes only its own tasks down; the next decode
      // starts another.
      (error) => {
        if (slots.includes(slot)) retire(slot, error);
      },
    );
    worker.post({ type: 'init', module });
    slots.push(slot);
    return slot;
  }

  /** The least loaded worker, or a new one while every worker is busy and
   * the limit allows another. */
  function pick(module) {
    let best = null;
    for (const slot of slots.slice(0, limit)) {
      if (!best || slot.pending.size < best.pending.size) best = slot;
    }
    if ((!best || best.pending.size > 0) && slots.length < limit) return open(module) ?? best;
    return best;
  }

  const onThisThread = () =>
    (local ??= load().then((module) => init({ module_or_path: module })));

  async function decode(data, attributes, { transfer = false } = {}) {
    if (disposed) throw new Error('the decoder pool was disposed');
    const bytes = ArrayBuffer.isView(data)
      ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength)
      : new Uint8Array(data);
    if (limit > 0 && workersAvailable()) {
      const module = await load();
      if (disposed) throw new Error('the decoder pool was disposed');
      const slot = pick(module);
      if (slot) {
        const id = nextId++;
        const sent = transfer ? bytes : bytes.slice();
        return new Promise((resolve, reject) => {
          slot.pending.set(id, { resolve, reject });
          slot.worker.busy(true);
          slot.worker.post({ id, data: sent, attributes }, [sent.buffer]);
        });
      }
    }
    await onThisThread();
    return decode_draco(bytes, attributes);
  }

  return {
    decode,
    async preload() {
      if (limit > 0 && workersAvailable()) {
        const module = await load();
        if (!disposed && slots.length === 0) open(module);
      } else {
        await onThisThread();
      }
    },
    setWorkerLimit(workers) {
      limit = workers;
      for (const slot of slots.slice(limit)) if (slot.pending.size === 0) retire(slot);
    },
    get workerLimit() {
      return limit;
    },
    dispose() {
      disposed = true;
      for (const slot of [...slots]) retire(slot, new Error('the decoder pool was disposed'));
      if (blob) URL.revokeObjectURL(blob);
    },
  };
}
