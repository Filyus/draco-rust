import { GL } from './gl-utils.ts';

/**
 * Image upload and sampler configuration.
 *
 * A texture starts as a single opaque white texel so a mesh is never drawn
 * against uninitialized memory while its bitmap is still decoding.
 */

/** One mip level of a texture already in a GPU block format. */
export interface CompressedLevel {
  width: number;
  height: number;
  bytes: Uint8Array;
}

/** One viewer texture: decoded image plus its sampler parameters. */
export interface ViewerTexture {
  image?: ImageBitmap | HTMLImageElement | null;
  /**
   * Mip levels already in a format the GPU samples directly.
   *
   * Present instead of `image` for a KTX2 texture on a machine whose context
   * takes the block format it was transcoded into.
   */
  compressed?: { format: number; levels: CompressedLevel[] } | null;
  flipY?: boolean;
  wrapS?: number;
  wrapT?: number;
  minFilter?: number;
  magFilter?: number;
}

export function uploadImage(gl: WebGL2RenderingContext, tex: ViewerTexture): WebGLTexture {
  const glTexture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, glTexture);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, !!tex.flipY);
  // Placeholder color while the bitmap is decoding.
  gl.texImage2D(
    gl.TEXTURE_2D,
    0,
    gl.RGBA,
    1,
    1,
    0,
    gl.RGBA,
    gl.UNSIGNED_BYTE,
    new Uint8Array([255, 255, 255, 255]),
  );
  if (tex.compressed && tex.compressed.levels.length > 0) {
    uploadCompressed(gl, glTexture, tex);
  } else if (tex.image instanceof ImageBitmap || tex.image instanceof HTMLImageElement) {
    gl.bindTexture(gl.TEXTURE_2D, glTexture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, tex.image);
    setSampler(gl, tex);
  }
  return glTexture;
}

/**
 * Upload a texture that is already in a block format.
 *
 * Its mip chain comes from the file rather than from `generateMipmap`, which
 * cannot run on a compressed texture: the driver would have to decompress,
 * filter and recompress, so GL simply refuses. That also means the chain has
 * to be declared complete — `TEXTURE_MAX_LEVEL` — or sampling reads levels
 * that were never uploaded and the texture draws black.
 */
function uploadCompressed(
  gl: WebGL2RenderingContext,
  glTexture: WebGLTexture,
  tex: ViewerTexture,
) {
  const { format, levels } = tex.compressed!;
  gl.bindTexture(gl.TEXTURE_2D, glTexture);
  // Compressed uploads take the rows as they are stored; the flip a decoded
  // bitmap might need cannot be applied to blocks.
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
  levels.forEach((level, index) => {
    gl.compressedTexImage2D(
      gl.TEXTURE_2D,
      index,
      format,
      level.width,
      level.height,
      0,
      level.bytes,
    );
  });
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAX_LEVEL, levels.length - 1);
  setSampler(gl, tex, levels.length);
}

/**
 * Apply the sampler state.
 *
 * `mipLevels` says how many levels the texture actually has. One level with a
 * mipmapping filter is an incomplete texture, which samples as black, so the
 * filter is lowered instead — a single-level KTX2 file is common enough that
 * refusing to draw it would be the wrong answer.
 */
export function setSampler(gl: WebGL2RenderingContext, tex: ViewerTexture, mipLevels?: number) {
  const wrapS = tex.wrapS || GL.REPEAT;
  const wrapT = tex.wrapT || GL.REPEAT;
  let minFilter = tex.minFilter || GL.LINEAR_MIPMAP_LINEAR;
  const magFilter = tex.magFilter || GL.LINEAR;
  if (mipLevels === 1 && minFilter !== GL.NEAREST && minFilter !== GL.LINEAR) {
    minFilter = GL.LINEAR;
  }
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, wrapS);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, wrapT);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, minFilter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, magFilter);
  if (mipLevels === undefined) gl.generateMipmap(gl.TEXTURE_2D);
}
