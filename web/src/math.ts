/**
 * Minimal column-major vec3/quat/mat4 helpers used by the WebGL2 preview.
 *
 * Matrices are Float32Array(16) in glTF / WebGL column-major order:
 *   [m00 m10 m20 m30  m01 m11 m21 m31  m02 m12 m22 m32  m03 m13 m23 m33]
 *   where mXY means column X, row Y.
 */

/** Destinations are always the Float32Arrays the viewer keeps around. */
export type Mat4 = Float32Array;
export type Vec3 = Float32Array;
export type Quat = Float32Array;

/**
 * Read-only operand. Kept wider than the destinations because callers also
 * pass plain arrays straight out of glTF nodes and accessors.
 */
type Numbers = ArrayLike<number>;

export const mat4 = {
    create(): Mat4 {
        const m = new Float32Array(16);
        m[0] = m[5] = m[10] = m[15] = 1;
        return m;
    },

    identity(out: Mat4): Mat4 {
        out[0] = 1; out[1] = 0; out[2] = 0; out[3] = 0;
        out[4] = 0; out[5] = 1; out[6] = 0; out[7] = 0;
        out[8] = 0; out[9] = 0; out[10] = 1; out[11] = 0;
        out[12] = 0; out[13] = 0; out[14] = 0; out[15] = 1;
        return out;
    },

    copy(out: Mat4, a: Numbers): Mat4 {
        for (let i = 0; i < 16; i++) out[i] = a[i];
        return out;
    },

    /** out = a * b */
    multiply(out: Mat4, a: Numbers, b: Numbers): Mat4 {
        const a00 = a[0], a01 = a[1], a02 = a[2], a03 = a[3];
        const a10 = a[4], a11 = a[5], a12 = a[6], a13 = a[7];
        const a20 = a[8], a21 = a[9], a22 = a[10], a23 = a[11];
        const a30 = a[12], a31 = a[13], a32 = a[14], a33 = a[15];

        let b0 = b[0], b1 = b[1], b2 = b[2], b3 = b[3];
        out[0] = b0 * a00 + b1 * a10 + b2 * a20 + b3 * a30;
        out[1] = b0 * a01 + b1 * a11 + b2 * a21 + b3 * a31;
        out[2] = b0 * a02 + b1 * a12 + b2 * a22 + b3 * a32;
        out[3] = b0 * a03 + b1 * a13 + b2 * a23 + b3 * a33;

        b0 = b[4]; b1 = b[5]; b2 = b[6]; b3 = b[7];
        out[4] = b0 * a00 + b1 * a10 + b2 * a20 + b3 * a30;
        out[5] = b0 * a01 + b1 * a11 + b2 * a21 + b3 * a31;
        out[6] = b0 * a02 + b1 * a12 + b2 * a22 + b3 * a32;
        out[7] = b0 * a03 + b1 * a13 + b2 * a23 + b3 * a33;

        b0 = b[8]; b1 = b[9]; b2 = b[10]; b3 = b[11];
        out[8] = b0 * a00 + b1 * a10 + b2 * a20 + b3 * a30;
        out[9] = b0 * a01 + b1 * a11 + b2 * a21 + b3 * a31;
        out[10] = b0 * a02 + b1 * a12 + b2 * a22 + b3 * a32;
        out[11] = b0 * a03 + b1 * a13 + b2 * a23 + b3 * a33;

        b0 = b[12]; b1 = b[13]; b2 = b[14]; b3 = b[15];
        out[12] = b0 * a00 + b1 * a10 + b2 * a20 + b3 * a30;
        out[13] = b0 * a01 + b1 * a11 + b2 * a21 + b3 * a31;
        out[14] = b0 * a02 + b1 * a12 + b2 * a22 + b3 * a32;
        out[15] = b0 * a03 + b1 * a13 + b2 * a23 + b3 * a33;
        return out;
    },

    /** Inverse of a general 4x4; returns null if singular. */
    invert(out: Mat4, a: Numbers): Mat4 | null {
        const a00 = a[0], a01 = a[1], a02 = a[2], a03 = a[3];
        const a10 = a[4], a11 = a[5], a12 = a[6], a13 = a[7];
        const a20 = a[8], a21 = a[9], a22 = a[10], a23 = a[11];
        const a30 = a[12], a31 = a[13], a32 = a[14], a33 = a[15];

        const b00 = a00 * a11 - a01 * a10;
        const b01 = a00 * a12 - a02 * a10;
        const b02 = a00 * a13 - a03 * a10;
        const b03 = a01 * a12 - a02 * a11;
        const b04 = a01 * a13 - a03 * a11;
        const b05 = a02 * a13 - a03 * a12;
        const b06 = a20 * a31 - a21 * a30;
        const b07 = a20 * a32 - a22 * a30;
        const b08 = a20 * a33 - a23 * a30;
        const b09 = a21 * a32 - a22 * a31;
        const b10 = a21 * a33 - a23 * a31;
        const b11 = a22 * a33 - a23 * a32;

        let det =
            b00 * b11 - b01 * b10 + b02 * b09 + b03 * b08 - b04 * b07 + b05 * b06;
        if (Math.abs(det) < 1e-12) return null;
        det = 1.0 / det;

        out[0] = (a11 * b11 - a12 * b10 + a13 * b09) * det;
        out[1] = (a02 * b10 - a01 * b11 - a03 * b09) * det;
        out[2] = (a31 * b05 - a32 * b04 + a33 * b03) * det;
        out[3] = (a22 * b04 - a21 * b05 - a23 * b03) * det;
        out[4] = (a12 * b08 - a10 * b11 - a13 * b07) * det;
        out[5] = (a00 * b11 - a02 * b08 + a03 * b07) * det;
        out[6] = (a32 * b02 - a30 * b05 - a33 * b01) * det;
        out[7] = (a20 * b05 - a22 * b02 + a23 * b01) * det;
        out[8] = (a10 * b10 - a11 * b08 + a13 * b06) * det;
        out[9] = (a01 * b08 - a00 * b10 - a03 * b06) * det;
        out[10] = (a30 * b04 - a31 * b02 + a33 * b00) * det;
        out[11] = (a21 * b02 - a20 * b04 - a23 * b00) * det;
        out[12] = (a11 * b07 - a10 * b09 - a12 * b06) * det;
        out[13] = (a00 * b09 - a01 * b07 + a02 * b06) * det;
        out[14] = (a31 * b01 - a30 * b03 - a32 * b00) * det;
        out[15] = (a20 * b03 - a21 * b01 + a22 * b00) * det;
        return out;
    },

    transpose(out: Mat4, a: Numbers): Mat4 {
        if (out === a) {
            const a01 = a[1], a02 = a[2], a03 = a[3];
            const a12 = a[6], a13 = a[7];
            const a23 = a[11];
            out[1] = a[4]; out[2] = a[8]; out[3] = a[12];
            out[4] = a01; out[6] = a[9]; out[7] = a[13];
            out[8] = a02; out[9] = a12; out[11] = a[14];
            out[12] = a03; out[13] = a13; out[14] = a23;
        } else {
            out[0] = a[0]; out[1] = a[4]; out[2] = a[8]; out[3] = a[12];
            out[4] = a[1]; out[5] = a[5]; out[6] = a[9]; out[7] = a[13];
            out[8] = a[2]; out[9] = a[6]; out[10] = a[10]; out[11] = a[14];
            out[12] = a[3]; out[13] = a[7]; out[14] = a[11]; out[15] = a[15];
        }
        return out;
    },

    perspective(out: Mat4, fovy: number, aspect: number, near: number, far: number): Mat4 {
        const f = 1.0 / Math.tan(fovy * 0.5);
        const nf = 1.0 / (near - far);
        out[0] = f / aspect;
        out[1] = 0; out[2] = 0; out[3] = 0;
        out[4] = 0; out[5] = f; out[6] = 0; out[7] = 0;
        out[8] = 0; out[9] = 0;
        out[10] = (far + near) * nf;
        out[11] = -1;
        out[12] = 0; out[13] = 0;
        out[14] = 2 * far * near * nf;
        out[15] = 0;
        return out;
    },

    /** Build a view matrix looking from eye to target. */
    lookAt(out: Mat4, eye: Numbers, target: Numbers, up: Numbers): Mat4 {
        let zx = eye[0] - target[0];
        let zy = eye[1] - target[1];
        let zz = eye[2] - target[2];
        let len = Math.hypot(zx, zy, zz) || 1;
        zx /= len; zy /= len; zz /= len;

        let xx = up[1] * zz - up[2] * zy;
        let xy = up[2] * zx - up[0] * zz;
        let xz = up[0] * zy - up[1] * zx;
        len = Math.hypot(xx, xy, xz);
        if (len === 0) { xx = xy = xz = 0; } else { xx /= len; xy /= len; xz /= len; }

        const yx = zy * xz - zz * xy;
        const yy = zz * xx - zx * xz;
        const yz = zx * xy - zy * xx;

        out[0] = xx; out[1] = yx; out[2] = zx; out[3] = 0;
        out[4] = xy; out[5] = yy; out[6] = zy; out[7] = 0;
        out[8] = xz; out[9] = yz; out[10] = zz; out[11] = 0;
        out[12] = -(xx * eye[0] + xy * eye[1] + xz * eye[2]);
        out[13] = -(yx * eye[0] + yy * eye[1] + yz * eye[2]);
        out[14] = -(zx * eye[0] + zy * eye[1] + zz * eye[2]);
        out[15] = 1;
        return out;
    },

    /** Extract translation into out (vec3). */
    getTranslation(out: Vec3, a: Numbers): Vec3 {
        out[0] = a[12];
        out[1] = a[13];
        out[2] = a[14];
        return out;
    },
};

