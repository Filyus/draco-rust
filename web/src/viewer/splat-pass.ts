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
 * Sorting re-uploads the instance buffer, which is the simple half of the
 * trade: it is fourteen floats a splat moved whenever the camera turns enough,
 * and it stops being the right answer somewhere in the hundreds of thousands.
 * The scale-up is splat data in textures with a sorted index buffer, four
 * bytes a splat instead of fifty-six.
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
// One splat: where it is, how big, which way, how solid, what colour.
layout(location = 1) in vec3 aCenter;
layout(location = 2) in vec3 aScale;
layout(location = 3) in vec4 aRotation;   // (w, x, y, z), as the file orders it
layout(location = 4) in float aAlpha;
layout(location = 5) in vec3 aDc;

uniform mat4 uView;
uniform mat4 uProjection;
uniform vec2 uViewport;
// How many standard deviations the quad reaches. Past three the gaussian is
// under 1.2% and the quad is mostly wasted fill.
uniform float uExtent;

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

void main() {
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
  instances: WebGLBuffer;
  /** The interleaved per-splat data, in draw order. */
  packed: Float32Array;
  cloud: SplatCloud;
  /** Splat indices, ordered back to front for the direction below. */
  order: Uint32Array;
  /** The view direction the order was made for. */
  sortedFor: [number, number, number];
  uniforms: {
    view: WebGLUniformLocation | null;
    projection: WebGLUniformLocation | null;
    viewport: WebGLUniformLocation | null;
    extent: WebGLUniformLocation | null;
  };
}

/** Floats per splat in the instance buffer: centre, scale, rotation, alpha, dc. */
const STRIDE = 3 + 3 + 4 + 1 + 3;

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

/** The cloud as one interleaved array, which is what the sort reorders. */
export function packSplats(cloud: SplatCloud): Float32Array {
  const packed = new Float32Array(cloud.count * STRIDE);
  for (let splat = 0; splat < cloud.count; splat += 1) {
    const at = splat * STRIDE;
    packed[at] = cloud.positions[splat * 3];
    packed[at + 1] = cloud.positions[splat * 3 + 1];
    packed[at + 2] = cloud.positions[splat * 3 + 2];
    packed[at + 3] = cloud.scales[splat * 3];
    packed[at + 4] = cloud.scales[splat * 3 + 1];
    packed[at + 5] = cloud.scales[splat * 3 + 2];
    packed[at + 6] = cloud.rotations[splat * 4];
    packed[at + 7] = cloud.rotations[splat * 4 + 1];
    packed[at + 8] = cloud.rotations[splat * 4 + 2];
    packed[at + 9] = cloud.rotations[splat * 4 + 3];
    packed[at + 10] = cloud.alphas[splat];
    packed[at + 11] = cloud.dc[splat * 3];
    packed[at + 12] = cloud.dc[splat * 3 + 1];
    packed[at + 13] = cloud.dc[splat * 3 + 2];
  }
  return packed;
}

/**
 * Splat indices, furthest first, along `direction`.
 *
 * Furthest first because the blend below is `over`: what is drawn later sits
 * in front. Sorting on the view direction rather than on distance to the eye
 * is what makes the order stable while the camera dollies, and wrong only for
 * a splat that is behind the camera, which is not drawn anyway.
 */
export function sortOrder(
  cloud: SplatCloud,
  direction: readonly [number, number, number],
  into?: Uint32Array,
): Uint32Array {
  const count = cloud.count;
  const depths = new Float32Array(count);
  const [dx, dy, dz] = direction;
  for (let splat = 0; splat < count; splat += 1) {
    depths[splat] = cloud.positions[splat * 3] * dx
      + cloud.positions[splat * 3 + 1] * dy
      + cloud.positions[splat * 3 + 2] * dz;
  }
  const order = into && into.length === count ? into : new Uint32Array(count);
  for (let splat = 0; splat < count; splat += 1) order[splat] = splat;
  // Descending: the largest projection along the view direction is the
  // furthest away, and goes first.
  const sorted = Array.from(order).sort((a, b) => depths[b] - depths[a]);
  order.set(sorted);
  return order;
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

  const packed = packSplats(cloud);
  const instances = gl.createBuffer()!;
  gl.bindBuffer(gl.ARRAY_BUFFER, instances);
  gl.bufferData(gl.ARRAY_BUFFER, packed.byteLength, gl.DYNAMIC_DRAW);
  const bytes = STRIDE * 4;
  const layout: [number, number, number][] = [
    [1, 3, 0],   // centre
    [2, 3, 12],  // scale
    [3, 4, 24],  // rotation
    [4, 1, 40],  // alpha
    [5, 3, 44],  // dc
  ];
  for (const [location, size, offset] of layout) {
    gl.enableVertexAttribArray(location);
    gl.vertexAttribPointer(location, size, gl.FLOAT, false, bytes, offset);
    gl.vertexAttribDivisor(location, 1);
  }
  gl.bindVertexArray(null);

  return {
    program,
    vao,
    corners,
    instances,
    packed,
    cloud,
    order: new Uint32Array(cloud.count),
    // No direction yet, so the first frame always sorts.
    sortedFor: [0, 0, 0],
    uniforms: {
      view: gl.getUniformLocation(program, 'uView'),
      projection: gl.getUniformLocation(program, 'uProjection'),
      viewport: gl.getUniformLocation(program, 'uViewport'),
      extent: gl.getUniformLocation(program, 'uExtent'),
    },
  };
}

/** The packed data in `order`, which is what the instance buffer holds. */
export function reorder(packed: Float32Array, order: Uint32Array, into: Float32Array): Float32Array {
  for (let slot = 0; slot < order.length; slot += 1) {
    const from = order[slot] * STRIDE;
    into.set(packed.subarray(from, from + STRIDE), slot * STRIDE);
  }
  return into;
}

/**
 * Re-sort when the camera has turned enough to matter.
 *
 * The threshold is on the direction rather than on a frame count: a still
 * camera never re-sorts, and one being dragged re-sorts as often as it must.
 */
export function ensureOrder(
  gl: WebGL2RenderingContext,
  splats: SplatResources,
  direction: readonly [number, number, number],
  scratch: { buffer?: Float32Array },
): boolean {
  const [px, py, pz] = splats.sortedFor;
  const [dx, dy, dz] = direction;
  if (px * dx + py * dy + pz * dz > RESORT_COSINE) return false;

  sortOrder(splats.cloud, direction, splats.order);
  splats.sortedFor = [dx, dy, dz];
  if (!scratch.buffer || scratch.buffer.length !== splats.packed.length) {
    scratch.buffer = new Float32Array(splats.packed.length);
  }
  reorder(splats.packed, splats.order, scratch.buffer);
  gl.bindBuffer(gl.ARRAY_BUFFER, splats.instances);
  gl.bufferSubData(gl.ARRAY_BUFFER, 0, scratch.buffer);
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
) {
  if (splats.cloud.count === 0) return;
  gl.useProgram(splats.program);
  gl.bindVertexArray(splats.vao);
  gl.uniformMatrix4fv(splats.uniforms.view, false, view);
  gl.uniformMatrix4fv(splats.uniforms.projection, false, projection);
  gl.uniform2f(splats.uniforms.viewport, viewportWidth, viewportHeight);
  gl.uniform1f(splats.uniforms.extent, extent);

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
  gl.deleteProgram(splats.program);
}
