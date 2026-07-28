/**
 * Where a transmissive volume ends, and which way its far wall faces.
 *
 * `KHR_materials_volume` states a thickness, and a thickness is an author's
 * number: it says how deep the glass is meant to be, not how deep it is at
 * this pixel. A bulb is thin at the neck and deep through the middle; a lens
 * is nothing else. So the far wall is drawn - back faces only, once, before
 * the frame - and what comes out is the distance light actually crosses and
 * the normal it meets on the way out.
 *
 * That second part is the one screen-space refraction usually skips. A ray is
 * bent going in and then just walked, as though the glass had no other side;
 * the exit is where the whole prism-ness of a prism comes from. With the far
 * wall in hand it can be bent again, and past the critical angle it can fail
 * to leave at all.
 *
 * The texture holds the world normal and the view depth of the far wall, which
 * is everything the surface shader needs and nothing it does not.
 */

import { framebufferFor, linearTexture } from './scene-target.ts';

export interface BackFaceDepth {
  framebuffer: WebGLFramebuffer;
  texture: WebGLTexture;
  depth: WebGLRenderbuffer;
  width: number;
  height: number;
}

export function ensureBackFaceDepth(
  gl: WebGL2RenderingContext,
  current: BackFaceDepth | null,
  width: number,
  height: number,
): BackFaceDepth {
  if (current && current.width === width && current.height === height) return current;
  if (current) disposeBackFaceDepth(gl, current);

  // Half floats because the payload is a normal and a distance in world units,
  // neither of which fits a byte.
  const texture = linearTexture(gl, width, height, true);
  const framebuffer = framebufferFor(gl, texture);
  const depth = gl.createRenderbuffer()!;
  gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
  gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT24, width, height);
  gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
  gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
  gl.bindRenderbuffer(gl.RENDERBUFFER, null);
  const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  if (status !== gl.FRAMEBUFFER_COMPLETE) {
    gl.deleteFramebuffer(framebuffer);
    gl.deleteRenderbuffer(depth);
    gl.deleteTexture(texture);
    throw new Error(`back-face framebuffer is incomplete (0x${status.toString(16)})`);
  }
  return { framebuffer, texture, depth, width, height };
}

export function disposeBackFaceDepth(gl: WebGL2RenderingContext, target: BackFaceDepth | null) {
  if (!target) return;
  gl.deleteFramebuffer(target.framebuffer);
  gl.deleteRenderbuffer(target.depth);
  gl.deleteTexture(target.texture);
}

/**
 * Point drawing at it, keeping the far wall rather than the near one.
 *
 * Front faces are culled and the depth test is reversed, so what survives is
 * the *last* back face along the ray: a bulb inside a shade leaves the bulb's
 * far side, not the shade's.
 */
export function beginBackFaceDepth(gl: WebGL2RenderingContext, target: BackFaceDepth) {
  gl.bindFramebuffer(gl.FRAMEBUFFER, target.framebuffer);
  gl.viewport(0, 0, target.width, target.height);
  // A zero normal is how the shader knows no wall was found here.
  gl.clearColor(0, 0, 0, 0);
  gl.clearDepth(0);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
  // Stated rather than inherited. Whatever drew last owns this state, and a
  // blended surface leaves the depth mask off - with it off nothing is
  // written, the reversed test never updates, and which wall survives comes
  // down to draw order. That is stable within a frame and different from one
  // camera angle to the next, which is what it looks like.
  gl.enable(gl.DEPTH_TEST);
  gl.depthMask(true);
  gl.disable(gl.BLEND);
  gl.depthFunc(gl.GREATER);
  gl.enable(gl.CULL_FACE);
  gl.cullFace(gl.FRONT);
}

export function endBackFaceDepth(gl: WebGL2RenderingContext) {
  gl.depthFunc(gl.LESS);
  gl.clearDepth(1);
  gl.clearColor(0, 0, 0, 0);
  gl.cullFace(gl.BACK);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
}
