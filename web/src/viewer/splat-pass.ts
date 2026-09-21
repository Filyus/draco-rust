/**
 * Drawing a Gaussian splat cloud, as its own pass beside the mesh pipeline.
 *
 * A splat is not a primitive this viewer's pipeline has a shape for. It has no
 * indices, no material, and fourteen numbers a point rather than a vertex
 * layout, and it has to be drawn back to front with blending. Rather than
 * bending `uploadPrimitive` around that, this is a pass of its own: its own
 * program, its own buffers, its own sort, drawn after the surfaces. Nothing
 * here can break a mesh.
 *
 * What it draws, per splat: the 3D gaussian `R S Sᵀ Rᵀ` projected to a screen
 * ellipse, as one instanced quad, with the gaussian falloff in the fragment
 * shader. The colour is the degree-0 harmonic and does not change with the
 * view — the higher bands are not carried yet, so a shiny surface looks flat.
 *
 * Turning the camera re-sorts, and panning does not, which is what the two
 * mouse buttons feeling different was: the order depends on the view direction
 * alone. The first version of this file made that asymmetry expensive -- a
 * comparator sort over a boxed array, then fifty-six bytes a splat re-uploaded
 * in the new order, 344 ms for 742k splats. Both halves are gone:
 *
 *   - the sort is a counting sort over depth quantized to sixteen bits, which
 *     is linear in the splats and touches no boxed values;
 *   - the splats themselves never move. They live in a texture, and what the
 *     sort produces is an index per instance -- four bytes a splat uploaded
 *     instead of fifty-six.
 */

import type { SplatCloud } from '../splat.ts';

/** The degree-0 harmonic to a colour, which is the renderer's conversion. */
const SH_C0 = 0.2820948;

/** Past this much camera rotation the order is stale enough to redo. */
const RESORT_COSINE = 0.999;

