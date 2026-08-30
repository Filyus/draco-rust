/**
 * FBX `Lcl Rotation` keys are absolute values in the node's own rotation
 * basis. Where that basis is the node's authored `Lcl Rotation` -- what
 * `usesAuthoredModelTrs` marks -- multiplying the key by it applies the rest
 * rotation twice, so every animated bone is turned by its own rest amount at
 * every frame. On one measured character 336 of 341 animated bones carry a
 * non-zero rest rotation, median 25 degrees, compounding down chains 17 deep:
 * the mesh is torn at the first frame rather than drifting out of shape.
 *
 * Neither Mixamo probe in this repo can see it. In both, every animated bone's
 * rest rotation is the identity, so composing and replacing agree exactly.
 * These check the case they cannot.
 *
 * The expectation is stated as a relation between the adapter's own outputs
 * rather than against a hand-built quaternion: what has to hold is that the
 * rest rotation does not reach the result, and saying it that way does not
 * depend on reproducing the adapter's Euler convention.
 */
import assert from 'node:assert/strict';
import { adaptFbxAnimation } from '../src/fbx-animation-adapter.ts';

const DEG = Math.PI / 180;

function eulerXyz(x: number, y: number, z: number): number[] {
    const [cx, sx] = [Math.cos(x / 2), Math.sin(x / 2)];
    const [cy, sy] = [Math.cos(y / 2), Math.sin(y / 2)];
    const [cz, sz] = [Math.cos(z / 2), Math.sin(z / 2)];
    return [
        sx * cy * cz + cx * sy * sz,
        cx * sy * cz - sx * cy * sz,
        cx * cy * sz + sx * sy * cz,
        cx * cy * cz - sx * sy * sz,
    ];
}

/** Angle between two unit quaternions in degrees, sign-insensitive. */
function angleBetween(a: ArrayLike<number>, b: ArrayLike<number>): number {
    let dot = 0;
    for (let i = 0; i < 4; i++) dot += a[i] * b[i];
    return (2 * Math.acos(Math.min(1, Math.abs(dot)))) / DEG;
}

function adapt(restEuler: number[], keyEuler: number[], usesAuthoredModelTrs: boolean) {
    const node: any = {
        usesAuthoredModelTrs,
        animationTrs: {
            translation: [0, 0, 0],
            rotation: eulerXyz(restEuler[0], restEuler[1], restEuler[2]),
            scale: [1, 1, 1],
        },
        restTrs: { translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1] },
    };
    const clip = {
        name: 'take',
        duration: 1,
        channels: [{
            nodeId: 1,
            nodeName: 'bone',
            path: 'rotation',
            sampler: { input: [0], output: keyEuler, interpolation: 'linear' },
        }],
    };
    const adapted = adaptFbxAnimation(clip, new Map([[1, node]]), new Map([['bone', node]]));
    assert.ok(adapted && adapted.channels.length === 1, 'the channel should adapt');
    return adapted!.channels[0].sampler.output;
}

// The measured character's own Bip001-Pelvis rest rotation. An authored-TRS
// bone must receive the key it was given -- the same result a bone with no
// rest rotation gets for that key -- and not the key turned by its rest.
{
    const rest = [90 * DEG, 0, 90 * DEG];
    const key = [90 * DEG, 0, 90 * DEG];
    const withRest = adapt(rest, key, true);
    const withoutRest = adapt([0, 0, 0], key, true);
    const off = angleBetween(withRest, withoutRest);
    // A tenth of a degree, which is the samplers' `Float32Array` rounding and
    // three orders below the 120 degrees this is watching for.
    assert.ok(
        off < 0.1,
        `an authored-TRS bone's rest rotation must not reach its keys; off by ${off.toFixed(3)} degrees`,
    );
}

// A complex stack -- PreRotation and friends -- still needs the basis, and
// that path is deliberately unchanged: there the rest rotation must reach it.
{
    const rest = [90 * DEG, 0, 0];
    const key = [0, 0, 0];
    const composed = adapt(rest, key, false);
    const bare = adapt([0, 0, 0], key, false);
    assert.ok(
        angleBetween(composed, bare) > 89,
        'a complex-stack bone keeps its static basis; the two should differ by the rest rotation',
    );
}

// The case both Mixamo fixtures are made of: a zero rest rotation, where
// composing and replacing agree and no fixture can tell them apart.
{
    const key = [30 * DEG, 0, 0];
    assert.ok(
        angleBetween(adapt([0, 0, 0], key, false), adapt([0, 0, 0], key, true)) < 0.1,
        'with an identity basis the two agree, which is why no existing fixture sees this',
    );
}

// Consecutive keys must stay in one hemisphere. `q` and `-q` name the same
// rotation, so converting each Euler key independently can straddle the
// boundary, and a viewer interpolating linearly between them passes through
// zero: the bone snaps instead of turning. Measured on one character, 8 of
// 341 channels flipped, two within a quarter second of the clip's midpoint.
{
    // Two keys a degree either side of a full turn about X, which is where
    // the conversion changes sign: the same rotation comes back as -q.
    const keys = [359 * DEG, 0, 0, 1 * DEG, 0, 0];
    const node: any = {
        usesAuthoredModelTrs: true,
        animationTrs: { translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1] },
        restTrs: { translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1] },
    };
    const clip = {
        name: 'take',
        duration: 1,
        channels: [{
            nodeId: 1,
            nodeName: 'bone',
            path: 'rotation',
            sampler: { input: [0, 1], output: keys, interpolation: 'linear' },
        }],
    };
    const adapted = adaptFbxAnimation(clip, new Map([[1, node]]), new Map([['bone', node]]));
    const out = adapted!.channels[0].sampler.output;
    let dot = 0;
    for (let i = 0; i < 4; i++) dot += out[i] * out[4 + i];
    assert.ok(
        dot >= 0,
        `consecutive keys must share a hemisphere or a viewer interpolates through zero; dot was ${dot.toFixed(4)}`,
    );
    // And the two keys really are two degrees apart, not the 358 the raw
    // conversion would have made of them.
    const apart = angleBetween(out.subarray(0, 4), out.subarray(4, 8));
    assert.ok(apart < 5, `the pair should be a couple of degrees apart, got ${apart.toFixed(1)}`);
    assert.ok(
        Math.hypot(out[0] - out[4], out[1] - out[5], out[2] - out[6], out[3] - out[7]) < 0.1,
        'and close together as raw components, which is what a linear sampler reads',
    );
}

console.log('fbx-rotation-basis: OK');
