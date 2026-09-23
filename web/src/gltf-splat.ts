/**
 * `KHR_gaussian_splatting`: a glTF splat primitive as a `SplatCloud`.
 *
 * The same splat a 3DGS PLY describes, in other domains. glTF stores the
 * gaussian's axes as lengths rather than their log, its opacity as the alpha
 * itself rather than a logit, its orientation in glTF's `(x, y, z, w)` order,
 * and its harmonics one `VEC3` a coefficient -- which is already the layout
 * `SplatCloud.sh` keeps. The basis, its signs, the coefficient order within a
 * degree, the `0.5` bias and the direction the harmonics are asked about
 * (from the eye to the splat) are the reference implementation's, and the
 * specification writes them out term for term.
 *
 * What glTF adds is the node. A splat primitive's positions are in its node's
 * space, and the specification has the harmonics turn with the node's
 * rotation. Positions, orientations and scales are moved into the world here,
 * as Cesium does; the harmonics stay where they were stored, and the node's
 * inverse rotation becomes the cloud's `shFrame`, so the renderer turns the
 * view direction instead of the coefficients.
 *
 * Read against the ratified specification (KhronosGroup/glTF `81762cc`), and
 * checked against three implementations of it: Khronos's sample renderer and
 * Cesium read the harmonics exactly this way, and two converters from PLY
 * write the domains exactly this way. What those converters do *not* agree
 * on is the frame -- one leaves a Y-down PLY as it is, one turns it half about
 * Z, this viewer turns it half about X -- so nothing here assumes one: the
 * file's nodes say where its splats stand.
 *
 * The draft form some tools still write -- harmonics as `VEC4`, colour in
 * `COLOR_0` with no `SH_DEGREE_0_COEF_0`, no `colorSpace` -- is not this, and
 * is reported rather than guessed at.
 */
import {
  componentByteWidth, isNormalizedIntegerType, normalizeComponent, readComponent,
} from './component-values.ts';
import { composeTrs, decomposeMat4, identityMat4, multiplyMat4 } from './mat4.ts';
import type { SceneAccessor, SceneDocument, ScenePrimitive } from './scene-document.ts';
import { coefficientsFor } from './splat.ts';
import type { SplatCloud } from './splat.ts';

const PREFIX = 'KHR_gaussian_splatting:';

/** What a document's splat primitives come to, and what could not be taken. */
export interface GltfSplats {
  cloud: SplatCloud | null;
  warnings: string[];
}

/** The harmonics' semantic for one coefficient of one degree. */
function harmonicSemantic(degree: number, coefficient: number): string {
  return `${PREFIX}SH_DEGREE_${degree}_COEF_${coefficient}`;
}

/**
 * An accessor's values as floats, dequantized the way glTF defines it: a
 * normalized integer is a fraction of its range, anything else is its value.
 * Null when it does not hold `components` values per element.
 */
function readFloats(accessor: SceneAccessor | undefined, components: number): Float32Array | null {
  if (!accessor || accessor.components !== components) return null;
  const width = componentByteWidth(accessor.componentType);
  if (width === undefined) return null;
  const length = accessor.count * components;
  if (accessor.bytes.byteLength < length * width) return null;
  if (accessor.componentType === 5126) {
    return new Float32Array(
      accessor.bytes.buffer.slice(accessor.bytes.byteOffset, accessor.bytes.byteOffset + length * 4),
    );
  }
  const view = new DataView(accessor.bytes.buffer, accessor.bytes.byteOffset, accessor.bytes.byteLength);
  const normalized = Boolean(accessor.normalized) && isNormalizedIntegerType(accessor.componentType);
  const values = new Float32Array(length);
  for (let index = 0; index < length; index += 1) {
    const value = readComponent(view, index * width, accessor.componentType);
    values[index] = normalized ? normalizeComponent(value, accessor.componentType) : value;
  }
  return values;
}