const VERTEX_SHADER = `#version 300 es
precision highp float;

// The quad, in its own square. The gaussian is evaluated in these coordinates.
layout(location = 0) in vec2 aCorner;
// Which splat this instance draws. The splat itself is in the texture, so a
// re-sort moves this and nothing else.
layout(location = 1) in uint aIndex;

// Four texels a splat: centre and alpha, the scale, the rotation, the colour.
uniform highp sampler2D uSplats;
uniform int uTextureWidth;

uniform mat4 uView;
uniform mat4 uProjection;
uniform vec2 uViewport;
// How many standard deviations the quad reaches. Past three the gaussian is
// under 1.2% and the quad is mostly wasted fill.
uniform float uExtent;
// The largest ellipse this will draw, as a fraction of the viewport's height.
uniform float uMaxRadius;

out vec2 vLocal;
out vec4 vColour;

mat3 rotationOf(vec4 q) {
  float w = q.x, x = q.y, y = q.z, z = q.w;
  return mat3(
    1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + w * z),       2.0 * (x * z - w * y),
    2.0 * (x * y - w * z),       1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + w * x),
    2.0 * (x * z + w * y),       2.0 * (y * z - w * x),       1.0 - 2.0 * (x * x + y * y)
  );
}

vec4 splatTexel(uint splat, int which) {
  int texel = int(splat) * 4 + which;
  return texelFetch(uSplats, ivec2(texel % uTextureWidth, texel / uTextureWidth), 0);
}

void main() {
  vec4 centreAlpha = splatTexel(aIndex, 0);
  vec3 aCenter = centreAlpha.xyz;
  float aAlpha = centreAlpha.w;
  vec3 aScale = splatTexel(aIndex, 1).xyz;
  vec4 aRotation = splatTexel(aIndex, 2);
  vec3 aDc = splatTexel(aIndex, 3).xyz;

  vec4 viewCenter = uView * vec4(aCenter, 1.0);
  vec4 clip = uProjection * viewCenter;
  if (clip.w <= 0.0) {
    // Behind the eye: collapse the quad rather than let it wrap around.
    gl_Position = vec4(0.0, 0.0, 2.0, 1.0);
    vLocal = vec2(0.0);
    vColour = vec4(0.0);
    return;
  }

  // The 3D covariance, and the same in view space.
  mat3 rotation = rotationOf(aRotation);
  mat3 scaled = mat3(
    rotation[0] * aScale.x,
    rotation[1] * aScale.y,
    rotation[2] * aScale.z
  );
  mat3 covariance = scaled * transpose(scaled);
  mat3 viewRotation = mat3(uView);
  mat3 viewCovariance = viewRotation * covariance * transpose(viewRotation);

  // The perspective divide is not linear, so the projection is taken as its
  // Jacobian at the splat's own depth. Anything further from the centre than
  // the quad reaches is wrong by more than it is worth correcting.
  float fx = uProjection[0][0] * 0.5 * uViewport.x;
  float fy = uProjection[1][1] * 0.5 * uViewport.y;
  float z = -viewCenter.z;
  float invZ = 1.0 / z;
  mat3x2 jacobian = mat3x2(
    fx * invZ, 0.0,
    0.0, fy * invZ,
    -fx * viewCenter.x * invZ * invZ, -fy * viewCenter.y * invZ * invZ
  );
  mat2 screen = jacobian * viewCovariance * transpose(jacobian);
  // A splat thinner than a pixel has no shape left to project; the dilation
  // keeps it a dot instead of an invisible sliver.
  screen[0][0] += 0.3;
  screen[1][1] += 0.3;

  // The ellipse's own axes, from the 2x2's eigenvalues.
  float mid = 0.5 * (screen[0][0] + screen[1][1]);
  float discriminant = sqrt(max(0.0, mid * mid - determinant(screen)));
  float major = mid + discriminant;
  float minor = max(mid - discriminant, 0.0);
  vec2 majorAxis = normalize(vec2(screen[0][1], major - screen[0][0]));
  if (screen[0][1] == 0.0) majorAxis = vec2(1.0, 0.0);
  vec2 minorAxis = vec2(majorAxis.y, -majorAxis.x);

  // A splat close enough to the eye projects to an ellipse spanning much of
  // the frame, and drawing it is worse than dropping it: the projection above
  // is the Jacobian at the splat's centre, a first-order approximation that
  // holds only while the footprint is small. A metre-wide gaussian half a
  // metre from the lens is not the shape this draws, and forty thousand of
  // them -- which is what a trained scene leaves along the camera path --
  // arrive last in the order and smear the frame to a flat grey.
  //
  // So they are dropped rather than drawn wrong. The limit is generous: a
  // splat reaching a quarter of the frame's height is already far outside
  // where the approximation is worth anything.
  float radius = sqrt(major) * uExtent;
  if (radius > uMaxRadius * uViewport.y) {
    gl_Position = vec4(0.0, 0.0, 2.0, 1.0);
    vLocal = vec2(0.0);
    vColour = vec4(0.0);
    return;
  }

  vec2 offset = aCorner.x * majorAxis * sqrt(major) * uExtent
              + aCorner.y * minorAxis * sqrt(minor) * uExtent;

  vLocal = aCorner * uExtent;
  vColour = vec4(SH_C0_CONST * aDc + 0.5, aAlpha);
  gl_Position = vec4(
    clip.xy / clip.w + offset / uViewport * 2.0,
    clip.z / clip.w,
    1.0
  );
}
`.replace('SH_C0_CONST', String(SH_C0));

