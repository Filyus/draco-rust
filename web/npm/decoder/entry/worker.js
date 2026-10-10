// A decoder worker. The pool sends it the compiled module once, then streams
// to decode; it answers each with its result, handing the arrays back by
// transfer. Runs as a Web Worker and as a Node worker thread.
import init, { decode_draco } from './index.js';

const web = typeof self !== 'undefined' && typeof self.postMessage === 'function';
const port = web ? self : process.getBuiltinModule('node:worker_threads').parentPort;
const listen = (handler) =>
  web ? self.addEventListener('message', (event) => handler(event.data)) : port.on('message', handler);

let ready = null;

listen(async (message) => {
  if (message.type === 'init') {
    ready = init({ module_or_path: message.module });
    return;
  }
  const { id, data, attributes } = message;
  try {
    await ready;
    const result = decode_draco(data, attributes);
    const transfer = result.attributes.map((attribute) => attribute.array.buffer);
    if (result.index) transfer.push(result.index.buffer);
    port.postMessage({ id, result }, transfer);
  } catch (error) {
    port.postMessage({ id, error: String(error?.message ?? error) });
  }
});
