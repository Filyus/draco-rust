/**
 * The space an FBX file states it is in, and the two spaces this writer offers.
 *
 * FBX does not fix a coordinate system. It declares one, in seven
 * `GlobalSettings` fields: `UnitScaleFactor` says how many centimetres are in a
 * unit, and six more say which axis is up, which points front and which points
 * right, each with a sign. glTF fixes all of them — metres, `+Y` up, `+Z`
 * front, `+X` right — so both directions across the boundary are the same
 * question asked twice, and answering it in one place is what keeps them
 * inverses of each other.
 *
 * Before this, each side answered separately and neither read the file: the
 * importer divided by a hundred and ignored the axes, the writer multiplied by a
 * hundred or turned the geometry depending on where the document came from, and
 * the declarations matched none of it.
 */

import { identityMat4, invertMat4 } from './mat4.ts';

/** The seven fields, as `fbx-wasm` spells them. */
export interface FbxGlobalSettings {
  upAxis?: number;
  upAxisSign?: number;
  frontAxis?: number;
  frontAxisSign?: number;
  coordAxis?: number;
  coordAxisSign?: number;
  unitScaleFactor?: number;
  originalUnitScaleFactor?: number;
  timeMode?: number;
}

export interface FbxSpace {
  /** What the file says, or what a file being written will say. */
  settings: FbxGlobalSettings;
  /** Metres in one unit: `UnitScaleFactor / 100`. */
  metersPerUnit: number;
  /** FBX → glTF, column-major, and back. */
  basis: number[];
  inverse: number[];
  /** The basis as a quaternion, for re-basing rotations. */
  rotation: number[];
  /** Which FBX axis feeds each glTF axis, for permuting per-axis scale. */
  axes: [number, number, number];
  /**
   * Whether the space is glTF's already.
   *
   * Not an optimization: it is the guarantee that a file needing no change is
   * handed back untouched rather than run through arithmetic that ought to
   * cancel out. Every real-world fixture here is such a file.
   */
  identity: boolean;
}

/**
 * glTF's own space, spelled as FBX would.
 *
 * `+Y` up, `+Z` front, `+X` right, and a hundred centimetres to the unit, which
 * is to say metres. Every FBX this workspace has to hand — Mixamo, Samba
 * Dancing, `morph_test`, the Stanford bunny — states these axes, so it is both
 * the least surprising thing to write and the one that needs no conversion.
 */
export const FBX_METERS_Y_UP: FbxGlobalSettings = {
  upAxis: 1,
  upAxisSign: 1,
  frontAxis: 2,
  frontAxisSign: 1,
  coordAxis: 0,
  coordAxisSign: 1,
  unitScaleFactor: 100,
  originalUnitScaleFactor: 100,
};

/**
 * `+Z` up, still metres: what this writer produced before the choice existed.
 *
 * A good deal of existing FBX is Z-up, and a consumer that ignores the declared
 * axes will show a Y-up file lying on its side, so the convention is worth
 * keeping reachable. It is not the default, and not only because Y-up needs no
 * conversion: the writer turns positions and node transforms but not animated
 * rotations, so a rotating node written into this space rotates about the wrong
 * axis. Static geometry is unaffected.
 *
 * The two signs describe what the conversion actually does — glTF `(x, y, z)`
 * written as `(x, z, -y)` puts up along `-Z` — rather than what it was once
 * claimed to do.
 */
export const FBX_METERS_Z_UP: FbxGlobalSettings = {
  upAxis: 2,
  upAxisSign: -1,
  frontAxis: 1,
  frontAxisSign: 1,
  coordAxis: 0,
  coordAxisSign: 1,
  unitScaleFactor: 100,
  originalUnitScaleFactor: 100,
};

/** The spaces the export routes can ask for, by name. */
export const FBX_EXPORT_SPACES = {
  'meters-y-up': FBX_METERS_Y_UP,
  'meters-z-up': FBX_METERS_Z_UP,
} as const;

export type FbxExportSpaceName = keyof typeof FBX_EXPORT_SPACES;

/**
 * The space FBX exports are written in unless a caller says otherwise.
 *
 * glTF's own, which makes the conversion the identity and the round trip exact,
 * and which is what every FBX on hand from other tools declares.
 */