const FRAGMENT_SHADER = `#version 300 es
precision highp float;

in vec2 vLocal;
in vec4 vColour;
out vec4 outColour;

void main() {
  // The gaussian, in the quad's own coordinates. vLocal is already in
  // standard deviations, so this is exp(-r*r/2) and nothing else.
  float power = -0.5 * dot(vLocal, vLocal);
  float weight = exp(power) * vColour.a;
  if (weight < 1.0 / 255.0) discard;

  // A splat's colour is what a splat renderer would put on the screen, which
  // is display-referred; this frame is linear and tone mapped on the way out.
  // Writing the one into the other without converting is what turns a lit
  // scene into a white one.
  //
  // The clamp comes first and is not a detail: a degree-0 harmonic reaches
  // outside [0, 1] for about a third of the values in a real scene, because
  // the higher bands are expected to bring it back. Nothing here carries those
  // bands yet, so the colour has to be clamped rather than allowed to shine.
  vec3 display = clamp(vColour.rgb, 0.0, 1.0);
  vec3 linear = pow(display, vec3(2.2));

  // Premultiplied: the blend adds, so the colour arrives already weighted and
  // the destination keeps what is left.
  outColour = vec4(linear * weight, weight);
}
`;

export interface SplatResources {
  program: WebGLProgram;
  vao: WebGLVertexArrayObject;
  corners: WebGLBuffer;
  /** One unsigned index an instance, in draw order. */
  instances: WebGLBuffer;
  /** Four RGBA32F texels a splat; this never changes after upload. */
  texture: WebGLTexture;
  textureHeight: number;
  cloud: SplatCloud;
  scratch: SortScratch;
  /** The view direction the order was made for. */
  sortedFor: [number, number, number];
  uniforms: {
    view: WebGLUniformLocation | null;
    projection: WebGLUniformLocation | null;
    viewport: WebGLUniformLocation | null;
    extent: WebGLUniformLocation | null;
    maxRadius: WebGLUniformLocation | null;
    splats: WebGLUniformLocation | null;
    textureWidth: WebGLUniformLocation | null;
  };
}

/** Floats a splat in the texture: four RGBA texels. */
const STRIDE = 16;

/** Texels across the splat texture. Four a splat, so 512 splats a row. */
const TEXTURE_WIDTH = 2048;

function compile(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type)!;
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(shader);
    gl.deleteShader(shader);
    throw new Error(`splat shader: ${log}`);
  }
  return shader;
}

function link(gl: WebGL2RenderingContext): WebGLProgram {
  const program = gl.createProgram()!;
  const vertex = compile(gl, gl.VERTEX_SHADER, VERTEX_SHADER);
  const fragment = compile(gl, gl.FRAGMENT_SHADER, FRAGMENT_SHADER);
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  gl.deleteShader(vertex);
  gl.deleteShader(fragment);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(program);
    gl.deleteProgram(program);
    throw new Error(`splat program: ${log}`);
  }
  return program;
}

/**
 * The cloud as the texture holds it: four RGBA texels a splat.
 *
 * Centre and alpha share a texel because both are wanted first and a vec4 is
 * one fetch. The two floats left spare are the price of a layout the shader
 * indexes by shifting rather than by multiplying.
 */
export function packSplats(cloud: SplatCloud): Float32Array {
  const packed = new Float32Array(cloud.count * STRIDE);
  for (let splat = 0; splat < cloud.count; splat += 1) {
    const at = splat * STRIDE;
    packed[at] = cloud.positions[splat * 3];
    packed[at + 1] = cloud.positions[splat * 3 + 1];
    packed[at + 2] = cloud.positions[splat * 3 + 2];
    packed[at + 3] = cloud.alphas[splat];
    packed[at + 4] = cloud.scales[splat * 3];
    packed[at + 5] = cloud.scales[splat * 3 + 1];
    packed[at + 6] = cloud.scales[splat * 3 + 2];
    packed[at + 8] = cloud.rotations[splat * 4];
    packed[at + 9] = cloud.rotations[splat * 4 + 1];
    packed[at + 10] = cloud.rotations[splat * 4 + 2];
    packed[at + 11] = cloud.rotations[splat * 4 + 3];
    packed[at + 12] = cloud.dc[splat * 3];
    packed[at + 13] = cloud.dc[splat * 3 + 1];
    packed[at + 14] = cloud.dc[splat * 3 + 2];
  }
  return packed;
}