/** Each node's world matrix, column-major, or null for a node no scene reaches. */
function worldMatrices(document: SceneDocument): Array<number[] | null> {
  const worlds: Array<number[] | null> = document.nodes.map(() => null);
  const visit = (index: number, parent: number[]) => {
    const node = document.nodes[index];
    if (!node || worlds[index]) return;
    const local = node.matrix && node.matrix.length === 16
      ? node.matrix
      : composeTrs({
        translation: node.translation || [0, 0, 0],
        rotation: node.rotation || [0, 0, 0, 1],
        scale: node.scale || [1, 1, 1],
      });
    const world = multiplyMat4(parent, local) || identityMat4();
    worlds[index] = world;
    for (const child of node.children || []) visit(child, world);
  };
  for (const root of document.rootNodes) visit(root, identityMat4());
  return worlds;
}

/** `r` then `q`, both `(x, y, z, w)`: the orientation `q` has once `r` turns it. */
function multiplyQuaternions(r: readonly number[], q: readonly number[]): number[] {
  const [rx, ry, rz, rw] = r;
  const [qx, qy, qz, qw] = q;
  return [
    rw * qx + rx * qw + ry * qz - rz * qy,
    rw * qy - rx * qz + ry * qw + rz * qx,
    rw * qz + rx * qy - ry * qx + rz * qw,
    rw * qw - rx * qx - ry * qy - rz * qz,
  ];
}

/** The rotation `(x, y, z, w)` stands for, inverted, row-major: world to local. */
function inverseRotationRowMajor(q: readonly number[]): number[] {
  const [x, y, z, w] = q;
  // The rotation matrix, transposed as it is written out.
  return [
    1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y),
    2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x),
    2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y),
  ];
}

/** One primitive's splats, in the world, before they are joined into a cloud. */
interface PrimitiveSplats {
  count: number;
  positions: Float32Array;
  scales: Float32Array;
  rotations: Float32Array;
  alphas: Float32Array;
  dc: Float32Array;
  /** Coefficient-major, rgb inside, `coefficientsFor(degree)` a splat. */
  sh: Float32Array;
  degree: number;
  shFrame: number[];
  colorSpace: 'srgb' | 'linear';
}

/**
 * The highest degree whose every coefficient is present, and whether the file
 * carried part of a degree above it -- which the specification forbids and
 * which is therefore reported rather than read as a lower degree silently.
 */
function harmonicDegree(primitive: ScenePrimitive): { degree: number; partial: boolean } {
  let degree = 0;
  for (let candidate = 1; candidate <= 3; candidate += 1) {
    const present = Array.from({ length: 2 * candidate + 1 }, (_, coefficient) =>
      primitive.attributes[harmonicSemantic(candidate, coefficient)] !== undefined);
    if (present.every(Boolean)) {
      degree = candidate;
      continue;
    }
    const partial = present.some(Boolean)
      || [candidate + 1, candidate + 2].some((above) => above <= 3
        && primitive.attributes[harmonicSemantic(above, 0)] !== undefined);
    return { degree, partial };
  }
  return { degree, partial: false };
}

