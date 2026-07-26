/** FBX material and texture import boundary for the format-neutral viewer. */
import { basename, mimeFromUri, resolveResource } from './scene-resources.ts';
import type { ResourceMap } from './scene-resources.ts';

/** One material as fbx-wasm materializes it, before viewer interpretation. */
interface FbxMaterial {
  name?: string;
  diffuse?: number[];
  diffuseFactor?: number;
  emissive?: number[];
  emissiveFactor?: number;
  opacity?: number;
  transparencyFactor?: number;
  shininess?: number;
  reflectionFactor?: number;
}

interface FbxTexture {
  name?: string;
  filename?: string;
  content?: Uint8Array;
}

/** A decoded texture, ready for the viewer to upload. */
export interface AdaptedFbxTexture {
  name: string;
  image: ImageBitmap;
  flipY: boolean;
  wrapS: number;
  wrapT: number;
  minFilter: number;
  magFilter: number;
}

/** Diagnostics sink shared with the rest of the FBX import path. */
interface AdapterHooks {
  onLog?: (message: string, level: string) => void;
}

/** Convert an FBX Phong/Lambert material into the viewer material contract. */
export function adaptFbxMaterial(material: FbxMaterial, index: number) {
  const diffuse = material.diffuse || [1, 1, 1];
  const diffuseFactor = typeof material.diffuseFactor === 'number' ? material.diffuseFactor : 1;
  const emissive = material.emissive || [0, 0, 0];
  const emissiveFactor = typeof material.emissiveFactor === 'number' ? material.emissiveFactor : 1;
  const alpha = typeof material.opacity === 'number'
    ? material.opacity
    : typeof material.transparencyFactor === 'number' ? 1 - material.transparencyFactor : 1;
  const [dr, dg, db] = diffuse.map((component) => component * diffuseFactor);
  const [er, eg, eb] = emissive.map((component) => component * emissiveFactor);
  const shininess = typeof material.shininess === 'number' ? Math.max(material.shininess, 0) : 20;
  return {
    name: material.name || `material_${index}`,
    baseColorFactor: [dr, dg, db, alpha],
    baseColorTexture: null,
    baseColorTexCoord: 0,
    baseColorTextureTransform: { offset: [0, 0], scale: [1, 1], rotation: 0 },
    metallic: typeof material.reflectionFactor === 'number' ? material.reflectionFactor : 0,
    roughness: Math.min(1, Math.max(0, 1 - Math.sqrt(shininess) / 10)),
    metallicRoughnessTexture: null,
    emissiveFactor: [er, eg, eb],
    emissiveTexture: null,
    normalTexture: null,
    occlusionTexture: null,
    doubleSided: false,
    alphaMode: alpha < 1 ? 'BLEND' : 'OPAQUE',
    alphaCutoff: 0.5,
    unlit: false,
  };
}

/** Decode FBX texture objects (embedded bytes or selected external files). */
export async function adaptFbxTextures(
  fbxTextures: FbxTexture[],
  resources: ResourceMap,
  warnings: string[],
  hooks: AdapterHooks,
): Promise<AdaptedFbxTexture[]> {
  const textures: AdaptedFbxTexture[] = [];
  for (let index = 0; index < fbxTextures.length; index++) {
    const texture = fbxTextures[index];
    const bytes = texture.content && texture.content.length > 0
      ? texture.content
      : texture.filename ? resolveResource(texture.filename, resources) : null;
    if (!bytes) {
      if (texture.filename) warnings.push(`FBX texture not selected: ${texture.filename}`);
      continue;
    }
    try {
      // BlobPart insists on an ArrayBuffer-backed view; these bytes never
      // come from a SharedArrayBuffer, which is the only case it excludes.
      const part = bytes as BlobPart;
      const bitmap = await createImageBitmap(new Blob([part], { type: mimeFromUri(texture.filename) || 'application/octet-stream' }));
      textures[index] = {
        name: texture.name || basename(texture.filename || `texture_${index}`),
        image: bitmap,
        flipY: true,
        wrapS: WebGL2RenderingContext.REPEAT,
        wrapT: WebGL2RenderingContext.REPEAT,
        minFilter: WebGL2RenderingContext.LINEAR_MIPMAP_LINEAR,
        magFilter: WebGL2RenderingContext.LINEAR,
      };
    } catch (error) {
      const message = `Failed to decode FBX texture ${texture.filename}: ${(error as Error).message}`;
      warnings.push(message);
      hooks.onLog?.(message, 'warning');
    }
  }
  return textures;
}

