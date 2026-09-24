/**
 * The Gaussian-splat PLY dialect: what makes a file one, and what its
 * properties mean.
 *
 * PLY has no notion of a splat. A splat file is an ordinary point cloud whose
 * payload lives entirely in properties no mesh reader names, and the agreement
 * about which names those are, and what the numbers in them stand for, is a
 * convention outside the format. This module is that convention, and nothing
 * else in the pipeline is allowed to know it: the wasm reader fetches
 * properties by name, and everything downstream sees the typed result below.
 *
 * The rules here are not invented. They were measured against Blender's
 * importer one row per value -- the rows are in `tests/splat-dialect.ts` --
 * and the activation functions are named in Blender's own
 * `io/common/IO_gsplat.hh` crediting the 3DGS supplemental code — so they
 * belong to the dialect rather than to Blender.
 */

/** The properties a file must have, all of them, to be a splat. */
export const REQUIRED_SPLAT_PROPERTIES = [
  'f_dc_0', 'f_dc_1', 'f_dc_2',
  'opacity',
  'scale_0', 'scale_1', 'scale_2',
  'rot_0', 'rot_1', 'rot_2', 'rot_3',
] as const;

/**
 * A splat cloud, with every activation already applied — the only form in
 * which the numbers mean what their names say, and the one every reader
 * produces whatever its file stored.
 */
export interface SplatCloud {
  count: number;
  /** `xyz` per splat, in the viewer's world: Y up. */
  positions: Float32Array;
  /** The gaussian's axes as lengths, not the log a PLY stores. */
  scales: Float32Array;
  /** Normalized, `(w, x, y, z)` — not glTF's `xyzw`. */
  rotations: Float32Array;
  /** The alpha a renderer multiplies by, not the logit a PLY stores. */
  alphas: Float32Array;
  /**
   * The degree-0 harmonic, raw and untouched, three per splat.
   *
   * Not a colour yet: a renderer makes one with `0.2820948 * dc + 0.5`, and
   * the result is routinely outside `[0, 1]` because the higher bands are
   * expected to bring it back.
   */
  dc: Float32Array;
  /**
   * The higher bands, `3 * coefficientsFor(shDegree)` floats a splat, ordered
   * coefficient-major with rgb inside -- which is a PLY's layout transposed,
   * and exactly glTF's `SH_DEGREE_l_COEF_m` in order.
   *
   * **In the frame the file stored them in, not the world's.** Positions and
   * orientations are moved into the world on the way in, and these are not,
   * because rotating a harmonic basis is a much heavier job than rotating a
   * direction. So the renderer turns the view direction into this frame
   * instead, through `shFrame`.
   */
  sh: Float32Array;
  /** 0 when the file carried no usable harmonics, up to 3. */
  shDegree: number;
  /**
   * Takes a direction in the world into the frame `sh` is stored in: nine
   * numbers, row-major. What the harmonics are asked about is this times the
   * direction from the eye to the splat.
   *
   * A PLY's is the upright turn, which is its own inverse; a glTF splat's is
   * the inverse of the rotation of the node that placed it.
   */
  shFrame: Float32Array;
  /**
   * What the reconstructed colour means. `srgb` is display-encoded, as 3DGS
   * trains, and is decoded to light before blending; `linear` already is
   * light. glTF names which one a splat is in `colorSpace`.
   */
  colorSpace: 'srgb' | 'linear';
}

/**
 * The half turn about X that stands a 3DGS PLY upright: `y` and `z` change
 * sign. Row-major, and its own inverse.
 */
export const UPRIGHT_TURN: readonly number[] = [1, 0, 0, 0, -1, 0, 0, 0, -1];

/** Coefficients per colour channel at each degree, excluding the DC term. */
export function coefficientsFor(degree: number): number {
  return degree <= 0 ? 0 : (degree + 1) * (degree + 1) - 1;
}

/**
 * The degree a count of `f_rest` values describes, as Blender reads it.
 *
 * `ply_import_gsplat.cc` turns the count into a dimension and puts it through
 * `degree_for_dimension`, a staircase with thresholds at 3 / 8 / 15 / 24. Both
 * steps lose data and both are deliberate here rather than repaired: a count
 * that is not a multiple of three collapses to degree 0, and one between
 * thresholds is floored to the degree below. Matching the importer matters
 * more than salvaging the odd file, because the importer is the oracle this
 * module is checked against.
 */
export function degreeForRestCount(count: number): number {
  if (count <= 0 || count % 3 !== 0) return 0;
  const dimension = count / 3;
  if (dimension >= 15) return 3;
  if (dimension >= 8) return 2;
  if (dimension >= 3) return 1;
  return 0;
}

/**
 * The direction to ask a splat's harmonics about: from the eye to the splat,
 * normalized, turned into the harmonics' own frame.
 *
 * `from` and `to` are in the world, where the rest of `SplatCloud` lives;
 * `shFrame` is the cloud's. What the renderer computes, in one place a test
 * can reach.
 */
