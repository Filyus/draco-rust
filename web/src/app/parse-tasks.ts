import { buildSceneDocumentWithFbxProvenance } from '../fbx-scene-document.ts';
import type { FbxSceneProvenance } from '../fbx-scene-provenance.ts';
import { buildSceneDocumentWithGltfProvenance } from '../gltf-scene-document.ts';
import type { GltfSceneProvenance } from '../gltf-scene-provenance.ts';
import { splatCloudFromSceneDocument } from '../gltf-splat.ts';
import type { LoadedObjMaterial } from '../mesh-loader.ts';
import { summarizeSceneDocumentGeometry } from '../scene-document.ts';
import type { SceneDocument } from '../scene-document.ts';
import { basename } from '../scene-resources.ts';
import type { ResourceMap } from '../scene-resources.ts';
import { isSplatPly, readSplatCloud, selectedFromNamedAttributes } from '../splat.ts';
import { mtlMapPath, objLibraries } from './model-intake.ts';
import type { LoadedFile } from './state.ts';

/**
 * Format-specific parsing and decoding, free of DOM dependencies.
 *
 * Shared verbatim between the background parse worker (`parse-worker.ts`) and
 * the main-thread fallback in `parsers.ts`, so both execution paths produce the
 * exact same plain-data `ParseOutcome` and never leak wasm-bindgen handles
 * across thread boundaries.
 */

export type ParseFormat = 'obj' | 'ply' | 'stl' | 'drc' | 'gltf' | 'fbx';

/** Where a wasm32 linear memory stops growing: 65536 pages of 64 KiB. */
export const WASM32_MEMORY_CAP = 2 ** 32;

/** Human-readable labels for the "module not loaded" messages. */
export const MODULE_LABELS: Record<ParseFormat, string> = {
  obj: 'OBJ',
  ply: 'PLY',
  stl: 'STL',
  drc: 'DRC',
  gltf: 'glTF',
  fbx: 'FBX',
};

/** Everything the shell needs from one parse pass, in plain transferable data. */
export interface ParseOutcome {
  result: LoadedFile;
  sceneDocument?: SceneDocument | null;
  gltfProvenance?: GltfSceneProvenance | null;
  fbxProvenance?: FbxSceneProvenance | null;
  /** Non-fatal warnings produced while assembling the scene document or splats. */
  warnings?: string[];
  /**
   * True when the underlying wasm instance trapped (e.g. OOM at 4 GiB) during
   * this parse and must be discarded so the next file starts with empty memory.
   */
  trapped?: boolean;
}

/** Format a wasm trap the same way `modules.ts` does on the main thread. */
export function formatTrapMessage(key: string, memory: WebAssembly.Memory | null, error: Error): string {
  const full = memory !== null && memory.buffer.byteLength > WASM32_MEMORY_CAP - 64 * 2 ** 20;
  return full
    ? `the ${key} module ran out of memory: a WebAssembly module holds at most 4 GiB, and this file needs more`
    : `the ${key} module stopped (${error.message}) and is being restarted`;
}

/**
 * Parse companion `.mtl` libraries referenced by an OBJ file.
 *
 * Accepts raw bytes (scanning for `mtllib` without decoding the whole OBJ into
 * a string) or already-decoded text for callers that already hold it.
 */
export function parseObjMaterials(
  objSource: string | Uint8Array,
  resources: ResourceMap,
  warnings: string[],
): Record<string, LoadedObjMaterial> {
  const materials: Record<string, LoadedObjMaterial> = Object.create(null);
  const libraries = typeof objSource === 'string'
    ? objLibrariesFromText(objSource)
    : objLibraries(objSource);

  for (const library of libraries) {
    const bytes = resources[library] || resources[basename(library)];
    if (!bytes) {
      warnings.push(`OBJ material library not selected: ${library}`);
      continue;
    }
    const raw = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    let current: string | null = null;
    const text = new TextDecoder().decode(raw);
    for (const line of text.split(/\r\n|[\r\n]/)) {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith('#')) continue;
      const [directive, ...values] = trimmed.split(/\s+/);
      if (directive === 'newmtl') {
        current = values.join(' ');
        if (current) materials[current] ||= { diffuse: [1, 1, 1], alpha: 1 };
      } else if (directive === 'Kd' && current && values.length >= 3) {
        const diffuse = values.slice(0, 3).map(Number);
        if (diffuse.every(Number.isFinite)) materials[current].diffuse = diffuse;
      } else if ((directive === 'd' || directive === 'Tr') && current && values.length >= 1) {
        const value = Number(values[0]);
        if (Number.isFinite(value)) materials[current].alpha = directive === 'Tr' ? 1 - value : value;
      } else if (directive.toLowerCase() === 'map_kd' && current) {
        const texture = mtlMapPath(values);
        if (texture) materials[current].baseColorTextureUri = texture;
      }
    }
  }
  return materials;
}