function readPrimitive(
  document: SceneDocument,
  primitive: ScenePrimitive,
  world: number[],
  label: string,
  warnings: string[],
): PrimitiveSplats | null {
  if ((primitive.mode ?? 4) !== 0) {
    warnings.push(`${label}: a Gaussian splat primitive must be POINTS; skipped`);
    return null;
  }
  const accessor = (semantic: string) => {
    const index = primitive.attributes[semantic];
    return index === undefined ? undefined : document.accessors[index];
  };
  const positions = readFloats(accessor('POSITION'), 3);
  const rotations = readFloats(accessor(`${PREFIX}ROTATION`), 4);
  const scales = readFloats(accessor(`${PREFIX}SCALE`), 3);
  const opacities = readFloats(accessor(`${PREFIX}OPACITY`), 1);
  const dc = readFloats(accessor(harmonicSemantic(0, 0)), 3);
  if (!positions || !rotations || !scales || !opacities || !dc) {
    // The draft form is the likely one: colour in COLOR_0 and harmonics as
    // VEC4. Saying which attribute failed is what makes that recognizable.
    const missing = [
      ['POSITION', positions], ['ROTATION', rotations], ['SCALE', scales],
      ['OPACITY', opacities], ['SH_DEGREE_0_COEF_0', dc],
    ].filter(([, values]) => !values).map(([name]) => name);
    warnings.push(
      `${label}: not a Gaussian splat as KHR_gaussian_splatting defines it `
      + `(missing or mistyped: ${missing.join(', ')}); drawn as points`,
    );
    return null;
  }
  const count = positions.length / 3;
  if ([rotations.length / 4, scales.length / 3, opacities.length, dc.length / 3]
    .some((length) => length !== count)) {
    warnings.push(`${label}: its splat attributes disagree on the splat count; skipped`);
    return null;
  }

  const { degree: declaredDegree, partial } = harmonicDegree(primitive);
  if (partial) {
    warnings.push(
      `${label}: a spherical harmonic degree is only partly present, which the extension forbids; `
      + `read to degree ${declaredDegree}`,
    );
  }
  let degree = declaredDegree;
  const planes: Float32Array[] = [];
  for (let band = 1; band <= degree; band += 1) {
    for (let coefficient = 0; coefficient < 2 * band + 1; coefficient += 1) {
      const plane = readFloats(accessor(harmonicSemantic(band, coefficient)), 3);
      if (!plane || plane.length !== count * 3) {
        warnings.push(`${label}: harmonic degree ${band} is not VEC3 per splat; read to degree ${band - 1}`);
        degree = band - 1;
        break;
      }
      planes.push(plane);
    }
    if (degree < band) break;
  }
  const perChannel = coefficientsFor(degree);
  planes.length = perChannel;

  // An unknown or missing colorSpace is read as the display encoding 3DGS
  // trains in, which is what a file that omits it was trained in. Khronos's
  // sample renderer falls back to linear instead; the specification leaves
  // it open.
  const extension = primitive.gaussianSplatting;
  let colorSpace: 'srgb' | 'linear' = 'srgb';
  if (extension?.colorSpace === 'lin_rec709_display') {
    colorSpace = 'linear';
  } else if (extension?.colorSpace !== 'srgb_rec709_display') {
    warnings.push(
      `${label}: colorSpace ${JSON.stringify(extension?.colorSpace ?? null)} is not one the `
      + 'extension defines; read as srgb_rec709_display',
    );
  }
  if (extension && extension.kernel !== 'ellipse') {
    warnings.push(`${label}: kernel ${JSON.stringify(extension.kernel)} is drawn as an ellipse`);
  }

  // The node: positions take the whole matrix; orientations its rotation; the
  // axes its scale, which only a uniform scale carries exactly -- a
  // non-uniform one shears a rotated gaussian into a shape no axis triple
  // describes, and is approximated by its mean.
  const { rotation, scale } = decomposeMat4(world);
  const meanScale = Math.cbrt(Math.abs(scale[0] * scale[1] * scale[2])) || 1;
  if (Math.max(...scale) - Math.min(...scale) > 1e-4 * meanScale) {
    warnings.push(`${label}: its node scales unevenly; the gaussians are scaled by the mean`);
  }

  const outPositions = new Float32Array(count * 3);
  const outScales = new Float32Array(count * 3);
  const outRotations = new Float32Array(count * 4);
  const outAlphas = new Float32Array(count);
  const outSh = new Float32Array(count * perChannel * 3);
  for (let splat = 0; splat < count; splat += 1) {
    const [x, y, z] = [positions[splat * 3], positions[splat * 3 + 1], positions[splat * 3 + 2]];
    for (let axis = 0; axis < 3; axis += 1) {
      outPositions[splat * 3 + axis] = world[axis] * x + world[4 + axis] * y + world[8 + axis] * z
        + world[12 + axis];
      outScales[splat * 3 + axis] = scales[splat * 3 + axis] * meanScale;
    }
    const turned = multiplyQuaternions(rotation, [
      rotations[splat * 4], rotations[splat * 4 + 1], rotations[splat * 4 + 2], rotations[splat * 4 + 3],
    ]);
    // Normalized here even though the file promises unit quaternions: a
    // normalized byte or short cannot hold one exactly.
    const length = Math.hypot(...turned) || 1;
    outRotations.set([turned[3] / length, turned[0] / length, turned[1] / length, turned[2] / length], splat * 4);
    outAlphas[splat] = Math.min(1, Math.max(0, opacities[splat]));
    for (let coefficient = 0; coefficient < perChannel; coefficient += 1) {
      outSh.set(planes[coefficient].subarray(splat * 3, splat * 3 + 3), (splat * perChannel + coefficient) * 3);
    }
  }
  return {
    count,
    positions: outPositions,
    scales: outScales,
    rotations: outRotations,
    alphas: outAlphas,
    dc: dc.slice(0, count * 3),
    sh: outSh,
    degree,
    shFrame: inverseRotationRowMajor(rotation),
    colorSpace,
  };
}

