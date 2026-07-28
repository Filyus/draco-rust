/**
 * The frame the scene is drawn into: linear light, at more than screen
 * resolution, kept until the output pass turns it into a picture.
 *
 * Everything the viewer draws lands here rather than on the canvas — backdrop,
 * grid, surfaces, the transmissive half — and nothing tone maps on the way in.
 * That single decision is what makes the rest correct rather than nearly
 * correct: light adds and averages linearly, so every step that combines two
 * values has to happen before the curve that turns light into a display
 * signal, not after. The multisample resolve, the mip chain a rough refraction
 * reads, the glare the output pass spreads - all of them are averages, and all
 * of them are wrong by construction if the values have already been through a
 * tone map.
 *
 * It can be drawn larger than it is shown and brought down by an exact box
 * filter at the end. Multisampling antialiases coverage; supersampling
 * antialiases everything else with it - shading, highlights, the specular
 * glint along an edge - and the two multiply rather than overlap. It is off by
 * default all the same, because unlike samples it multiplies the *shading*
 * cost: four times the fragments, every one of them a full material
 * evaluation. That is a trade to opt into on a still, not to make on
 * everyone's behalf every frame.
 *
 * Transmission takes a copy of it partway through, once the opaque half is
 * down. That is a blit rather than a second pass over the geometry: the frame
 * is already the linear light a refracted ray wants to read, which it was not
 * back when the canvas was the only buffer.
 */

/**
 * Above this many texels supersampling is refused whatever was asked for: four
 * times the texels of a large frame is hundreds of megabytes of attachments.
 */
const MAX_SUPERSAMPLED_PIXELS = 1_200_000;

/**
 * How much scene to draw beyond what is shown, as a fraction of the frame per
 * side.
 *
 * Transmission reads the frame, and a refracted ray does not care where the
 * frame ends: leaving it, there is nothing to read, and the border texel
 * repeats into a streak. Every substitute for that is an invention. Drawing
 * wider is not - the ray lands in geometry that was actually there, and the
 * only thing spent is fill.
 *
 * A fifth of the frame per side costs about ninety per cent more texels and
 * covers refraction through anything of ordinary thickness. Nothing else in
 * the viewer sees it: the output pass shows the middle, and it is the only
 * pass that writes to the canvas.
 */
export const GUARD_BAND = {
  /** Fraction of the shown frame added on each side. Zero draws only it. */
  margin: 0.2,
};

export interface SceneTarget {
  /** Multisampled draw target; null when the driver offers one sample. */
  framebuffer: WebGLFramebuffer | null;
  color: WebGLRenderbuffer | null;
  depth: WebGLRenderbuffer;
  /** Where the samples resolve to, and what the output pass reads. */
  resolveFramebuffer: WebGLFramebuffer;
  resolved: WebGLTexture;
  /** The opaque half, mipmapped, as a refracted ray reads it. */
  captureFramebuffer: WebGLFramebuffer;
  capture: WebGLTexture;
  /** The canvas size the frame is shown at. */
  width: number;
  height: number;
  /** The size it is drawn at: `scale` times larger, and wider by the guard. */
  renderWidth: number;
  renderHeight: number;
  scale: number;
  /**
   * How much wider than the canvas the frame is, as a factor. The output pass
   * crops by its reciprocal, and the projection widens by it.
   */
  guard: number;
  samples: number;
  hdr: boolean;
}

/**
 * Whether this context can render into a half-float colour attachment.
 *
 * The whole pipeline depends on it: eight bits describe a display signal, not
 * light, and the values here run past white wherever an emitter does.
 */
export function sceneTargetHdrSupported(gl: WebGL2RenderingContext): boolean {
  return !!(gl.getExtension('EXT_color_buffer_half_float')
    || gl.getExtension('EXT_color_buffer_float'));
}

