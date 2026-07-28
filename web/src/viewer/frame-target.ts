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
 * How well it is sampled is the other half of the problem. What a transmissive
 * surface shows is this texture rather than the canvas, so the canvas keeping
 * its own antialiasing does nothing for the scene behind glass — and a
 * filament twenty-five times brighter than white is the case that punishes
 * every shortcut. Multisampling alone leaves as many gradations as it has
 * samples, and at that contrast the tone map crushes all but the faintest of
 * them back into white: the staircase survives. Averaging somewhere compressed,
 * so that the result survives tone mapping, hides the staircase by losing
 * energy instead — a half-covered texel comes back at a twentieth of its
 * brightness, and since the compression is per-channel it comes back the wrong
 * hue as well, which reads as bright and dim texels alternating along one wire.
 *
 * So the capture is drawn larger than it is read, multisampled, and brought
 * down by an exact box filter in linear light. Every step conserves energy, and
 * the gradations are the product of the two: sixteen where the machine offers
 * four samples at twice the size. The scale falls back to one where the frame
 * is already large enough that four times the texels would cost more memory
 * than the artefact is worth.
 */

/**
 * Above this many texels in the frame itself, the capture stops being drawn at
 * twice the size: four times the texels of a large frame runs to hundreds of
 * megabytes, and the staircase it would smooth is not worth that.
 */
const MAX_SUPERSAMPLED_PIXELS = 1_200_000;

/** The opaque half of the frame, mipmapped for roughness-driven blur. */
export interface FrameTarget {
  /** Where the pass draws when the machine offers more than one sample. */
  sampleFramebuffer: WebGLFramebuffer | null;
  sampleColor: WebGLRenderbuffer | null;
  depth: WebGLRenderbuffer;
  /** The resolve's destination, still at the size the capture was drawn. */
  resolveFramebuffer: WebGLFramebuffer;
  resolved: WebGLTexture;
  /** The downsample's destination: what the frame reads, mipmapped. */
  framebuffer: WebGLFramebuffer;
  color: WebGLTexture;
  /** The size the frame is sampled at: the drawing buffer's own. */
  width: number;
  height: number;
  /** The size it is drawn at, which is `scale` times larger. */
  captureWidth: number;
  captureHeight: number;
  scale: number;
  /** Whether the colour attachments hold half floats rather than bytes. */
  hdr: boolean;
  /** Samples per texel the capture is drawn with; one when unavailable. */
  samples: number;
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

function captureTexture(
  gl: WebGL2RenderingContext,
  width: number,
  height: number,
  hdr: boolean,
  mipmapped: boolean,
  filter?: number,
) {
  const texture = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texImage2D(
    gl.TEXTURE_2D, 0, hdr ? gl.RGBA16F : gl.RGBA8, width, height, 0,
    gl.RGBA, hdr ? gl.HALF_FLOAT : gl.UNSIGNED_BYTE, null,
  );
  // A transmissive surface picks its mip level from its own roughness, so the
  // levels are declared on the texture it reads and filled on every capture.
  const chosen = filter ?? gl.NEAREST;
  gl.texParameteri(
    gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, mipmapped ? gl.LINEAR_MIPMAP_LINEAR : chosen);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, mipmapped ? gl.LINEAR : chosen);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.bindTexture(gl.TEXTURE_2D, null);
  return texture;
}

