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

import { coefficientsFor } from '../splat.ts';
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

// The higher harmonics, one texel a coefficient with rgb inside. Empty and
// unread when the file carried none.
uniform highp sampler2D uHarmonics;
uniform int uHarmonicsWidth;
uniform int uShDegree;
// The eye, in the world the positions are in, and the rotation that takes a
// world direction into the frame the harmonics were stored in -- which for a
// PLY is the upright turn and for a glTF splat the inverse of its node's
// rotation. The harmonics are a function of direction in their own frame.
uniform vec3 uEye;
uniform mat3 uShFrame;

uniform mat4 uView;
uniform mat4 uProjection;
uniform vec2 uViewport;
// How many standard deviations the quad reaches. Past three the gaussian is
// under 1.2% and the quad is mostly wasted fill.
uniform float uExtent;
// The longest ellipse axis this will draw, in pixels. Not a cull: an axis past
// it is shortened, and the splat is still drawn.
uniform float uMaxAxis;

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

vec3 harmonic(uint splat, int coefficients, int k) {
  int texel = int(splat) * coefficients + k;
  return texelFetch(uHarmonics, ivec2(texel % uHarmonicsWidth, texel / uHarmonicsWidth), 0).rgb;
}

// The real spherical harmonics of the 3DGS reference, up to degree 3. The
// constants and the term order are that implementation's; a different order
// would still be a basis, just not the one the file's coefficients were
// trained against.
const float C1 = 0.4886025119029199;
const float C2[5] = float[5](
  1.0925484305920792, -1.0925484305920792, 0.31539156525252005,
  -1.0925484305920792, 0.5462742152960396
);
const float C3[7] = float[7](
  -0.5900435899266435, 2.890611442640554, -0.4570457994644658,
  0.3731763325901154, -0.4570457994644658, 1.445305721320277,
  -0.5900435899266435
);