/**
 * Splat indices, furthest first along `direction`.
 *
 * Furthest first because the blend is `over`: what is drawn later sits in
 * front. Sorting on the view direction rather than on distance to the eye is
 * what keeps the order stable while the camera dollies, and wrong only for a
 * splat behind the camera, which is not drawn anyway.
 *
 * A counting sort, not a comparator one. The depths are quantized to sixteen
 * bits across the range they actually span, which is finer than the difference
 * between two splats that matters and costs one pass each to bucket, total and
 * place. The comparator version this replaced took 243 ms on 742k splats
 * against 12 for this, and that difference was the whole of why turning the
 * camera stuttered where panning did not.
 */
export function sortOrder(
  cloud: SplatCloud,
  direction: readonly [number, number, number],
  scratch?: SortScratch,
): Uint32Array {
  const count = cloud.count;
  const work = scratch && scratch.depths.length === count ? scratch : makeSortScratch(count);
  const { depths, counts, order } = work;

  const [dx, dy, dz] = direction;
  let low = Infinity;
  let high = -Infinity;
  for (let splat = 0; splat < count; splat += 1) {
    const depth = cloud.positions[splat * 3] * dx
      + cloud.positions[splat * 3 + 1] * dy
      + cloud.positions[splat * 3 + 2] * dz;
    depths[splat] = depth;
    if (depth < low) low = depth;
    if (depth > high) high = depth;
  }

  const buckets = counts.length;
  // A scene with every splat at one depth has no order to find; any is right.
  const scale = high > low ? (buckets - 1) / (high - low) : 0;
  counts.fill(0);
  const bucketOf = (depth: number) => (buckets - 1) - ((depth - low) * scale | 0);
  for (let splat = 0; splat < count; splat += 1) counts[bucketOf(depths[splat])] += 1;
  let running = 0;
  for (let bucket = 0; bucket < buckets; bucket += 1) {
    const here = counts[bucket];
    counts[bucket] = running;
    running += here;
  }
  for (let splat = 0; splat < count; splat += 1) {
    const bucket = bucketOf(depths[splat]);
    order[counts[bucket]] = splat;
    counts[bucket] += 1;
  }
  return order;
}

/** The three arrays the sort reuses, so that turning the camera allocates nothing. */
export interface SortScratch {
  depths: Float32Array;
  counts: Uint32Array;
  order: Uint32Array;
}

export function makeSortScratch(count: number): SortScratch {
  return {
    depths: new Float32Array(count),
    // Sixteen bits of depth: finer than a splat is wide at any framing this
    // draws, and a histogram that still fits a cache.
    counts: new Uint32Array(1 << 16),
    order: new Uint32Array(count),
  };
}

export function uploadSplats(gl: WebGL2RenderingContext, cloud: SplatCloud): SplatResources {
  const program = link(gl);
  const vao = gl.createVertexArray()!;
  gl.bindVertexArray(vao);

  // One quad, as a triangle strip in its own square.
  const corners = gl.createBuffer()!;
  gl.bindBuffer(gl.ARRAY_BUFFER, corners);
  gl.bufferData(
    gl.ARRAY_BUFFER,
    new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]),
    gl.STATIC_DRAW,
  );
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);

  // The only per-instance attribute: which splat to draw. Integer, so the
  // pointer is the `I` form -- the float one would round past 2^24.
  const instances = gl.createBuffer()!;
  gl.bindBuffer(gl.ARRAY_BUFFER, instances);
  gl.bufferData(gl.ARRAY_BUFFER, cloud.count * 4, gl.DYNAMIC_DRAW);
  gl.enableVertexAttribArray(1);
  gl.vertexAttribIPointer(1, 1, gl.UNSIGNED_INT, 0, 0);
  gl.vertexAttribDivisor(1, 1);
  gl.bindVertexArray(null);

  // The splats themselves, which never move again.
  const packed = packSplats(cloud);
  const texels = cloud.count * 4;
  const height = Math.max(1, Math.ceil(texels / TEXTURE_WIDTH));
  const padded = new Float32Array(TEXTURE_WIDTH * height * 4);
  padded.set(packed);
  const texture = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.texImage2D(
    gl.TEXTURE_2D, 0, gl.RGBA32F, TEXTURE_WIDTH, height, 0, gl.RGBA, gl.FLOAT, padded,
  );
  gl.bindTexture(gl.TEXTURE_2D, null);

  return {
    program,
    vao,
    corners,
    instances,
    texture,
    textureHeight: height,
    cloud,
    scratch: makeSortScratch(cloud.count),
    // No direction yet, so the first frame always sorts.
    sortedFor: [0, 0, 0],
    uniforms: {
      view: gl.getUniformLocation(program, 'uView'),
      projection: gl.getUniformLocation(program, 'uProjection'),
      viewport: gl.getUniformLocation(program, 'uViewport'),
      extent: gl.getUniformLocation(program, 'uExtent'),
      maxRadius: gl.getUniformLocation(program, 'uMaxRadius'),
      splats: gl.getUniformLocation(program, 'uSplats'),
      textureWidth: gl.getUniformLocation(program, 'uTextureWidth'),
    },
  };
}

