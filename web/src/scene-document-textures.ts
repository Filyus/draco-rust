/**
 * Decoding the texture bytes a SceneDocument carries into images the GPU can
 * take.
 *
 * `scene-document-viewer.ts` builds a runtime scene without touching the DOM —
 * that is what lets the node gates import it without a WebGL or `ImageBitmap`
 * stand-in — so it hands over each texture's source bytes and stops there. A
 * scene in that state uploads as the placeholder white texel `uploadImage`
 * starts every texture with, and renders every textured model flat white.
 *
 * This is the step that finishes the job, kept separate for the same reason:
 * it is the only part that needs a browser.
 */

import { mimeFromUri, sniffMime } from './scene-resources.ts';
import type { ViewerScene, ViewerTexture } from './viewer-scene.ts';

/**
 * Decode every texture in `scene` that arrived as bytes.
 *
 * Textures are decoded concurrently and the scene is mutated in place: the
 * caller already holds it, and returning a copy would mean rebuilding the
 * material indices that point into it.
 *
 * A texture that cannot be decoded keeps `image` null and reports itself. That
 * matches the glTF loader, which also warns and carries on: one unreadable
 * image is not a reason to refuse the model.
 */
export async function hydrateSceneTextures(scene: ViewerScene): Promise<ViewerScene> {
  await Promise.all(scene.textures.map(async (texture, index) => {
    if (!texture || texture.image || !texture.bytes) return;
    const warning = await decodeInto(texture, index);
    if (warning) scene.warnings.push(warning);
  }));
  return scene;
}

/** @returns The warning to report, or null when the image decoded. */
async function decodeInto(texture: ViewerTexture, index: number): Promise<string | null> {
  const bytes = texture.bytes!;
  const mime = texture.mimeType || sniffMime(bytes) || mimeFromUri(texture.name || '');
  if (mime === 'image/ktx2') {
    // Worded exactly as the glTF loader words it: the two paths are making the
    // same observation about the same asset, and a user comparing them should
    // not have to wonder whether they mean different things.
    return 'KTX2 textures require a transcoder; skipping image';
  }
  try {
    // BlobPart excludes SharedArrayBuffer-backed views; these never are.
    const blob = new Blob([bytes as BlobPart], { type: mime || 'application/octet-stream' });
    texture.image = await createImageBitmap(blob);
    return null;
  } catch (error) {
    return `Failed to decode texture ${texture.name || index}: ${(error as Error).message}`;
  }
}