/**
 * Every splat primitive a document's scenes place, as one cloud.
 *
 * The viewer draws one cloud, so several primitives are joined. What cannot be
 * joined exactly is reported: primitives whose harmonics stop at different
 * degrees are cut to the lowest, and ones placed under different rotations
 * share the first one's harmonic frame, which leaves the others' view-dependent
 * colour turned.
 */
export function splatCloudFromSceneDocument(document: SceneDocument): GltfSplats {
  const warnings: string[] = [];
  const worlds = worldMatrices(document);
  const parts: PrimitiveSplats[] = [];
  document.nodes.forEach((node, nodeIndex) => {
    const world = worlds[nodeIndex];
    if (node.mesh === undefined || !world) return;
    const mesh = document.meshes[node.mesh];
    mesh?.primitives.forEach((primitive, primitiveIndex) => {
      const isSplat = primitive.gaussianSplatting
        || Object.keys(primitive.attributes).some((semantic) => semantic.startsWith(PREFIX));
      if (!isSplat) return;
      if (node.instancing) {
        warnings.push(`node ${nodeIndex}: GPU instancing of a splat primitive is not drawn; one copy is`);
      }
      const label = `mesh ${node.mesh} primitive ${primitiveIndex}`;
      const part = readPrimitive(document, primitive, world, label, warnings);
      if (part) parts.push(part);
    });
  });
  if (parts.length === 0) return { cloud: null, warnings };

  const degree = Math.min(...parts.map((part) => part.degree));
  if (parts.some((part) => part.degree !== degree)) {
    warnings.push(`Splat primitives carry different harmonic degrees; all are drawn at degree ${degree}`);
  }
  const [first] = parts;
  if (parts.some((part) => part.shFrame.some((value, index) => Math.abs(value - first.shFrame[index]) > 1e-6))) {
    warnings.push('Splat primitives are placed under different rotations; their view-dependent colour uses the first one\'s');
  }
  if (parts.some((part) => part.colorSpace !== first.colorSpace)) {
    warnings.push(`Splat primitives disagree on colorSpace; all are drawn as ${first.colorSpace}`);
  }

  const count = parts.reduce((sum, part) => sum + part.count, 0);
  const perChannel = coefficientsFor(degree);
  const cloud: SplatCloud = {
    count,
    positions: new Float32Array(count * 3),
    scales: new Float32Array(count * 3),
    rotations: new Float32Array(count * 4),
    alphas: new Float32Array(count),
    dc: new Float32Array(count * 3),
    sh: new Float32Array(count * perChannel * 3),
    shDegree: degree,
    shFrame: Float32Array.from(first.shFrame),
    colorSpace: first.colorSpace,
  };
  let at = 0;
  for (const part of parts) {
    cloud.positions.set(part.positions, at * 3);
    cloud.scales.set(part.scales, at * 3);
    cloud.rotations.set(part.rotations, at * 4);
    cloud.alphas.set(part.alphas, at);
    cloud.dc.set(part.dc, at * 3);
    const partPerChannel = coefficientsFor(part.degree);
    for (let splat = 0; splat < part.count; splat += 1) {
      cloud.sh.set(
        part.sh.subarray(splat * partPerChannel * 3, (splat * partPerChannel + perChannel) * 3),
        (at + splat) * perChannel * 3,
      );
    }
    at += part.count;
  }
  return { cloud, warnings };
}
