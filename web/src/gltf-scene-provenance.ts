/**
 * Optional, glTF-only provenance kept beside a portable SceneDocument.
 *
 * SceneDocument stays source-neutral: it records what a scene *is*, not what
 * the file that carried it claimed about itself. But a consumer that has to
 * report what it could not act on needs those claims, and re-opening the asset
 * to read two arrays is exactly the second parse this pairing exists to avoid.
 *
 * So the extension lists travel alongside, the way the FBX evaluator
 * convention does in `fbx-scene-provenance.ts`. What each consumer then counts
 * as honored is its own policy and stays with the consumer: the document
 * carries image bytes whatever the codec, while the preview can only claim an
 * alternate image source once the browser has actually decoded it.
 */

export const GLTF_SCENE_PROVENANCE_VERSION = 1;

export interface GltfSceneProvenance {
  version: number;
  format: 'gltf';
  /** `extensionsUsed`, verbatim and in document order. */
  extensionsUsed: string[];
  /** `extensionsRequired`, verbatim and in document order. */
  extensionsRequired: string[];
}

/** Read the extension claims out of a parsed glTF manifest. */
export function createGltfSceneProvenance(manifest: unknown): GltfSceneProvenance {
  const root = (manifest ?? {}) as { extensionsUsed?: unknown; extensionsRequired?: unknown };
  return {
    version: GLTF_SCENE_PROVENANCE_VERSION,
    format: 'gltf',
    extensionsUsed: names(root.extensionsUsed),
    extensionsRequired: names(root.extensionsRequired),
  };
}

/** External JSON, so anything that is not a list of strings is no claim at all. */
function names(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((name): name is string => typeof name === 'string') : [];
}
