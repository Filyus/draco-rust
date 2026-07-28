/**
 * What a transmissive surface sees, as a direction rather than as a place on
 * the screen.
 *
 * Screen-space refraction reads the frame, and the frame is what the camera
 * happened to be pointed at. A ray bent even slightly leaves it, and then there
 * is nothing to read: clamping repeats the border texel into a streak, and
 * anything substituted for it - the sky, the frame's own average - is an
 * invention that shows as a band the moment the glass is looked at from an
 * angle. None of that is a defect of the code; it is the method having no data.
 *
 * A cube rendered from inside the object has the data. It is indexed by
 * direction, so there is no edge to fall off, no dependence on where the object
 * sits on screen, and nothing to stand in for. It also makes the exit boundary
 * worth having again: bending a ray on the way out changes its direction and
 * not the distance it then travels, and a distance is exactly what screen space
 * could not supply.
 *
 * What it costs is six faces of the opaque scene per probe. What it cannot do
 * is be right from more than one point: the cube is correct at its own centre
 * and parallaxes elsewhere, which is what the bounds correction is for - the
 * ray is intersected with the scene's own box and looked up as if from there.
 */

import { mat4, vec3 } from '../math.ts';
import type { Mat4, Vec3 } from '../math.ts';

/**
 * The knobs, in one place.
 *
 * Resolution is the face size, and it is the one that matters: a rough surface
 * reads the mip chain and is happy with little, while mirror-smooth glass over
 * a fine backdrop shows every texel. Six faces of half floats with a mip chain
 * come to about eight megabytes at 256, thirty-three at 512.
 */
export const REFRACTION_PROBE = {
  /** Face size of the cube the scene is rendered into. */
  resolution: 512,
  /** Field of view per face. Ninety degrees is what tiles a cube. */
  fieldOfView: Math.PI / 2,
  /** Near plane. Closer than this the probe's own object would be in the way. */
  near: 0.01,
};

/** The six faces, as the direction each looks and the up it keeps. */
const FACES: { target: Vec3; up: Vec3 }[] = [
  { target: [1, 0, 0], up: [0, -1, 0] },
  { target: [-1, 0, 0], up: [0, -1, 0] },
  { target: [0, 1, 0], up: [0, 0, 1] },
  { target: [0, -1, 0], up: [0, 0, -1] },
  { target: [0, 0, 1], up: [0, -1, 0] },
  { target: [0, 0, -1], up: [0, -1, 0] },
].map(({ target, up }) => ({
  target: vec3.set(vec3.create(), target[0], target[1], target[2]),
  up: vec3.set(vec3.create(), up[0], up[1], up[2]),
}));

export interface RefractionProbe {
  cubemap: WebGLTexture;
  framebuffer: WebGLFramebuffer;
  depth: WebGLRenderbuffer;
  size: number;
  levels: number;
  hdr: boolean;
  /** Where the cube was rendered from, in world space. */
  center: Vec3;
  /** The scene's own box, which the parallax correction intersects. */
  boundsMin: Vec3;
  boundsMax: Vec3;
}

export function ensureRefractionProbe(
  gl: WebGL2RenderingContext,
  current: RefractionProbe | null,
  hdr: boolean,
): RefractionProbe {
  const size = Math.max(16, REFRACTION_PROBE.resolution | 0);
  if (current && current.size === size && current.hdr === hdr) return current;
  if (current) disposeRefractionProbe(gl, current);

  const levels = Math.floor(Math.log2(size)) + 1;
  const cubemap = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_CUBE_MAP, cubemap);
  gl.texStorage2D(
    gl.TEXTURE_CUBE_MAP, levels, hdr ? gl.RGBA16F : gl.RGBA8, size, size);
  gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
  gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.bindTexture(gl.TEXTURE_CUBE_MAP, null);

  const depth = gl.createRenderbuffer()!;
  gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
  gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT24, size, size);
  gl.bindRenderbuffer(gl.RENDERBUFFER, null);

  const framebuffer = gl.createFramebuffer()!;
  gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
  gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);

  return {
    cubemap,
    framebuffer,
    depth,
    size,
    levels,
    hdr,
    center: vec3.create(),
    boundsMin: vec3.create(),
    boundsMax: vec3.create(),
  };
}

export function disposeRefractionProbe(
  gl: WebGL2RenderingContext,
  probe: RefractionProbe | null,
) {
  if (!probe) return;
  gl.deleteFramebuffer(probe.framebuffer);
  gl.deleteRenderbuffer(probe.depth);
  gl.deleteTexture(probe.cubemap);
}

/**
 * Point drawing at one face, and hand back the view matrix it wants.
 *
 * The far plane is taken from the scene's own size rather than the camera's:
 * the probe is not the camera, and what it needs to hold is the scene.
 */
export function beginProbeFace(
  gl: WebGL2RenderingContext,
  probe: RefractionProbe,
  face: number,
  projection: Mat4,
  view: Mat4,
  radius: number,
) {
  gl.bindFramebuffer(gl.FRAMEBUFFER, probe.framebuffer);
  gl.framebufferTexture2D(
    gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0,
    gl.TEXTURE_CUBE_MAP_POSITIVE_X + face, probe.cubemap, 0);
  gl.viewport(0, 0, probe.size, probe.size);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);

  mat4.perspective(
    projection, REFRACTION_PROBE.fieldOfView, 1,
    REFRACTION_PROBE.near, Math.max(radius * 4, 1));
  const at = vec3.create();
  vec3.add(at, probe.center, FACES[face].target);
  mat4.lookAt(view, probe.center, at, FACES[face].up);
}

export const PROBE_FACE_COUNT = FACES.length;

/** Fill the mip chain a rough surface reads, and hand drawing back. */
export function finishProbe(gl: WebGL2RenderingContext, probe: RefractionProbe) {
  gl.bindTexture(gl.TEXTURE_CUBE_MAP, probe.cubemap);
  gl.generateMipmap(gl.TEXTURE_CUBE_MAP);
  gl.bindTexture(gl.TEXTURE_CUBE_MAP, null);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
}
