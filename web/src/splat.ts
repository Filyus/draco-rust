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
 * The rules here are not invented. They are measured against Blender's
 * importer in `dev/docs/format-research/notes/blender-gsplat.md`, one row per
 * value, and the activation functions are named in Blender's own
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
 * A splat cloud, with the dialect's activations already applied — which is
 * what Blender's importer hands its renderer, and the only form in which the
 * numbers mean what their names say.
 */
export interface SplatCloud {
  count: number;
  /** `xyz` per splat, as the file stores them. */
  positions: Float32Array;
  /** The gaussian's axes in metres: `exp` of the log the file stores. */
  scales: Float32Array;
  /** Normalized, `(w, x, y, z)` — the order the file writes, not `xyzw`. */
  rotations: Float32Array;
  /** `sigmoid(opacity)`, which is the alpha a renderer multiplies by. */
  alphas: Float32Array;
  /**
   * The degree-0 harmonic, raw and untouched, three per splat.
   *
   * Not a colour yet: a renderer makes one with `0.2820948 * dc + 0.5`, and
   * the result is routinely outside `[0, 1]` because the higher bands are
   * expected to bring it back.
   */
  dc: Float32Array;
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

/** Every property name the first stage of drawing a splat needs. */
export function splatPropertyNames(): string[] {
  return [...REQUIRED_SPLAT_PROPERTIES];
}

/** What `parse_ply_properties` hands back. */
export interface SelectedProperties {
  success: boolean;
  error?: string;
  count: number;
  positions: Float32Array;
  properties: Record<string, Float32Array>;
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

  return turnUpright({ count, positions, scales, rotations, alphas, dc });
}