export function linearTexture(
  gl: WebGL2RenderingContext,
  width: number,
  height: number,
  hdr: boolean,
  mipmapped = false,
) {
  const texture = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texImage2D(
    gl.TEXTURE_2D, 0, hdr ? gl.RGBA16F : gl.RGBA8, width, height, 0,
    gl.RGBA, hdr ? gl.HALF_FLOAT : gl.UNSIGNED_BYTE, null,
  );
  gl.texParameteri(
    gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, mipmapped ? gl.LINEAR_MIPMAP_LINEAR : gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.bindTexture(gl.TEXTURE_2D, null);
  return texture;
}

export function framebufferFor(gl: WebGL2RenderingContext, texture: WebGLTexture) {
  const framebuffer = gl.createFramebuffer()!;
  gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  return framebuffer;
}

export function ensureSceneTarget(
  gl: WebGL2RenderingContext,
  current: SceneTarget | null,
  width: number,
  height: number,
  hdr: boolean,
  supersample = false,
): SceneTarget {
  const wanted = supersample && width * height <= MAX_SUPERSAMPLED_PIXELS ? 2 : 1;
  const wantedGuard = 1 + 2 * Math.max(0, GUARD_BAND.margin);
  if (current && current.width === width && current.height === height
    && current.hdr === hdr && current.scale === wanted && current.guard === wantedGuard) {
    return current;
  }
  if (current) disposeSceneTarget(gl, current);

  const scale = wanted;
  const samples = Math.min(8, gl.getParameter(gl.MAX_SAMPLES) as number);
  const multisampled = samples > 1;
  const guard = 1 + 2 * Math.max(0, GUARD_BAND.margin);
  const renderWidth = Math.round(width * scale * guard);
  const renderHeight = Math.round(height * scale * guard);

  const resolved = linearTexture(gl, renderWidth, renderHeight, hdr);
  const capture = linearTexture(gl, renderWidth, renderHeight, hdr, true);
  const resolveFramebuffer = framebufferFor(gl, resolved);
  const captureFramebuffer = framebufferFor(gl, capture);

  const depth = gl.createRenderbuffer()!;
  gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
  if (multisampled) {
    gl.renderbufferStorageMultisample(
      gl.RENDERBUFFER, samples, gl.DEPTH_COMPONENT24, renderWidth, renderHeight);
  } else {
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT24, renderWidth, renderHeight);
    gl.bindFramebuffer(gl.FRAMEBUFFER, resolveFramebuffer);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  }
  gl.bindRenderbuffer(gl.RENDERBUFFER, null);

  let framebuffer: WebGLFramebuffer | null = null;
  let color: WebGLRenderbuffer | null = null;
  if (multisampled) {
    color = gl.createRenderbuffer()!;
    gl.bindRenderbuffer(gl.RENDERBUFFER, color);
    gl.renderbufferStorageMultisample(
      gl.RENDERBUFFER, samples, hdr ? gl.RGBA16F : gl.RGBA8, renderWidth, renderHeight);
    gl.bindRenderbuffer(gl.RENDERBUFFER, null);

    framebuffer = gl.createFramebuffer()!;
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, color);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  }

  const target: SceneTarget = {
    framebuffer,
    color,
    depth,
    resolveFramebuffer,
    resolved,
    captureFramebuffer,
    capture,
    width,
    height,
    renderWidth,
    renderHeight,
    scale,
    guard,
    samples: multisampled ? samples : 1,
    hdr,
  };

  for (const each of [framebuffer, resolveFramebuffer, captureFramebuffer]) {
    if (!each) continue;
    gl.bindFramebuffer(gl.FRAMEBUFFER, each);
    const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
    if (status !== gl.FRAMEBUFFER_COMPLETE) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      disposeSceneTarget(gl, target);
      throw new Error(`scene framebuffer is incomplete (0x${status.toString(16)})`);
    }
  }
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  return target;
}

export function disposeSceneTarget(gl: WebGL2RenderingContext, target: SceneTarget | null) {
  if (!target) return;
  if (target.framebuffer) gl.deleteFramebuffer(target.framebuffer);
  if (target.color) gl.deleteRenderbuffer(target.color);
  gl.deleteFramebuffer(target.resolveFramebuffer);
  gl.deleteFramebuffer(target.captureFramebuffer);
  gl.deleteRenderbuffer(target.depth);
  gl.deleteTexture(target.resolved);
  gl.deleteTexture(target.capture);
}

/** Point drawing at the scene target and clear it. */
export function beginScene(gl: WebGL2RenderingContext, target: SceneTarget) {
  gl.bindFramebuffer(gl.FRAMEBUFFER, target.framebuffer ?? target.resolveFramebuffer);
  gl.viewport(0, 0, target.renderWidth, target.renderHeight);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
}

function blitInto(gl: WebGL2RenderingContext, target: SceneTarget, destination: WebGLFramebuffer) {
  gl.bindFramebuffer(gl.READ_FRAMEBUFFER, target.framebuffer ?? target.resolveFramebuffer);
  gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, destination);
  // A multisampled read resolves only into a rectangle of its own size, and
  // only with NEAREST; both are what a resolve means rather than a choice.
  gl.blitFramebuffer(
    0, 0, target.renderWidth, target.renderHeight,
    0, 0, target.renderWidth, target.renderHeight,
    gl.COLOR_BUFFER_BIT, gl.NEAREST,
  );
  gl.bindFramebuffer(gl.READ_FRAMEBUFFER, null);
  gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null);
  gl.bindFramebuffer(gl.FRAMEBUFFER, target.framebuffer ?? target.resolveFramebuffer);
}

/**
 * Take the copy transmission refracts, and go on drawing.
 *
 * Called with the opaque half down and nothing blended over it yet, which is
 * the moment the frame holds everything a transmissive surface is allowed to
 * see. The mip chain is the blur a rough one reads through.
 */
export function captureOpaqueHalf(gl: WebGL2RenderingContext, target: SceneTarget) {
  blitInto(gl, target, target.captureFramebuffer);
  gl.bindTexture(gl.TEXTURE_2D, target.capture);
  gl.generateMipmap(gl.TEXTURE_2D);
  gl.bindTexture(gl.TEXTURE_2D, null);
}

/** Resolve the finished frame, ready for the output pass to read. */
export function resolveScene(gl: WebGL2RenderingContext, target: SceneTarget) {
  if (target.framebuffer) blitInto(gl, target, target.resolveFramebuffer);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
}
