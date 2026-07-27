/**
 * A texture copy of the frame as it stood once the opaque surfaces were drawn.
 *
 * Drawing straight to the canvas is enough right up until a surface needs to
 * see what is behind it. `KHR_materials_transmission` is that case: a
 * transmissive material samples the scene already drawn, blurred by its own
 * roughness, so what the opaque half produced has to exist as a texture before
 * the transmissive half runs.
 *
 * It is a copy *out of* the canvas rather than a buffer the frame is composed
 * in, and that is deliberate: the context is multisampled, and routing the
 * visible frame through a single-sample framebuffer would quietly cost it its
 * antialiasing. Copying reads through the resolve instead, so the image on
 * screen is exactly what it was.
 */

/** The opaque half of the frame, mipmapped for roughness-driven blur. */
export interface FrameTarget {
  color: WebGLTexture;
  width: number;
  height: number;
}

/**
 * The snapshot texture for a drawing buffer of this size, reusing the previous
 * one while the size holds.
 *
 * A canvas resize is the one event that invalidates it: the texture is
 * allocated at a fixed size, and `copyTexSubImage2D` into a smaller one would
 * capture a corner of the frame rather than the frame.
 */
export function ensureFrameTarget(
  gl: WebGL2RenderingContext,
  current: FrameTarget | null,
  width: number,
  height: number,
): FrameTarget {
  if (current && current.width === width && current.height === height) return current;
  if (current) disposeFrameTarget(gl, current);

  const color = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, color);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
  // A transmissive surface picks its mip level from its own roughness, so the
  // levels are declared here and filled on every capture.
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.bindTexture(gl.TEXTURE_2D, null);

  return { color, width, height };
}

export function disposeFrameTarget(gl: WebGL2RenderingContext, target: FrameTarget | null) {
  if (!target) return;
  gl.deleteTexture(target.color);
}

/**
 * Take the snapshot, mip levels included.
 *
 * Called between the opaque and the blended halves of the frame, which is the
 * only moment when the canvas holds "everything a transmissive surface is
 * allowed to see" — after it, the blended surfaces themselves are in it.
 */
export function captureFrameTarget(gl: WebGL2RenderingContext, target: FrameTarget) {
  gl.bindTexture(gl.TEXTURE_2D, target.color);
  gl.copyTexSubImage2D(gl.TEXTURE_2D, 0, 0, 0, 0, 0, target.width, target.height);
  gl.generateMipmap(gl.TEXTURE_2D);
  gl.bindTexture(gl.TEXTURE_2D, null);
}