export function viewDirectionForSh(
  from: readonly [number, number, number],
  to: readonly [number, number, number],
  shFrame: ArrayLike<number> = UPRIGHT_TURN,
): [number, number, number] {
  const x = to[0] - from[0];
  const y = to[1] - from[1];
  const z = to[2] - from[2];
  const length = Math.hypot(x, y, z) || 1;
  const turned = [0, 1, 2].map((row) => (
    shFrame[row * 3] * x + shFrame[row * 3 + 1] * y + shFrame[row * 3 + 2] * z
  ) / length);
  return [turned[0], turned[1], turned[2]];
}

/**
 * Whether these property names describe a splat.
 *
 * Blender's `convert_gsplat_ply_to_point_cloud` requires all eleven and treats
 * `f_rest_*` as optional; a file missing any one of them is ordinary geometry.
 * Taken verbatim, because a looser rule here would claim files that importer
 * would not.
 */
export function isSplatPly(properties: readonly string[]): boolean {
  const present = new Set(properties);
  return REQUIRED_SPLAT_PROPERTIES.every((name) => present.has(name));
}

/** The splat's properties by name, as the reader below takes them. */
export interface SelectedProperties {
  success: boolean;
  error?: string;
  count: number;
  positions: Float32Array;
  properties: Record<string, Float32Array>;
}

/**
 * One attribute a reader carried without interpreting it, by name.
 *
 * The PLY reader hands these over for every property a mesh has no slot for,
 * and the Draco reader for every generic attribute whose name travelled in
 * metadata. A splat is nothing but these, which is why both formats reach the
 * splat reader the same way.
 */
export interface NamedAttribute {
  name?: string | null;
  components: number;
  /** One tuple per point, `components` long. */
  values: ArrayLike<number>;
}

/**
 * Carried attributes as the splat reader's input.
 *
 * A multi-component attribute is spread into `name_0`, `name_1`, ... because
 * that is how the same values appear in a PLY, and every rule in this module
 * is written against the PLY's names. An attribute without a name is skipped:
 * nothing here can guess what it was.
 *
 * A single-component `Float32Array` of the right length is taken as it is
 * rather than copied. That is every property of a splat PLY, and copying them
 * would briefly hold a second copy of a payload that runs to hundreds of
 * megabytes.
 */
export function selectedFromNamedAttributes(
  count: number,
  positions: ArrayLike<number>,
  attributes: readonly NamedAttribute[],
): SelectedProperties {
  const properties: Record<string, Float32Array> = {};
  for (const extra of attributes) {
    if (!extra.name) continue;
    const components = Math.max(1, extra.components);
    if (components === 1 && extra.values instanceof Float32Array
      && extra.values.length === count) {
      properties[extra.name] = extra.values;
      continue;
    }
    for (let component = 0; component < components; component += 1) {
      const name = components === 1 ? extra.name : `${extra.name}_${component}`;
      const column = new Float32Array(count);
      for (let point = 0; point < count; point += 1) {
        column[point] = extra.values[point * components + component];
      }
      properties[name] = column;
    }
  }
  return { success: true, count, positions: Float32Array.from(positions), properties };
}

/**
 * The bit budget a splat is written with, per attribute.
 *
 * Measured rather than chosen: each setting here was encoded, decoded, drawn
 * from eight cameras and compared against the unquantized scene, on a street
 * capture and two interiors, in linear radiance.
 *
 * The errors of different properties add, so each group is worth bits in
 * proportion to what it removes, and once the harmonics are coarsened the
 * rest carry most of what error is left.
 *
 * - **Positions at 18.** 16 bits over a street's box leave a visible step:
 *   18 buy it 1.2 dB for 0.75 bytes a splat, 93 per cent of what 20 buy. An
 *   interior gains nothing, and loses 2 per cent of the file. 12 bits cost
 *   5 to 15 dB.
 * - **Harmonics at 6.** A quarter of the file for 0.1 dB on the street and
 *   about 1 dB on an interior -- the best trade measured by a distance. Four
 *   bits halve the file again and cost an interior 10 dB.
 * - **Everything else at 10.** Colour, opacity and scale: 1.7 bytes a splat
 *   buy 2.8 to 3.3 dB over 8 bits, and 12 add only 0.3 for the same again.
 *   Rotation: a byte buys 1.2 to 1.6 dB, nearly all that 12 bits buy for two.
 *   Opacity stays a logit: storing alpha instead was measured as a small loss,
 *   not a gain.
 *
 * At this budget the street renders at 54.8 dB in 34.8 bytes a splat and an
 * interior at 54.6 in 34.3, against SPZ at 8/8 with 49.5 in 35.9 and 52.1 in
 * 40.9.
 */
export const SPLAT_BUDGET = { positions: 18, harmonics: 6, other: 10 } as const;

/** The bits a splat property is written with, by its name in the file. */
export function splatBitsFor(name: string): number {
  return name.startsWith('f_rest_') ? SPLAT_BUDGET.harmonics : SPLAT_BUDGET.other;
}

function sigmoid(x: number): number {
  return 1 / (1 + Math.exp(-x));
}

