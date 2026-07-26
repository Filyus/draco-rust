import { GL } from './gl-utils.ts';

/**
 * Image upload and sampler configuration.
 *
 * A texture starts as a single opaque white texel so a mesh is never drawn
 * against uninitialized memory while its bitmap is still decoding.
 */

/** One viewer texture: decoded image plus its sampler parameters. */
export interface ViewerTexture {
  image?: ImageBitmap | HTMLImageElement | null;
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
  if (tex.image instanceof ImageBitmap || tex.image instanceof HTMLImageElement) {
    gl.bindTexture(gl.TEXTURE_2D, glTexture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, tex.image);
    setSampler(gl, tex);
  }
  return glTexture;
}

export function setSampler(gl: WebGL2RenderingContext, tex: ViewerTexture) {
  const wrapS = tex.wrapS || GL.REPEAT;
  const wrapT = tex.wrapT || GL.REPEAT;
  const minFilter = tex.minFilter || GL.LINEAR_MIPMAP_LINEAR;
  const magFilter = tex.magFilter || GL.LINEAR;
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, wrapS);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, wrapT);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, minFilter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, magFilter);
  gl.generateMipmap(gl.TEXTURE_2D);
}