export const vec3 = {
    create(): Vec3 { return new Float32Array(3); },
    set(out: Vec3, x: number, y: number, z: number): Vec3 { out[0] = x; out[1] = y; out[2] = z; return out; },
    copy(out: Vec3, a: Numbers): Vec3 { out[0] = a[0]; out[1] = a[1]; out[2] = a[2]; return out; },
    add(out: Vec3, a: Numbers, b: Numbers): Vec3 { out[0] = a[0] + b[0]; out[1] = a[1] + b[1]; out[2] = a[2] + b[2]; return out; },
    sub(out: Vec3, a: Numbers, b: Numbers): Vec3 { out[0] = a[0] - b[0]; out[1] = a[1] - b[1]; out[2] = a[2] - b[2]; return out; },
    scale(out: Vec3, a: Numbers, s: number): Vec3 { out[0] = a[0] * s; out[1] = a[1] * s; out[2] = a[2] * s; return out; },
    lerp(out: Vec3, a: Numbers, b: Numbers, t: number): Vec3 {
        out[0] = a[0] + (b[0] - a[0]) * t;
        out[1] = a[1] + (b[1] - a[1]) * t;
        out[2] = a[2] + (b[2] - a[2]) * t;
        return out;
    },
    length(a: Numbers): number { return Math.hypot(a[0], a[1], a[2]); },
    normalize(out: Vec3, a: Numbers): Vec3 {
        const len = Math.hypot(a[0], a[1], a[2]) || 1;
        out[0] = a[0] / len; out[1] = a[1] / len; out[2] = a[2] / len;
        return out;
    },
    cross(out: Vec3, a: Numbers, b: Numbers): Vec3 {
        const ax = a[0], ay = a[1], az = a[2];
        const bx = b[0], by = b[1], bz = b[2];
        out[0] = ay * bz - az * by;
        out[1] = az * bx - ax * bz;
        out[2] = ax * by - ay * bx;
        return out;
    },
    transformMat4(out: Vec3, a: Numbers, m: Numbers): Vec3 {
        const x = a[0], y = a[1], z = a[2];
        let w = m[3] * x + m[7] * y + m[11] * z + m[15];
        w = w || 1;
        out[0] = (m[0] * x + m[4] * y + m[8] * z + m[12]) / w;
        out[1] = (m[1] * x + m[5] * y + m[9] * z + m[13]) / w;
        out[2] = (m[2] * x + m[6] * y + m[10] * z + m[14]) / w;
        return out;
    },
};

