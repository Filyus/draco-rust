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
 * Decoding is per *image*, not per texture: several textures reading one
 * resource share a single `ImageBitmap`, the way the glTF loader's own
 * `decodeImages` does. That sharing is not a micro-optimization — the viewer
 * keys its GPU uploads on bitmap identity, so one bitmap per image is what
 * keeps a model whose 24 textures name the same 4096² JPEG at one GL texture
 * instead of 24. Decoding per texture costs the decode again *and* the upload
 * again, and the upload is the larger of the two.
 *
 * Images are decoded concurrently and the scene is mutated in place: the
 * caller already holds it, and returning a copy would mean rebuilding the
 * material indices that point into it.
 *
 * An image that cannot be decoded leaves every texture reading it with a null
 * `image` and is reported once, named after the first of them. That matches
 * the glTF loader, which also warns per image and carries on: one unreadable
 * image is not a reason to refuse the model.
 */
export async function hydrateSceneTextures(scene: ViewerScene): Promise<ViewerScene> {
  const groups = new Map<unknown, number[]>();
  scene.textures.forEach((texture, index) => {
    if (!texture || texture.image || !texture.bytes) return;
    // A scene built without resource indices — anything but the SceneDocument
    // adapter — has no way to say two textures are one image, so each stands
    // alone and the behaviour is what it was.
    const key = texture.resource ?? `texture:${index}`;
    const group = groups.get(key);
    if (group) group.push(index);
    else groups.set(key, [index]);
  });

  await Promise.all([...groups.values()].map(async (indices) => {
    const first = scene.textures[indices[0]];
    const warning = await decodeInto(first, indices[0]);
    if (warning) scene.warnings.push(warning);
    for (const index of indices.slice(1)) scene.textures[index].image = first.image;
  }));
  return scene;
}

/**
 * glTF reaches these codecs only through their extension, so the codec a
 * resource is stored in says which extension named it. The document does not
 * record that on the texture, and it should not have to: it carries the bytes
 * whatever the codec, and only a consumer that has to decode them cares.
 */
const SOURCE_EXTENSION_BY_MIME: Record<string, string> = {
  'image/webp': 'EXT_texture_webp',
  'image/ktx2': 'KHR_texture_basisu',
};

/**
 * Per alternate-source extension, whether every texture that used it decoded.
 *
 * An observation about this browser, not a support claim: the codec belongs to
 * the host. Read off a hydrated scene, because before hydration the answer is
 * "not yet" for all of them and after it the scene holds the outcome.
 */
export function honoredTextureSources(scene: ViewerScene): Map<string, boolean> {
  const honored = new Map<string, boolean>();
  for (const texture of scene.textures) {
    const extension = SOURCE_EXTENSION_BY_MIME[texture?.mimeType ?? ''];
    if (!extension) continue;
    honored.set(extension, (honored.get(extension) ?? true) && Boolean(texture.image));
  }
  return honored;
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