/**
 * Re-sort when the camera has turned enough to matter.
 *
 * The threshold is on the direction rather than on a frame count: a still
 * camera never re-sorts, and one being dragged re-sorts as often as it must.
 * Only the index buffer moves, four bytes a splat.
 */
export function ensureOrder(
  gl: WebGL2RenderingContext,
  splats: SplatResources,
  direction: readonly [number, number, number],
): boolean {
  const [px, py, pz] = splats.sortedFor;
  const [dx, dy, dz] = direction;
  if (px * dx + py * dy + pz * dz > RESORT_COSINE) return false;

  const order = sortOrder(splats.cloud, direction, splats.scratch);
  splats.sortedFor = [dx, dy, dz];
  gl.bindBuffer(gl.ARRAY_BUFFER, splats.instances);
  gl.bufferSubData(gl.ARRAY_BUFFER, 0, order);
  return true;
}

export function drawSplats(
  gl: WebGL2RenderingContext,
  splats: SplatResources,
  view: Float32Array,
  projection: Float32Array,
  viewportWidth: number,
  viewportHeight: number,
  extent = 3,
  maxRadius = 0.25,
) {
  if (splats.cloud.count === 0) return;
  gl.useProgram(splats.program);
  gl.bindVertexArray(splats.vao);
  gl.activeTexture(gl.TEXTURE0);
  gl.bindTexture(gl.TEXTURE_2D, splats.texture);
  gl.uniform1i(splats.uniforms.splats, 0);
  gl.uniform1i(splats.uniforms.textureWidth, TEXTURE_WIDTH);
  gl.uniformMatrix4fv(splats.uniforms.view, false, view);
  gl.uniformMatrix4fv(splats.uniforms.projection, false, projection);
  gl.uniform2f(splats.uniforms.viewport, viewportWidth, viewportHeight);
  gl.uniform1f(splats.uniforms.extent, extent);
  gl.uniform1f(splats.uniforms.maxRadius, maxRadius);

  // Blended back to front, and the depth buffer is read but not written: one
  // splat does not hide the next, they accumulate.
  gl.enable(gl.BLEND);
  gl.blendFuncSeparate(gl.ONE, gl.ONE_MINUS_SRC_ALPHA, gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
  gl.depthMask(false);
  gl.disable(gl.CULL_FACE);
  gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, splats.cloud.count);
  gl.depthMask(true);
  gl.disable(gl.BLEND);
  gl.bindVertexArray(null);
}

export function disposeSplats(gl: WebGL2RenderingContext, splats: SplatResources) {
  gl.deleteVertexArray(splats.vao);
  gl.deleteBuffer(splats.corners);
  gl.deleteBuffer(splats.instances);
  gl.deleteTexture(splats.texture);
  gl.deleteProgram(splats.program);
}
