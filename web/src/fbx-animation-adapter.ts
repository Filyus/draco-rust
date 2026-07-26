/**
 * Adapt semantic FBX animation data to the format-neutral clip contract used
 * by viewer.js. This is an FBX import boundary: the viewer itself only sees
 * quaternion/TRS channels in the same shape it receives from glTF.
 *
 * Compatibility policies preserved here:
 * - Lcl Rotation keys are composed with the node's static FBX rotation basis.
 * - Lcl Translation is rebased to the bind-rest placement while preserving
 *   authored root-motion deltas.
 * - Value-only FBX cubic transform samples are sampled linearly because their
 *   separate tangents cannot be represented as glTF quaternion cubic tangents.
 */

import type { ViewerChannel, ViewerClip, ViewerNode } from './viewer-scene.ts';

/** One animation take as the semantic FBX decoder exposes it. */
type FbxClip = any;

/** Convert one FBX animation take into a viewer clip. */
export function adaptFbxAnimation(
    clip: FbxClip,
    nodeById: Map<unknown, ViewerNode>,
    nodeByName: Map<string, ViewerNode>,
): ViewerClip | null {
    const channels: ViewerChannel[] = [];
    for (const channel of clip.channels || []) {
        // Names are legal duplicates in FBX; use the object id emitted by
        // WASM first and retain names only for older parser results.
        const node = nodeById.get(channel.nodeId) || nodeByName.get(channel.nodeName);
        if (!node) continue;
        const sampler = channel.sampler || {};
        // Widened past the ArrayBuffer-backed default: the helpers below may
        // hand back the very array they were given.
        const input: Float32Array = Float32Array.from(sampler.input || []);
        let output: Float32Array = Float32Array.from(sampler.output || []);
        let interpolation = (sampler.interpolation || 'linear').toUpperCase() === 'CUBIC'
            ? 'CUBICSPLINE'
            : (sampler.interpolation || 'linear').toUpperCase();
        if (channel.path === 'morphweight') {
            const targetIndex = Number.isInteger(channel.morphTargetIndex)
                ? channel.morphTargetIndex
                : 0;
            const targetCount = node.weights?.length || targetIndex + 1;
            const values: number[] = [];
            const scalar = output;
            if (interpolation === 'CUBICSPLINE') {
                for (let frame = 0; frame < input.length; frame++) {
                    const inTangent = sampler.inTangents?.[frame] || 0;
                    const value = scalar[frame] || 0;
                    const outTangent = sampler.outTangents?.[frame] || 0;
                    for (const component of [inTangent, value, outTangent]) {
                        for (let target = 0; target < targetCount; target++) {
                            values.push(target === targetIndex ? component / 100 : 0);
                        }
                    }
                }
            } else {
                for (let frame = 0; frame < input.length; frame++) {
                    for (let target = 0; target < targetCount; target++) {
                        values.push(target === targetIndex ? (scalar[frame] || 0) / 100 : 0);
                    }
                }
            }
            channels.push({
                node,
                path: 'weights',
                targetCount,
                sampler: { input, output: Float32Array.from(values), interpolation },
            });
            continue;
        }
        if (channel.path === 'rotation') {
            output = composeFbxRotationBasis(node, input, output);
            // FBX Euler cubic tangents cannot be converted component-wise to
            // quaternion tangents. Keep the dense authored keys and sample
            // them linearly, matching the established compatibility policy.
            interpolation = 'LINEAR';
        } else if (channel.path === 'translation') {
            output = rebaseFbxTranslationToBindRest(node, input, output);
            // The semantic decoder exposes separate cubic tangents, while the
            // viewer expects interleaved glTF cubic data. The values above are
            // deliberately rebased, so use the established linear policy.
            if (interpolation === 'CUBICSPLINE') interpolation = 'LINEAR';
        } else if (interpolation === 'CUBICSPLINE') {
            output = interleaveFbxCubicTangents(channel, sampler, input, output);
        }
        channels.push({
            node,
            path: channel.path,
            sampler: { input, output, interpolation },
            targetCount: 3,
        });
    }
    if (channels.length === 0) return null;
    return {
        name: clip.name || `animation_${channels.length}`,
        duration: clip.duration || 0,
        channels,
    };
}