export const quat = {
    create(): Quat { return new Float32Array([0, 0, 0, 1]); },
    identity(out: Quat): Quat { out[0] = 0; out[1] = 0; out[2] = 0; out[3] = 1; return out; },
    copy(out: Quat, a: Numbers): Quat { out[0] = a[0]; out[1] = a[1]; out[2] = a[2]; out[3] = a[3]; return out; },
    /** Spherical linear interpolation; a, b are unit quats (x,y,z,w). */
    slerp(out: Quat, a: Numbers, b: Numbers, t: number): Quat {
        let bx = b[0], by = b[1], bz = b[2], bw = b[3];
        let dot = a[0] * bx + a[1] * by + a[2] * bz + a[3] * bw;
        if (dot < 0) { bx = -bx; by = -by; bz = -bz; bw = -bw; dot = -dot; }
        let s0: number, s1: number;
        if (dot > 0.9995) {
            // linear fallback
            s0 = 1 - t; s1 = t;
        } else {
            const theta = Math.acos(Math.min(1, Math.max(-1, dot)));
            const sinTheta = Math.sin(theta);
            s0 = Math.sin((1 - t) * theta) / sinTheta;
            s1 = Math.sin(t * theta) / sinTheta;
        }
        out[0] = s0 * a[0] + s1 * bx;
        out[1] = s0 * a[1] + s1 * by;
        out[2] = s0 * a[2] + s1 * bz;
        out[3] = s0 * a[3] + s1 * bw;
        return out;
    },
    normalize(out: Quat, a: Numbers): Quat {
        const len = Math.hypot(a[0], a[1], a[2], a[3]) || 1;
        out[0] = a[0] / len; out[1] = a[1] / len; out[2] = a[2] / len; out[3] = a[3] / len;
        return out;
    },
    /** Convert quaternion to a 4x4 rotation matrix. */
    toMat4(out: Mat4, q: Numbers): Mat4 {
        const x = q[0], y = q[1], z = q[2], w = q[3];
        const x2 = x + x, y2 = y + y, z2 = z + z;
        const xx = x * x2, xy = x * y2, xz = x * z2;
        const yy = y * y2, yz = y * z2, zz = z * z2;
        const wx = w * x2, wy = w * y2, wz = w * z2;
        out[0] = 1 - (yy + zz); out[1] = xy + wz; out[2] = xz - wy; out[3] = 0;
        out[4] = xy - wz; out[5] = 1 - (xx + zz); out[6] = yz + wx; out[7] = 0;
        out[8] = xz + wy; out[9] = yz - wx; out[10] = 1 - (xx + yy); out[11] = 0;
        out[12] = 0; out[13] = 0; out[14] = 0; out[15] = 1;
        return out;
    },
};

