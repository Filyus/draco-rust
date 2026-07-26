import type { FbxSceneProvenance } from '../fbx-scene-provenance.ts';
import type { SceneDocument } from '../scene-document.ts';
import type { ResourceMap } from '../scene-resources.ts';
import type { Viewer } from '../viewer.ts';

/**
 * What the shell carries between one loaded file and the next.
 *
 * A single object rather than loose bindings: ES modules export live but
 * read-only views, so the panels that reassign these have to write through a
 * shared holder.
 */
/** One lazily loaded wasm-pack module and whether its init has completed. */
interface ModuleSlot {
  loaded: boolean;
  module: any;
}

/** The lazily loaded wasm-pack modules, by format key. */
export const modules: Record<string, ModuleSlot> = {
  obj: { loaded: false, module: null },
  ply: { loaded: false, module: null },
  gltf: { loaded: false, module: null },
  fbx: { loaded: false, module: null },
};

export interface AppState {
  /** Parse result for the loaded file, in whatever shape its reader returns. */
  currentMeshData: any;
  currentFileType: string | null;
  currentSourceData: Uint8Array | null;
  currentSourceResources: ResourceMap;
  /**
   * FBX uses the source-neutral SceneDocument for cross-format GLB export.
   * Direct glTF/GLB inputs continue to use their lossless source-byte route.
   */
  currentSceneDocument: SceneDocument | null;
  currentFbxProvenance: FbxSceneProvenance | null;
  /** The 3D preview, created lazily on first use. */
  viewer: Viewer | null;
}

export const state: AppState = {
  currentMeshData: null,
  currentFileType: null,
  currentSourceData: null,
  currentSourceResources: Object.create(null),
  currentSceneDocument: null,
  currentFbxProvenance: null,
  viewer: null,
};
