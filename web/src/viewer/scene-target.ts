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
 * Transmission does not read it. What a refracted ray needs is indexed by
 * direction rather than by where the camera happened to be pointed, and that
 * lives in the probe.
 */

/**
 * Above this many texels supersampling is refused whatever was asked for: four
 * times the texels of a large frame is hundreds of megabytes of attachments.
 */
const MAX_SUPERSAMPLED_PIXELS = 1_200_000;

export interface SceneTarget {
  /** Multisampled draw target; null when the driver offers one sample. */
  framebuffer: WebGLFramebuffer | null;
  color: WebGLRenderbuffer | null;
  depth: WebGLRenderbuffer;
  /** Where the samples resolve to, and what the output pass reads. */
  resolveFramebuffer: WebGLFramebuffer;
  resolved: WebGLTexture;
  /** The canvas size the frame is shown at. */
  width: number;
  height: number;
  /** The size it is drawn at: `scale` times larger. */
  renderWidth: number;
  renderHeight: number;
  scale: number;
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
  if (current && current.width === width && current.height === height
    && current.hdr === hdr && current.scale === wanted) {
    return current;
  }
  if (current) disposeSceneTarget(gl, current);

  const scale = wanted;
  const samples = Math.min(8, gl.getParameter(gl.MAX_SAMPLES) as number);
  const multisampled = samples > 1;
  const renderWidth = width * scale;
  const renderHeight = height * scale;

  const resolved = linearTexture(gl, renderWidth, renderHeight, hdr);
  const resolveFramebuffer = framebufferFor(gl, resolved);

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
    width,
    height,
    renderWidth,
    renderHeight,
    scale,
    samples: multisampled ? samples : 1,
    hdr,
  };

  for (const each of [framebuffer, resolveFramebuffer]) {
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
  gl.deleteRenderbuffer(target.depth);
  gl.deleteTexture(target.resolved);
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

/** Resolve the finished frame, ready for the output pass to read. */
export function resolveScene(gl: WebGL2RenderingContext, target: SceneTarget) {
  if (target.framebuffer) blitInto(gl, target, target.resolveFramebuffer);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
}