vec3 evaluateHarmonics(uint splat, vec3 dir, int degree) {
  if (degree <= 0) return vec3(0.0);
  int coefficients = (degree + 1) * (degree + 1) - 1;
  float x = dir.x, y = dir.y, z = dir.z;

  vec3 result = C1 * (-y * harmonic(splat, coefficients, 0)
                      + z * harmonic(splat, coefficients, 1)
                      - x * harmonic(splat, coefficients, 2));
  if (degree < 2) return result;

  float xx = x * x, yy = y * y, zz = z * z;
  float xy = x * y, yz = y * z, xz = x * z;
  result += C2[0] * xy * harmonic(splat, coefficients, 3)
          + C2[1] * yz * harmonic(splat, coefficients, 4)
          + C2[2] * (2.0 * zz - xx - yy) * harmonic(splat, coefficients, 5)
          + C2[3] * xz * harmonic(splat, coefficients, 6)
          + C2[4] * (xx - yy) * harmonic(splat, coefficients, 7);
  if (degree < 3) return result;

  result += C3[0] * y * (3.0 * xx - yy) * harmonic(splat, coefficients, 8)
          + C3[1] * xy * z * harmonic(splat, coefficients, 9)
          + C3[2] * y * (4.0 * zz - xx - yy) * harmonic(splat, coefficients, 10)
          + C3[3] * z * (2.0 * zz - 3.0 * xx - 3.0 * yy) * harmonic(splat, coefficients, 11)
          + C3[4] * x * (4.0 * zz - xx - yy) * harmonic(splat, coefficients, 12)
          + C3[5] * z * (xx - yy) * harmonic(splat, coefficients, 13)
          + C3[6] * x * (xx - 3.0 * yy) * harmonic(splat, coefficients, 14);
  return result;
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
  // Jacobian at the splat's own depth.
  //
  // The point it is evaluated at is first clamped to 1.3 times the frustum's
  // own tangent, which is where the linearisation stops meaning anything: a
  // splat far off axis, or very close, otherwise projects to an ellipse that
  // is not the shape it has. This is the reference implementation's remedy and
  // Blender's, and it clamps the evaluation point rather than the splat --
  // nothing is dropped.
  float fx = uProjection[0][0] * 0.5 * uViewport.x;
  float fy = uProjection[1][1] * 0.5 * uViewport.y;
  float limX = 1.3 / uProjection[0][0];
  float limY = 1.3 / uProjection[1][1];
  float z = -viewCenter.z;
  vec2 evaluateAt = vec2(
    clamp(viewCenter.x / z, -limX, limX) * z,
    clamp(viewCenter.y / z, -limY, limY) * z
  );

  float invZ = 1.0 / z;
  mat3x2 jacobian = mat3x2(
    fx * invZ, 0.0,
    0.0, fy * invZ,
    -fx * evaluateAt.x * invZ * invZ, -fy * evaluateAt.y * invZ * invZ
  );
  mat2 screen = jacobian * viewCovariance * transpose(jacobian);
  // A splat thinner than a pixel has no shape left to project; the dilation
  // keeps it a dot instead of an invisible sliver. The constant is the
  // reference implementation's.
  screen[0][0] += 0.3;
  screen[1][1] += 0.3;

  // The ellipse's own axes, from the 2x2's eigenvalues.
  float mid = 0.5 * (screen[0][0] + screen[1][1]);
  float discriminant = sqrt(max(0.0, mid * mid - determinant(screen)));
  float major = mid + discriminant;
  // Floored rather than allowed to reach zero, so an edge-on gaussian stays a
  // sliver with a width instead of a line with none. The reference's floor.
  float minor = max(mid - discriminant, 0.001);
  vec2 majorAxis = normalize(vec2(screen[0][1], major - screen[0][0]));
  if (screen[0][1] == 0.0) majorAxis = vec2(1.0, 0.0);
  vec2 minorAxis = vec2(majorAxis.y, -majorAxis.x);

  // The axes are capped in pixels, not as a share of the frame, and a splat
  // that hits the cap is drawn smaller than it is rather than dropped. That is
  // what the reference does, and it is the honest half-measure: the shape is
  // already approximate there, and removing it outright would take real
  // geometry with it.
  float majorLength = min(sqrt(major) * uExtent, uMaxAxis);
  float minorLength = min(sqrt(minor) * uExtent, uMaxAxis);

  vec2 offset = aCorner.x * majorAxis * majorLength
              + aCorner.y * minorAxis * minorLength;

  vLocal = aCorner * uExtent;
  // From the eye to the splat, as the reference and glTF both define it, and
  // turned into the harmonics' frame rather than turning the harmonics.
  vec3 dir = normalize(uShFrame * (aCenter - uEye));
  vec3 colour = SH_C0_CONST * aDc + evaluateHarmonics(aIndex, dir, uShDegree) + 0.5;
  vColour = vec4(colour, aAlpha);
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

// Whether the reconstructed colour is already light, rather than the display
// encoding 3DGS trains in.
uniform bool uLinearColour;

void main() {
  // The gaussian, in the quad's own coordinates. vLocal is already in
  // standard deviations, so this is exp(-r*r/2) and nothing else.
  float power = -0.5 * dot(vLocal, vLocal);
  float weight = exp(power) * vColour.a;
  if (weight < 1.0 / 255.0) discard;

  // A splat's colour is usually what a splat renderer would put on the
  // screen, which is display-referred; this frame is linear and tone mapped
  // on the way out. Writing the one into the other without converting is what
  // turns a lit scene into a white one. A splat that says its colour is
  // already linear skips the decode.
  //
  // The clamp comes first and is not a detail: a degree-0 harmonic reaches
  // outside [0, 1] for about a third of the values in a real scene, and the
  // higher bands do not always bring it back, so the colour has to be clamped
  // rather than allowed to shine.
  vec3 display = clamp(vColour.rgb, 0.0, 1.0);
  vec3 linear = uLinearColour ? display : pow(display, vec3(2.2));

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
  textureWidth: number;
  textureHeight: number;
  /** One RGB32F texel a harmonic coefficient, or null when there are none. */
  harmonics: WebGLTexture | null;
  harmonicsWidth: number;
  shDegree: number;
  cloud: SplatCloud;
  scratch: SortScratch;
  /** The view direction the order was made for. */
  sortedFor: [number, number, number];
  uniforms: {
    view: WebGLUniformLocation | null;
    projection: WebGLUniformLocation | null;
    viewport: WebGLUniformLocation | null;
    extent: WebGLUniformLocation | null;
    maxAxis: WebGLUniformLocation | null;
    splats: WebGLUniformLocation | null;
    textureWidth: WebGLUniformLocation | null;
    harmonics: WebGLUniformLocation | null;
    harmonicsWidth: WebGLUniformLocation | null;
    shDegree: WebGLUniformLocation | null;
    eye: WebGLUniformLocation | null;
    shFrame: WebGLUniformLocation | null;
    linearColour: WebGLUniformLocation | null;
  };
}

/** Floats a splat in the texture: four RGBA texels. */
const STRIDE = 16;

/**
 * Where `texels` of splat data go, on a device with this texture limit.
 *
 * The width is the device's maximum rather than a constant, because the height
 * is what runs out: degree-3 harmonics are fifteen texels a splat, and two
 * million splats need 14036 rows at a width of 2048 -- past the 8192 a
 * software rasteriser offers, where the failure is a blank frame and not an
 * error. Taking the width from the device makes the limit an area instead of a
 * height: 67 million texels at 8192, which is four million splats with their
 * harmonics.
 *
 * A cloud past even that is refused rather than truncated, because a truncated
 * one draws -- wrongly, and without saying so.
 */
function layoutFor(gl: WebGL2RenderingContext, texels: number, what: string) {
  const width = gl.getParameter(gl.MAX_TEXTURE_SIZE) as number;
  const height = Math.max(1, Math.ceil(texels / width));
  if (height > width) {
    throw new Error(
      `splat ${what}: ${texels} texels need ${height} rows of ${width}, past this device's ${width}`,
    );
  }
  return { width, height };
}

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
  const { width: textureWidth, height } = layoutFor(gl, cloud.count * 4, 'data');
  const padded = new Float32Array(textureWidth * height * 4);
  padded.set(packed);
  const texture = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.texImage2D(
    gl.TEXTURE_2D, 0, gl.RGBA32F, textureWidth, height, 0, gl.RGBA, gl.FLOAT, padded,
  );
  gl.bindTexture(gl.TEXTURE_2D, null);

  // The harmonics, one RGB32F texel a coefficient. Degree 3 is fifteen of them
  // a splat, which for a million-splat scene is 180 MB on the card -- the
  // honest price of colour that changes with the view, and the reason this is
  // a second texture rather than more channels on the first.
  let harmonics: WebGLTexture | null = null;
  let harmonicsWidth = 1;
  const perChannel = coefficientsFor(cloud.shDegree);
  if (perChannel > 0 && cloud.sh.length >= cloud.count * perChannel * 3) {
    const coefficientTexels = cloud.count * perChannel;
    const shLayout = layoutFor(gl, coefficientTexels, 'harmonics');
    harmonicsWidth = shLayout.width;
    const shHeight = shLayout.height;
    const shPadded = new Float32Array(harmonicsWidth * shHeight * 3);
    shPadded.set(cloud.sh.subarray(0, coefficientTexels * 3));
    harmonics = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, harmonics);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texImage2D(
      gl.TEXTURE_2D, 0, gl.RGB32F, harmonicsWidth, shHeight, 0, gl.RGB, gl.FLOAT, shPadded,
    );
    gl.bindTexture(gl.TEXTURE_2D, null);
  }

  return {
    program,
    vao,
    corners,
    instances,
    texture,
    textureWidth,
    textureHeight: height,
    harmonics,
    harmonicsWidth,
    shDegree: harmonics ? cloud.shDegree : 0,
    cloud,
    scratch: makeSortScratch(cloud.count),
    // No direction yet, so the first frame always sorts.
    sortedFor: [0, 0, 0],
    uniforms: {
      view: gl.getUniformLocation(program, 'uView'),
      projection: gl.getUniformLocation(program, 'uProjection'),
      viewport: gl.getUniformLocation(program, 'uViewport'),
      extent: gl.getUniformLocation(program, 'uExtent'),
      maxAxis: gl.getUniformLocation(program, 'uMaxAxis'),
      splats: gl.getUniformLocation(program, 'uSplats'),
      textureWidth: gl.getUniformLocation(program, 'uTextureWidth'),
      harmonics: gl.getUniformLocation(program, 'uHarmonics'),
      harmonicsWidth: gl.getUniformLocation(program, 'uHarmonicsWidth'),
      shDegree: gl.getUniformLocation(program, 'uShDegree'),
      eye: gl.getUniformLocation(program, 'uEye'),
      shFrame: gl.getUniformLocation(program, 'uShFrame'),
      linearColour: gl.getUniformLocation(program, 'uLinearColour'),
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
  /** The eye, in the world; the harmonics are a function of it. */
  eye: readonly [number, number, number] = [0, 0, 0],
  extent = 3,
  /** The longest ellipse axis to draw, in pixels; the reference uses 1024. */
  maxAxis = 1024,
) {
  if (splats.cloud.count === 0) return;
  gl.useProgram(splats.program);
  gl.bindVertexArray(splats.vao);
  gl.activeTexture(gl.TEXTURE0);
  gl.bindTexture(gl.TEXTURE_2D, splats.texture);
  gl.uniform1i(splats.uniforms.splats, 0);
  gl.uniform1i(splats.uniforms.textureWidth, splats.textureWidth);
  gl.activeTexture(gl.TEXTURE1);
  gl.bindTexture(gl.TEXTURE_2D, splats.harmonics);
  gl.uniform1i(splats.uniforms.harmonics, 1);
  gl.uniform1i(splats.uniforms.harmonicsWidth, splats.harmonicsWidth);
  gl.uniform1i(splats.uniforms.shDegree, splats.shDegree);
  gl.uniform3f(splats.uniforms.eye, eye[0], eye[1], eye[2]);
  // Row-major in the cloud, column-major in GLSL.
  const frame = splats.cloud.shFrame;
  gl.uniformMatrix3fv(splats.uniforms.shFrame, false, [
    frame[0], frame[3], frame[6],
    frame[1], frame[4], frame[7],
    frame[2], frame[5], frame[8],
  ]);
  gl.uniform1i(splats.uniforms.linearColour, splats.cloud.colorSpace === 'linear' ? 1 : 0);
  gl.activeTexture(gl.TEXTURE0);
  gl.uniformMatrix4fv(splats.uniforms.view, false, view);
  gl.uniformMatrix4fv(splats.uniforms.projection, false, projection);
  gl.uniform2f(splats.uniforms.viewport, viewportWidth, viewportHeight);
  gl.uniform1f(splats.uniforms.extent, extent);
  gl.uniform1f(splats.uniforms.maxAxis, maxAxis);

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
  if (splats.harmonics) gl.deleteTexture(splats.harmonics);
  gl.deleteProgram(splats.program);
}
