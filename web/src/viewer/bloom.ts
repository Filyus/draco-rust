/**
 * The glare a bright thing leaves around itself.
 *
 * Not decoration, and not a way of hiding aliasing: no optical system forms a
 * point image of a point source. A lens scatters at every surface and edge, the
 * eye scatters in the cornea and the lens both, and what reaches the sensor is
 * the scene convolved with a spread function whose tails run far and wide. That
 * is why a filament twenty-five times brighter than white is *seen* as a glow
 * rather than as a hard-edged wire, and why leaving it out is the less
 * physical choice, not the more careful one.
 *
 * The shape is the usual pyramid: halve the frame a few times, each level
 * filtered as it goes down, then walk back up adding each level into the one
 * above with a tent filter. The sum of progressively wider blurs approximates
 * the long-tailed spread function far better than any single Gaussian, and it
 * costs a handful of passes over rapidly shrinking buffers.
 *
 * There is no threshold anywhere in it. Thresholded bloom - take what is over
 * some brightness, blur that, add it back - invents energy at the cut and is
 * the reason the effect has a reputation. Here the whole frame is spread, and
 * the result is mixed rather than added, so what glare takes from one place it
 * gives to another.
 */

import { framebufferFor, linearTexture } from './scene-target.ts';

/** How far down the pyramid goes; below this the levels stop being useful. */
const MIN_LEVEL_SIZE = 8;
const MAX_LEVELS = 6;

export interface BloomLevel {
  texture: WebGLTexture;
  framebuffer: WebGLFramebuffer;
  width: number;
  height: number;
}

export interface BloomChain {
  levels: BloomLevel[];
  width: number;
  height: number;
  hdr: boolean;
}

export function ensureBloomChain(
  gl: WebGL2RenderingContext,
  current: BloomChain | null,
  width: number,
  height: number,
  hdr: boolean,
): BloomChain {
  if (current && current.width === width && current.height === height && current.hdr === hdr) {
    return current;
  }
  if (current) disposeBloomChain(gl, current);

  const levels: BloomLevel[] = [];
  let levelWidth = width;
  let levelHeight = height;
  while (levels.length < MAX_LEVELS) {
    levelWidth = Math.max(1, levelWidth >> 1);
    levelHeight = Math.max(1, levelHeight >> 1);
    if (levelWidth < MIN_LEVEL_SIZE || levelHeight < MIN_LEVEL_SIZE) break;
    const texture = linearTexture(gl, levelWidth, levelHeight, hdr);
    levels.push({
      texture, framebuffer: framebufferFor(gl, texture), width: levelWidth, height: levelHeight,
    });
  }
  return { levels, width, height, hdr };
}

export function disposeBloomChain(gl: WebGL2RenderingContext, chain: BloomChain | null) {
  if (!chain) return;
  for (const level of chain.levels) {
    gl.deleteFramebuffer(level.framebuffer);
    gl.deleteTexture(level.texture);
  }
  chain.levels.length = 0;
}
