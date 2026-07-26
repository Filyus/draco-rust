/** Small, dependency-free 4×4 matrix helpers shared by format adapters. */

import type { Trs } from './viewer-scene.ts';

export function identityMat4(): number[] {
  return [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
}

/** Multiply two column-major 4×4 matrices. */
export function multiplyMat4(
  left: ArrayLike<number> | null | undefined,
  right: ArrayLike<number> | null | undefined,
): number[] | null {
  if (!left || !right || left.length !== 16 || right.length !== 16) return null;
  const result: number[] = new Array(16);
  for (let column = 0; column < 4; column += 1) {
    for (let row = 0; row < 4; row += 1) {
      result[column * 4 + row] =
        left[row] * right[column * 4]
        + left[4 + row] * right[column * 4 + 1]
        + left[8 + row] * right[column * 4 + 2]
        + left[12 + row] * right[column * 4 + 3];
    }
  }
  return result;
}

/** Return the inverse of a column-major 4×4 matrix, or null when singular. */
export function invertMat4(values: ArrayLike<number> | null | undefined): number[] | null {
  if (!values || values.length !== 16) return null;
  const a = Array.from(values, Number);
  const inverse = identityMat4();
  for (let column = 0; column < 4; column += 1) {
    let pivot = column;
    for (let row = column + 1; row < 4; row += 1) {
      if (Math.abs(a[column * 4 + row]) > Math.abs(a[column * 4 + pivot])) pivot = row;
    }
    const value = a[column * 4 + pivot];
    if (Math.abs(value) < 1e-8) return null;
    if (pivot !== column) {
      // Values are column-major; exchange the two matrix rows across
      // every packed column.
      for (let packedColumn = 0; packedColumn < 4; packedColumn += 1) {
        const left = packedColumn * 4 + column;
        const right = packedColumn * 4 + pivot;
        [a[left], a[right]] = [a[right], a[left]];
        [inverse[left], inverse[right]] = [inverse[right], inverse[left]];
      }
    }
    for (let packedColumn = 0; packedColumn < 4; packedColumn += 1) {
      a[packedColumn * 4 + column] /= value;
      inverse[packedColumn * 4 + column] /= value;
    }
    for (let row = 0; row < 4; row += 1) {
      if (row === column) continue;
      const factor = a[column * 4 + row];
      for (let packedColumn = 0; packedColumn < 4; packedColumn += 1) {
        a[packedColumn * 4 + row] -= factor * a[packedColumn * 4 + column];
        inverse[packedColumn * 4 + row] -= factor * inverse[packedColumn * 4 + column];
      }
    }
  }
  return inverse;
}

/**
 * Split a column-major matrix into the TRS triple glTF and FBX both want.
 *
 * Scale comes from the basis vector lengths, so a matrix carrying shear
 * decomposes to the nearest rotation rather than being rejected: the importers
 * that call this need a usable pose out of whatever an exporter wrote.
 */
export function decomposeMat4(matrix: ArrayLike<number>): Trs {
  const scale = [Math.hypot(matrix[0], matrix[1], matrix[2]) || 1, Math.hypot(matrix[4], matrix[5], matrix[6]) || 1, Math.hypot(matrix[8], matrix[9], matrix[10]) || 1];
  const m00 = matrix[0] / scale[0], m01 = matrix[4] / scale[1], m02 = matrix[8] / scale[2];
  const m10 = matrix[1] / scale[0], m11 = matrix[5] / scale[1], m12 = matrix[9] / scale[2];
  const m20 = matrix[2] / scale[0], m21 = matrix[6] / scale[1], m22 = matrix[10] / scale[2];
  const trace = m00 + m11 + m22;
  let rotation: number[];
  if (trace > 0) { const s = Math.sqrt(trace + 1) * 2; rotation = [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s]; }
  else if (m00 > m11 && m00 > m22) { const s = Math.sqrt(1 + m00 - m11 - m22) * 2; rotation = [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]; }
  else if (m11 > m22) { const s = Math.sqrt(1 + m11 - m00 - m22) * 2; rotation = [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s]; }
  else { const s = Math.sqrt(1 + m22 - m00 - m11) * 2; rotation = [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s]; }
  const length = Math.hypot(...rotation) || 1;
  return { translation: [matrix[12], matrix[13], matrix[14]], rotation: rotation.map((value) => value / length), scale };
}

/** Copy a TRS triple so the caller can mutate it without aliasing the source. */
export function cloneTrs(trs: Trs): Trs {
  return { translation: [...trs.translation], rotation: [...trs.rotation], scale: [...trs.scale] };
}

/** The identity pose, as a fresh TRS triple. */
export function identityTrs(): Trs {
  return { translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1] };
}
