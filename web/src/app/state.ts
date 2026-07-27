import type { FbxSceneProvenance } from '../fbx-scene-provenance.ts';
import type { GltfSceneProvenance } from '../gltf-scene-provenance.ts';
import type { LoadedMesh, LoadedObjMaterial } from '../mesh-loader.ts';
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
/**
 * One lazily loaded wasm-pack module and whether its init has completed.
 *
 * The module itself stays open: wasm-bindgen generates a different surface per
 * crate, the four here share no interface, and the bindings are regenerated on
 * every build. Describing them by hand would mean four hand-written mirrors
 * that drift the first time a signature changes.
 */
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
  // Fetched only when a file turns out to carry a KTX2 texture: it is the
  // largest of the modules and the rarest to be needed.
  ktx2: { loaded: false, module: null },
};

/**
 * The FBX semantic scene, as `parse_fbx` hands it over.
 *
 * Deliberately left open: it is a large reader structure that two independent
 * adapters interpret, and describing it properly belongs with reconciling those
 * adapters rather than with this pass. Named so that the gap is visible instead
 * of being one more anonymous `any`.
 */
export type FbxSceneData = any;

/**
 * The parse result for the loaded file, as its reader returned it.
 *
 * One shape rather than three, because the shell carries whichever reader ran
 * in a single slot and branches on which fields are present: `document` marks
 * the glTF route, `scene` the FBX route, and `meshes` alone is OBJ or PLY.
 */
export interface LoadedFile {
  success?: boolean;
  error?: string;
  warnings?: string[];
  meshes?: LoadedMesh[];
  materials?: Record<string, LoadedObjMaterial>;
  /**
   * glTF only, and a marker rather than the document itself: the SceneDocument
   * lives in `currentSceneDocument`, which may be null when this is set.
   */
  document?: boolean;
  format?: string;
  meshCount?: number;
  vertexCount?: number;
  triangleCount?: number;
  hasNormals?: boolean;
  hasUvs?: boolean;
  /** FBX only. */
  scene?: FbxSceneData;
}

export interface AppState {
  /** Parse result for the loaded file, in whatever shape its reader returns. */
  currentMeshData: LoadedFile | null;
  currentFileType: string | null;
  currentSourceData: Uint8Array | null;
  currentSourceResources: ResourceMap;
  /**
   * FBX uses the source-neutral SceneDocument for cross-format GLB export.
   * Direct glTF/GLB inputs continue to use their lossless source-byte route.
   */
  currentSceneDocument: SceneDocument | null;
  currentFbxProvenance: FbxSceneProvenance | null;
  /**
   * What the loaded glTF claimed about its own extensions.
   *
   * Kept because the preview has to report what it did not act on, and the
   * document deliberately does not record the file's claims about itself.
   */
  currentGltfProvenance: GltfSceneProvenance | null;
  /** The material variant the preview is showing, or null for the default one. */
  currentVariant: number | null;
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
  currentGltfProvenance: null,
  currentVariant: null,
  viewer: null,
};