/** Build a 4x4 matrix from glTF TRS. All inputs optional, default to identity. */
export function composeMatrix(
    out: Mat4,
    translation?: Numbers | null,
    rotation?: Numbers | null,
    scale?: Numbers | null,
): Mat4 {
    const t = translation || [0, 0, 0];
    const r = rotation || [0, 0, 0, 1];
    const s = scale || [1, 1, 1];

    const x = r[0], y = r[1], z = r[2], w = r[3];
    const x2 = x + x, y2 = y + y, z2 = z + z;
    const xx = x * x2, xy = x * y2, xz = x * z2;
    const yy = y * y2, yz = y * z2, zz = z * z2;
    const wx = w * x2, wy = w * y2, wz = w * z2;

    const sx = s[0], sy = s[1], sz = s[2];

    out[0] = (1 - (yy + zz)) * sx;
    out[1] = (xy + wz) * sx;
    out[2] = (xz - wy) * sx;
    out[3] = 0;
    out[4] = (xy - wz) * sy;
    out[5] = (1 - (xx + zz)) * sy;
    out[6] = (yz + wx) * sy;
    out[7] = 0;
    out[8] = (xz + wy) * sz;
    out[9] = (yz - wx) * sz;
    out[10] = (1 - (xx + yy)) * sz;
    out[11] = 0;
    out[12] = t[0];
    out[13] = t[1];
    out[14] = t[2];
    out[15] = 1;
    return out;
}