export const DEFAULT_FBX_EXPORT_SPACE: FbxExportSpaceName = 'meters-y-up';

/** Resolve the seven fields into everything both directions need. */
export function fbxSpace(settings: FbxGlobalSettings | null | undefined): FbxSpace {
  const source = settings || {};
  const axis = (value: unknown, fallback: number) => {
    const index = Number(value);
    return index === 0 || index === 1 || index === 2 ? index : fallback;
  };
  const sign = (value: unknown) => (Number(value) < 0 ? -1 : 1);
  // Y-up when the file says nothing, which is how every file without
  // `GlobalSettings` was already being read.
  const axes: [number, number, number] = [
    axis(source.coordAxis, 0),
    axis(source.upAxis, 1),
    axis(source.frontAxis, 2),
  ];
  const signs = [
    sign(source.coordAxisSign),
    sign(source.upAxisSign),
    sign(source.frontAxisSign),
  ];

  const basis = new Array(16).fill(0);
  basis[15] = 1;
  // Row r reads glTF axis r off FBX axis `axes[r]`; column-major storage puts
  // that entry at `column * 4 + row`.
  for (let row = 0; row < 3; row += 1) basis[axes[row] * 4 + row] = signs[row];

  const stated = Number(source.unitScaleFactor);
  const metersPerUnit = Number.isFinite(stated) && stated > 0 ? stated / 100 : 0.01;
  const identity = axes[0] === 0 && axes[1] === 1 && axes[2] === 2
    && signs.every((value) => value === 1);

  return {
    settings: source,
    metersPerUnit,
    basis,
    inverse: invertMat4(basis) || identityMat4(),
    rotation: quaternionFromBasis(basis),
    axes,
    identity,
  };
}

/**
 * The rotation as a quaternion, read straight off the basis.
 *
 * A signed permutation is orthonormal by construction, so the usual trace
 * extraction applies without normalizing anything first.
 */
function quaternionFromBasis(basis: number[]): number[] {
  const m = (row: number, column: number) => basis[column * 4 + row];
  const trace = m(0, 0) + m(1, 1) + m(2, 2);
  if (trace > 0) {
    const s = Math.sqrt(trace + 1) * 2;
    return [(m(2, 1) - m(1, 2)) / s, (m(0, 2) - m(2, 0)) / s, (m(1, 0) - m(0, 1)) / s, 0.25 * s];
  }
  if (m(0, 0) > m(1, 1) && m(0, 0) > m(2, 2)) {
    const s = Math.sqrt(1 + m(0, 0) - m(1, 1) - m(2, 2)) * 2;
    return [0.25 * s, (m(0, 1) + m(1, 0)) / s, (m(0, 2) + m(2, 0)) / s, (m(2, 1) - m(1, 2)) / s];
  }
  if (m(1, 1) > m(2, 2)) {
    const s = Math.sqrt(1 + m(1, 1) - m(0, 0) - m(2, 2)) * 2;
    return [(m(0, 1) + m(1, 0)) / s, 0.25 * s, (m(1, 2) + m(2, 1)) / s, (m(0, 2) - m(2, 0)) / s];
  }
  const s = Math.sqrt(1 + m(2, 2) - m(0, 0) - m(1, 1)) * 2;
  return [(m(0, 2) + m(2, 0)) / s, (m(1, 2) + m(2, 1)) / s, 0.25 * s, (m(1, 0) - m(0, 1)) / s];
}

/** Hamilton product, in glTF's `xyzw` order. */
export function multiplyQuaternion(left: ArrayLike<number>, right: ArrayLike<number>): number[] {
  const [lx, ly, lz, lw] = [left[0], left[1], left[2], left[3]];
  const [rx, ry, rz, rw] = [right[0], right[1], right[2], right[3]];
  return [
    lw * rx + lx * rw + ly * rz - lz * ry,
    lw * ry - lx * rz + ly * rw + lz * rx,
    lw * rz + lx * ry - ly * rx + lz * rw,
    lw * rw - lx * rx - ly * ry - lz * rz,
  ];
}