function objLibrariesFromText(objText: string): string[] {
  const libraries: string[] = [];
  for (const line of objText.split(/\r\n|[\r\n]/)) {
    const match = line.trim().match(/^mtllib\s+(.+)$/i);
    if (match) libraries.push(match[1].trim());
  }
  return libraries;
}

/**
 * Run one format's full read/decode pass against an initialized wasm module.
 *
 * For glTF and FBX, this also builds the portable `SceneDocument` and extracts
 * any `KHR_gaussian_splatting` cloud while still on the calling thread (the
 * worker in normal operation), so no `GltfAsset` wasm handle ever needs to
 * cross threads and the main thread receives ready-to-render typed arrays.
 */
export function executeParseTask(
  format: ParseFormat,
  data: Uint8Array,
  resources: ResourceMap,
  extension: string,
  module: any,
  memory: WebAssembly.Memory | null = null,
): ParseOutcome {
  switch (format) {
    case 'obj': {
      const result = module.read_obj(data);
      if (result?.success) {
        result.materials = parseObjMaterials(
          data,
          resources,
          result.warnings || (result.warnings = []),
        );
      }
      return { result };
    }

    case 'ply': {
      const result = module.read_ply(data);
      // A Gaussian splat is an ordinary point cloud to the reader above and a
      // scene to a renderer, and the difference is entirely in property names it
      // has no slot for. The reader carries those as named extras -- which is
      // also what lets an export keep them -- so the splat is read from the
      // same parse rather than from a second pass over the file.
      const properties: string[] = result?.header?.properties ?? [];
      const plyMesh = result?.meshes?.[0];
      if (isSplatPly(properties) && plyMesh) {
        const count = (plyMesh.positions?.length ?? 0) / 3;
        const selected = selectedFromNamedAttributes(count, plyMesh.positions ?? [], plyMesh.extras ?? []);
        const cloud = readSplatCloud(selected);
        if (cloud) result.splats = cloud;
      }
      return { result };
    }

    case 'stl': {
      return { result: module.read_stl(data) };
    }

    case 'drc': {
      try {
        const result = module.parse_drc_bytes(data);
        // The same recognition the PLY path does, one layer lower: Draco carries
        // attribute names in metadata, so they arrive with the extras.
        const mesh = result?.meshes?.[0];
        const extras = mesh?.extras ?? [];
        if (mesh && extras.length > 0) {
          const count = (mesh.positions?.length ?? 0) / 3;
          const selected = selectedFromNamedAttributes(count, mesh.positions ?? [], extras);
          if (isSplatPly(Object.keys(selected.properties))) {
            const cloud = readSplatCloud(selected);
            if (cloud) result.splats = cloud;
          }
        }
        return { result };
      } catch (error) {
        const trapped = error instanceof WebAssembly.RuntimeError;
        const detail = trapped
          ? formatTrapMessage('drc', memory, error)
          : error instanceof Error ? error.message : String(error);
        return {
          result: {
            success: false,
            error: `Malformed Draco payload: ${detail}`,
          },
          trapped,
        };
      }
    }

    case 'gltf': {
      const summary = module.inspect_gltf(data);
      const result: LoadedFile = { ...summary, document: true, format: extension };
      const warnings: string[] = [];
      let sceneDocument: SceneDocument | null = null;
      let gltfProvenance: GltfSceneProvenance | null = null;

      if (result?.success && result.document) {
        try {
          const adapted = buildSceneDocumentWithGltfProvenance(
            data,
            resources as Record<string, Uint8Array>,
            module,
          );
          sceneDocument = adapted.document;
          gltfProvenance = adapted.provenance;
          Object.assign(result, summarizeSceneDocumentGeometry(sceneDocument));
          const splats = splatCloudFromSceneDocument(sceneDocument);
          warnings.push(...splats.warnings);
          if (splats.cloud) result.splats = splats.cloud;
        } catch (error) {
          // A trap must not be swallowed as "scene details unavailable".
          if (error instanceof WebAssembly.RuntimeError) throw error;
          const message = error instanceof Error ? error.message : String(error);
          if (message.includes('External resource denied:') || message.includes('Draco decode error:')) {
            throw error;
          }
          warnings.push(`Scene details unavailable: ${message}`);
        }
      }

      return { result, sceneDocument, gltfProvenance, warnings };
    }

    case 'fbx': {
      const result = module.read_fbx(data);
      let sceneDocument: SceneDocument | null = null;
      let fbxProvenance: FbxSceneProvenance | null = null;
      if (result?.success && result.scene) {
        const adapted = buildSceneDocumentWithFbxProvenance(result, resources);
        sceneDocument = adapted.document;
        fbxProvenance = adapted.provenance;
      }
      return { result, sceneDocument, fbxProvenance };
    }
  }
}
