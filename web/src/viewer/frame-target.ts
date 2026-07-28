/**
 * The frame as it stood once the opaque surfaces were drawn, in linear light.
 *
 * Drawing straight to the canvas is enough right up until a surface needs to
 * see what is behind it. `KHR_materials_transmission` is that case: a
 * transmissive material samples the scene already drawn, blurred by its own
 * roughness, so what the opaque half produced has to exist as a texture before
 * the transmissive half runs.
 *
 * The scene is drawn into this target a second time rather than copied out of
 * the canvas, and that is the whole point: the shader reads the result as
 * radiance, so it has to *be* radiance. The canvas holds tone-mapped,
 * gamma-encoded pixels, and reading those back as light applies both a second
 * time — which is what the reference implementations avoid by rendering their
 * transmission pass with tone mapping switched off, and what Babylon fixed
 * after shipping the same bug. It also fixes the blur: mip levels averaged over
 * gamma-encoded pixels are not the average of the light they stand for.
 *
 * The target is not multisampled, and does not need to be. What is on screen is
 * still drawn by the pass that always drew it, with the canvas's own
 * antialiasing intact; this texture is only ever read through refraction, and
 * usually through the mip chain. three.js and the Khronos sample renderer both
 * multisample theirs because their transmission pass is the only time they draw
 * that geometry — for us it is the second time.
 */

/** The opaque half of the frame, mipmapped for roughness-driven blur. */
export interface FrameTarget {
  framebuffer: WebGLFramebuffer;
  color: WebGLTexture;
  depth: WebGLRenderbuffer;
  width: number;
  height: number;
  /** Whether the colour attachment holds half floats rather than bytes. */
  hdr: boolean;
}

/**
 * Whether this context can render into a half-float colour attachment.
 *
 * Worth asking because the values stored here are linear: eight bits are
 * enough for a display-encoded image and not for the light behind it, where
 * the same steps land far apart in the darks and cannot describe anything
 * brighter than white. Either extension makes RGBA16F renderable, and WebGL2
 * filters it without a further one, which the mip chain needs.
 */
export function frameTargetHdrSupported(gl: WebGL2RenderingContext): boolean {
  return !!(gl.getExtension('EXT_color_buffer_half_float')
    || gl.getExtension('EXT_color_buffer_float'));
}

/**
 * The snapshot target for a drawing buffer of this size, reusing the previous
 * one while the size holds.
 *
 * A canvas resize is the one event that invalidates it: the attachments are
 * allocated at a fixed size, and a smaller target would capture the frame at
 * the wrong scale rather than capture a smaller frame.
 */
export function ensureFrameTarget(
  gl: WebGL2RenderingContext,
  current: FrameTarget | null,
  width: number,
  height: number,
  hdr: boolean,
): FrameTarget {
  if (current && current.width === width && current.height === height && current.hdr === hdr) {
    return current;
  }
  if (current) disposeFrameTarget(gl, current);

  const color = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, color);
  gl.texImage2D(
    gl.TEXTURE_2D, 0, hdr ? gl.RGBA16F : gl.RGBA8, width, height, 0,
    gl.RGBA, hdr ? gl.HALF_FLOAT : gl.UNSIGNED_BYTE, null,
  );
  // A transmissive surface picks its mip level from its own roughness, so the
  // levels are declared here and filled on every capture.
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.bindTexture(gl.TEXTURE_2D, null);

  // The capture draws the same geometry the canvas pass does, so it needs the
  // same depth test; only the colour is ever read back.
  const depth = gl.createRenderbuffer()!;
  gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
  gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT24, width, height);
  gl.bindRenderbuffer(gl.RENDERBUFFER, null);

  const framebuffer = gl.createFramebuffer()!;
  gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, color, 0);
  gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
  const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  if (status !== gl.FRAMEBUFFER_COMPLETE) {
    gl.deleteFramebuffer(framebuffer);
    gl.deleteRenderbuffer(depth);
    gl.deleteTexture(color);
    throw new Error(`transmission framebuffer is incomplete (0x${status.toString(16)})`);
  }

  return { framebuffer, color, depth, width, height, hdr };
}

export function disposeFrameTarget(gl: WebGL2RenderingContext, target: FrameTarget | null) {
  if (!target) return;
  gl.deleteFramebuffer(target.framebuffer);
  gl.deleteRenderbuffer(target.depth);
  gl.deleteTexture(target.color);
}

/**
 * Point drawing at the target and clear it.
 *
 * Cleared with whatever colour the canvas is cleared with, and it does not
 * matter which: the background pass covers every pixel, and only the colour
 * channels are ever read back.
 */
export function beginFrameCapture(gl: WebGL2RenderingContext, target: FrameTarget) {
  gl.bindFramebuffer(gl.FRAMEBUFFER, target.framebuffer);
  gl.viewport(0, 0, target.width, target.height);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
}

/** Fill the mip chain and hand drawing back to the canvas. */
export function finishFrameCapture(gl: WebGL2RenderingContext, target: FrameTarget) {
  gl.bindTexture(gl.TEXTURE_2D, target.color);
  gl.generateMipmap(gl.TEXTURE_2D);
  gl.bindTexture(gl.TEXTURE_2D, null);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
}
