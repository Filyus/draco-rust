/** Small, dependency-free 4×4 matrix helpers shared by format adapters. */

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