/**
 * Turn the file's frame the right way up.
 *
 * 3DGS keeps the world frame COLMAP hands it, in which **Y points down**. A
 * viewer whose world is Y-up therefore shows a splat scene upside down, and no
 * camera can fix that: it is a property of the coordinates, not of the view.
 *
 * Measured rather than assumed: Blender's viewport has to be flown to an
 * up vector of −Y before this corpus's `train` scene stands upright, and
 * COLMAP's convention says why.
 *
 * The turn is a half rotation about X: `y` and `z` change sign. Positions
 * follow directly; a gaussian's orientation follows as `r · q` for that same
 * half turn, which for `r = (0, 1, 0, 0)` in `(w, x, y, z)` order works out to
 * the shuffle below. The axis lengths do not change — a rotation does not
 * stretch anything.
 */
function turnUpright(cloud: SplatCloud): SplatCloud {
  for (let splat = 0; splat < cloud.count; splat += 1) {
    cloud.positions[splat * 3 + 1] = -cloud.positions[splat * 3 + 1];
    cloud.positions[splat * 3 + 2] = -cloud.positions[splat * 3 + 2];

    const w = cloud.rotations[splat * 4];
    const x = cloud.rotations[splat * 4 + 1];
    const y = cloud.rotations[splat * 4 + 2];
    const z = cloud.rotations[splat * 4 + 3];
    cloud.rotations[splat * 4] = -x;
    cloud.rotations[splat * 4 + 1] = w;
    cloud.rotations[splat * 4 + 2] = -z;
    cloud.rotations[splat * 4 + 3] = y;
  }
  return cloud;
}

/**
 * The selected properties as a splat cloud, or `null` when they do not
 * describe one.
 *
 * Returning `null` rather than throwing, and rather than filling the gaps with
 * defaults: a file that is not a splat is an ordinary point cloud and has a
 * perfectly good path of its own.
 */
export function readSplatCloud(selected: SelectedProperties): SplatCloud | null {
  if (!selected.success) return null;
  const { count, properties } = selected;
  if (!isSplatPly(Object.keys(properties))) return null;
  if (selected.positions.length < count * 3) return null;
  for (const name of REQUIRED_SPLAT_PROPERTIES) {
    if (properties[name].length < count) return null;
  }

  // How many harmonics the file actually carried, as Blender would count them.
  let restCount = 0;
  while (properties[`f_rest_${restCount}`]?.length >= count) restCount += 1;
  const shDegree = degreeForRestCount(restCount);
  const perChannel = coefficientsFor(shDegree);
  const sh = new Float32Array(count * perChannel * 3);

  const scales = new Float32Array(count * 3);
  const rotations = new Float32Array(count * 4);
  const alphas = new Float32Array(count);
  const dc = new Float32Array(count * 3);

  const scalePlanes = [properties.scale_0, properties.scale_1, properties.scale_2];
  const dcPlanes = [properties.f_dc_0, properties.f_dc_1, properties.f_dc_2];
  const rotPlanes = [properties.rot_0, properties.rot_1, properties.rot_2, properties.rot_3];
  const opacity = properties.opacity;

  for (let splat = 0; splat < count; splat += 1) {
    for (let axis = 0; axis < 3; axis += 1) {
      scales[splat * 3 + axis] = Math.exp(scalePlanes[axis][splat]);
      dc[splat * 3 + axis] = dcPlanes[axis][splat];
    }
    alphas[splat] = sigmoid(opacity[splat]);

    // The file is channel-major: every coefficient of red, then of green, then
    // of blue. What a renderer wants is one coefficient at a time with rgb
    // together, which is the transpose — and getting it backwards does not
    // fail, it tints.
    for (let k = 0; k < perChannel; k += 1) {
      const at = (splat * perChannel + k) * 3;
      sh[at] = properties[`f_rest_${k}`][splat];
      sh[at + 1] = properties[`f_rest_${k + perChannel}`][splat];
      sh[at + 2] = properties[`f_rest_${k + 2 * perChannel}`][splat];
    }

    // Normalized on import, as Blender does. A zero quaternion has no rotation
    // to recover, so it becomes the identity rather than a NaN that would take
    // the splat off screen without saying why.
    let w = rotPlanes[0][splat];
    let x = rotPlanes[1][splat];
    let y = rotPlanes[2][splat];
    let z = rotPlanes[3][splat];
    const length = Math.hypot(w, x, y, z);
    if (length > 0) {
      w /= length; x /= length; y /= length; z /= length;
    } else {
      w = 1; x = 0; y = 0; z = 0;
    }
    rotations[splat * 4] = w;
    rotations[splat * 4 + 1] = x;
    rotations[splat * 4 + 2] = y;
    rotations[splat * 4 + 3] = z;
  }

  // The positions are the reader's buffer, and turning the cloud writes to
  // them, so take a copy rather than reach back into what the caller holds.
  const positions = selected.positions.slice(0, count * 3);

  return turnUpright({
    count, positions, scales, rotations, alphas, dc, sh, shDegree,
    shFrame: Float32Array.from(UPRIGHT_TURN),
    colorSpace: 'srgb',
  });
}
