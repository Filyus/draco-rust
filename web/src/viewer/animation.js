import { quat } from '../math.js';

/**
 * Animation sampling over the runtime scene.
 *
 * Deliberately free of WebGL: this evaluates channels into node TRS and is
 * exercised directly in Node by the parity tests.
 */

export function applyAnimation(scene, clipIndex, t) {
    const clip = scene.animations[clipIndex];
    if (!clip) return;
    // Reset each animated node before applying this frame. This is important
    // when switching clips, and for permissive FBX where a static rest matrix
    // is converted to TRS only once its Lcl animation is evaluated.
    const resetNodes = new Set();
    for (const channel of clip.channels) {
        const node = channel.node;
        const animationRest = node?.animationTrs || node?.restTrs;
        if (!node || resetNodes.has(node) || !animationRest) continue;
        node.trs.translation = [...animationRest.translation];
        node.trs.rotation = [...animationRest.rotation];
        node.trs.scale = [...animationRest.scale];
        resetNodes.add(node);
    }
    for (const channel of clip.channels) {
        const sampler = channel.sampler;
        const input = sampler.input;
        const output = sampler.output;
        if (!input || !output || input.length === 0) continue;

        let i0 = 0;
        for (let i = 0; i < input.length - 1; i++) {
            if (input[i] <= t && input[i + 1] >= t) { i0 = i; break; }
            if (t >= input[input.length - 1]) i0 = input.length - 1;
        }
        const t0 = input[i0];
        const t1 = input[Math.min(i0 + 1, input.length - 1)];
        let frac = (t1 > t0) ? (t - t0) / (t1 - t0) : 0;
        frac = Math.max(0, Math.min(1, frac));

        const interpolation = sampler.interpolation || 'LINEAR';
        const path = channel.path;
        applyChannel(
            channel.node,
            path,
            channel.targetCount,
            interpolation,
            output,
            i0,
            frac,
            t1 - t0,
        );
    }
}

function applyChannel(node, path, targetCount, interpolation, output, i0, frac, duration) {
    const out = path === 'weights' ? node.weights : node.trs[path];
    // Keep this guard at the render boundary so an unsupported future channel
    // cannot break the animation loop and leave the preview canvas stale.
    if (!out) return;

    if (path !== 'weights') {
        // A node animated through TRS must no longer use a static matrix. Such
        // an asset is invalid in strict glTF, but this gives the preview a
        // sensible result for permissively-authored files.
        node.localMatrix = null;
    }
    const components = path === 'weights' ? targetCount : path === 'rotation' ? 4 : 3;
    if (components <= 0) return;
    const stride = interpolation === 'CUBICSPLINE' ? components * 3 : components;
    const base0 = i0 * stride;
    const base1 = Math.min(i0 + 1, output.length / stride - 1) * stride;
    if (interpolation === 'STEP') {
        for (let k = 0; k < components; k++) out[k] = output[base0 + k];
        return;
    }
    if (interpolation === 'CUBICSPLINE') {
        // glTF cubic spline: out = (2t^3 - 3t^2 + 1) p0 + (t^3 - 2t^2 + t) m0 + (-2t^3 + 3t^2) p1 + (t^3 - t^2) m1
        // Layout per keyframe: [inTangent, value, outTangent]
        const p0 = base0 + components;
        const m0 = base0 + 2 * components;
        const p1 = base1 + components;
        const m1 = base1;
        for (let k = 0; k < components; k++) {
            out[k] = cubicSplineInterpolate(
                output[p0 + k],
                output[m0 + k],
                output[p1 + k],
                output[m1 + k],
                frac,
                duration,
            );
        }
        if (path === 'rotation') {
            const len = Math.hypot(out[0], out[1], out[2], out[3]) || 1;
            for (let k = 0; k < 4; k++) out[k] /= len;
        }
        return;
    }
    // LINEAR
    if (path === 'rotation') {
        const a = [output[base0], output[base0 + 1], output[base0 + 2], output[base0 + 3]];
        const b = [output[base1], output[base1 + 1], output[base1 + 2], output[base1 + 3]];
        quat.slerp(out, a, b, frac);
    } else {
        for (let k = 0; k < components; k++) {
            out[k] = output[base0 + k] + (output[base1 + k] - output[base0 + k]) * frac;
        }
    }
}

/** Evaluate one component of a glTF CUBICSPLINE animation segment. */
export function cubicSplineInterpolate(p0, outTangent0, p1, inTangent1, t, duration) {
    const t2 = t * t;
    const t3 = t2 * t;
    const c0 = 2 * t3 - 3 * t2 + 1;
    const c1 = t3 - 2 * t2 + t;
    const c2 = -2 * t3 + 3 * t2;
    const c3 = t3 - t2;
    return c0 * p0
        + c1 * duration * outTangent0
        + c2 * p1
        + c3 * duration * inTangent1;
}