function framebufferFor(gl: WebGL2RenderingContext, texture: WebGLTexture) {
  const framebuffer = gl.createFramebuffer()!;
  gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
  return framebuffer;
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

  // Twice the size while the frame is small enough to afford it, and as many
  // samples as the driver offers on top: the two multiply, so a machine giving
  // eight leaves thirty-two gradations along an edge where the reference
  // renderers have four. Past the budget the scale drops to one and the samples
  // carry it alone.
  const scale = width * height <= MAX_SUPERSAMPLED_PIXELS ? 2 : 1;
  const samples = Math.min(8, gl.getParameter(gl.MAX_SAMPLES) as number);
  const multisampled = samples > 1;
  const captureWidth = width * scale;
  const captureHeight = height * scale;

  // Read with a single bilinear tap per output texel, which is an exact box
  // average only because the filter is linear and the scale is two.
  const resolved = captureTexture(gl, captureWidth, captureHeight, hdr, false, gl.LINEAR);
  const color = captureTexture(gl, width, height, hdr, true);
  const resolveFramebuffer = framebufferFor(gl, resolved);
  const framebuffer = framebufferFor(gl, color);

  // The capture draws the same geometry the canvas pass does, so it needs the
  // same depth test; only the colour is ever read back. It belongs to whichever
  // framebuffer the pass draws into.
  const depth = gl.createRenderbuffer()!;
  gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
  if (multisampled) {
    gl.renderbufferStorageMultisample(
      gl.RENDERBUFFER, samples, gl.DEPTH_COMPONENT24, captureWidth, captureHeight);
  } else {
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT24, captureWidth, captureHeight);
    gl.bindFramebuffer(gl.FRAMEBUFFER, resolveFramebuffer);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
  }
  gl.bindRenderbuffer(gl.RENDERBUFFER, null);

  // Samples cannot live in a texture, so the pass draws into renderbuffers and
  // a blit resolves them into one.
  let sampleFramebuffer: WebGLFramebuffer | null = null;
  let sampleColor: WebGLRenderbuffer | null = null;
  if (multisampled) {
    sampleColor = gl.createRenderbuffer()!;
    gl.bindRenderbuffer(gl.RENDERBUFFER, sampleColor);
    gl.renderbufferStorageMultisample(
      gl.RENDERBUFFER, samples, hdr ? gl.RGBA16F : gl.RGBA8, captureWidth, captureHeight);
    gl.bindRenderbuffer(gl.RENDERBUFFER, null);

    sampleFramebuffer = gl.createFramebuffer()!;
    gl.bindFramebuffer(gl.FRAMEBUFFER, sampleFramebuffer);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, sampleColor);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
  }

  const target: FrameTarget = {
    sampleFramebuffer,
    sampleColor,
    depth,
    resolveFramebuffer,
    resolved,
    framebuffer,
    color,
    width,
    height,
    captureWidth,
    captureHeight,
    scale,
    hdr,
    samples: multisampled ? samples : 1,
  };

  for (const each of [sampleFramebuffer, resolveFramebuffer, framebuffer]) {
    if (!each) continue;
    gl.bindFramebuffer(gl.FRAMEBUFFER, each);
    const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
    if (status !== gl.FRAMEBUFFER_COMPLETE) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      disposeFrameTarget(gl, target);
      throw new Error(`transmission framebuffer is incomplete (0x${status.toString(16)})`);
    }
  }
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  return target;
}

export function disposeFrameTarget(gl: WebGL2RenderingContext, target: FrameTarget | null) {
  if (!target) return;
  if (target.sampleFramebuffer) gl.deleteFramebuffer(target.sampleFramebuffer);
  if (target.sampleColor) gl.deleteRenderbuffer(target.sampleColor);
  gl.deleteFramebuffer(target.resolveFramebuffer);
  gl.deleteFramebuffer(target.framebuffer);
  gl.deleteRenderbuffer(target.depth);
  gl.deleteTexture(target.resolved);
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
  gl.bindFramebuffer(gl.FRAMEBUFFER, target.sampleFramebuffer ?? target.resolveFramebuffer);
  gl.viewport(0, 0, target.captureWidth, target.captureHeight);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
}

/**
 * Resolve the samples, and point drawing at where the downsample goes.
 *
 * The caller draws one fullscreen pass between this and `finishFrameCapture`;
 * what that pass is belongs with the shaders.
 */
export function resolveFrameCapture(gl: WebGL2RenderingContext, target: FrameTarget) {
  if (target.sampleFramebuffer) {
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, target.sampleFramebuffer);
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, target.resolveFramebuffer);
    // A multisampled read resolves only into a rectangle of its own size, and
    // only with NEAREST; both are what a resolve means rather than a choice.
    gl.blitFramebuffer(
      0, 0, target.captureWidth, target.captureHeight,
      0, 0, target.captureWidth, target.captureHeight,
      gl.COLOR_BUFFER_BIT, gl.NEAREST,
    );
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, null);
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null);
  }
  gl.bindFramebuffer(gl.FRAMEBUFFER, target.framebuffer);
  gl.viewport(0, 0, target.width, target.height);
}

/** Fill the mip chain and hand drawing back to the canvas. */
export function finishFrameCapture(gl: WebGL2RenderingContext, target: FrameTarget) {
  gl.bindTexture(gl.TEXTURE_2D, target.color);
  gl.generateMipmap(gl.TEXTURE_2D);
  gl.bindTexture(gl.TEXTURE_2D, null);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
}
