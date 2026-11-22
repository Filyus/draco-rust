import type { ResourceMap } from '../scene-resources.ts';
import { debugLog, log } from './log.ts';
import { CACHE_BUST } from './modules.ts';
import { createParseClient } from './parse-client.ts';
import type { ParseCallResult } from './parse-client.ts';
import { MODULE_LABELS, parseObjMaterials } from './parse-tasks.ts';
import type { ParseFormat } from './parse-tasks.ts';
import { modules } from './state.ts';

/**
 * Reading a dropped file into whatever its format's module returns.
 *
 * Each parser runs in the background parse worker (`parse-worker.ts`) using
 * the already-compiled `WebAssembly.Module` from `modules.ts`, with input
 * buffers transferred across and handed straight back so large files neither
 * freeze the main thread nor double peak memory. If workers are unavailable,
 * `parse-client.ts` runs the exact same `executeParseTask` on this thread.
 */

export { mtlMapPath } from './model-intake.ts';
export { parseObjMaterials } from './parse-tasks.ts';
export type { ParseFormat, ParseOutcome } from './parse-tasks.ts';

const parseClient = createParseClient({
  cacheBust: CACHE_BUST,
  onWorkerRestart: (format) => {
    log(`${format} parse worker restarted with empty memory`, 'info');
  },
});

/** Cancel any in-flight off-thread parse and terminate a busy worker immediately. */
export function cancelActiveParse() {
  parseClient.cancel();
}

/**
 * Parse a loaded model off the main thread, transferring `data` and `resources`
 * into the worker and returning them alongside the parsed outcome.
 */
export async function parseFile(
  format: ParseFormat,
  data: Uint8Array,
  resources: ResourceMap,
  extension: string,
  options: { transfer?: boolean; signal?: AbortSignal } = {},
): Promise<ParseCallResult> {
  const slot = modules[format];
  if (!slot?.loaded) {
    return {
      outcome: { result: { success: false, error: `${MODULE_LABELS[format]} module not loaded` } },
      data,
      resources,
      offMainThread: false,
    };
  }

  const response = await parseClient.parse(
    {
      format,
      extension,
      data,
      resources,
      glueUrl: slot.glueUrl ?? null,
      wasmModule: slot.wasmModule ?? null,
      fallbackModule: slot.module,
    },
    options,
  );

  const result = response.outcome.result;
  debugLog(`${MODULE_LABELS[format]} parse result (worker=${response.offMainThread}):`, result);
  if (format === 'ply' && result.meshes) {
    for (const mesh of result.meshes) {
      debugLog('PLY mesh: positions=', mesh.positions?.length,
        ', indices=', mesh.indices?.length,
        ', normals=', mesh.normals?.length);
    }
  }
  return response;
}

// Per-format convenience wrappers kept for callers that expect the original
// single-result shape without buffer transfer.
export async function parseObjFile(data: Uint8Array, resources: ResourceMap = Object.create(null)) {
  return (await parseFile('obj', data, resources, 'obj')).outcome.result;
}

export async function parsePlyFile(data: Uint8Array) {
  return (await parseFile('ply', data, Object.create(null), 'ply')).outcome.result;
}

export async function parseStlFile(data: Uint8Array) {
  return (await parseFile('stl', data, Object.create(null), 'stl')).outcome.result;
}

export async function parseDrcFile(data: Uint8Array) {
  return (await parseFile('drc', data, Object.create(null), 'drc')).outcome.result;
}

export async function parseGltfFile(data: Uint8Array, extension: string) {
  return (await parseFile('gltf', data, Object.create(null), extension)).outcome.result;
}

export async function parseFbxFile(data: Uint8Array) {
  return (await parseFile('fbx', data, Object.create(null), 'fbx')).outcome.result;
}