function composeFbxRotationBasis(node: ViewerNode, input: Float32Array, values: Float32Array) {
    const quatOut = new Float32Array(input.length * 4);
    // Lcl Rotation keys are absolute authored values in the static FBX
    // rotation basis, not deltas from the skin BindPose. Normalizing against
    // the first key would replace the opening dance pose with the T-pose.
    const staticBasis = node.animationTrs?.rotation || node.restTrs?.rotation || [0, 0, 0, 1];
    for (let frame = 0; frame < input.length; frame++) {
        const offset = frame * 3;
        const q = quatMultiply(staticBasis, eulerXyzToQuat(
            values[offset] || 0,
            values[offset + 1] || 0,
            values[offset + 2] || 0,
        ));
        quatOut[frame * 4] = q[0];
        quatOut[frame * 4 + 1] = q[1];
        quatOut[frame * 4 + 2] = q[2];
        quatOut[frame * 4 + 3] = q[3];
    }
    return quatOut;
}

function rebaseFbxTranslationToBindRest(node: ViewerNode, input: Float32Array, values: Float32Array) {
    // Plain Model TRS keys already use the same source space as their static
    // local transform. Retaining their absolute values is necessary for the
    // Cluster TransformLink skin basis (for example Samba Dancing's hips).
    if (node.usesAuthoredModelTrs) return values;
    // Preserve the raw FBX root-motion delta but anchor its first key to the
    // rest translation reconstructed from the skin BindPose.
    const rest = node.animationTrs?.translation || node.restTrs?.translation || [0, 0, 0];
    const rawRest = [values[0] || 0, values[1] || 0, values[2] || 0];
    const translated = new Float32Array(values.length);
    for (let frame = 0; frame < input.length; frame++) {
        const offset = frame * 3;
        translated[offset] = rest[0] + (values[offset] - rawRest[0]);
        translated[offset + 1] = rest[1] + (values[offset + 1] - rawRest[1]);
        translated[offset + 2] = rest[2] + (values[offset + 2] - rawRest[2]);
    }
    return translated;
}

function interleaveFbxCubicTangents(
    channel: FbxClip,
    sampler: FbxClip,
    input: Float32Array,
    output: Float32Array,
) {
    const components = channel.path === 'weights' ? (channel.targetCount || 1) : 3;
    const inTangents = sampler.inTangents || [];
    const outTangents = sampler.outTangents || [];
    const interleaved = new Float32Array(input.length * components * 3);
    for (let frame = 0; frame < input.length; frame++) {
        const source = frame * components;
        const target = frame * components * 3;
        for (let component = 0; component < components; component++) {
            interleaved[target + component] = inTangents[source + component] || 0;
            interleaved[target + components + component] = output[source + component] || 0;
            interleaved[target + components * 2 + component] = outTangents[source + component] || 0;
        }
    }
    return interleaved;
}

/** Euler XYZ (radians) -> quaternion [x,y,z,w], matching FBX's Rz·Ry·Rx. */
function eulerXyzToQuat(rx: number, ry: number, rz: number): number[] {
    const cx = Math.cos(rx * 0.5), sx = Math.sin(rx * 0.5);
    const cy = Math.cos(ry * 0.5), sy = Math.sin(ry * 0.5);
    const cz = Math.cos(rz * 0.5), sz = Math.sin(rz * 0.5);
    return [
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
        cx * cy * cz + sx * sy * sz,
    ];
}

function quatMultiply(a: ArrayLike<number>, b: ArrayLike<number>): number[] {
    return [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ];
}
