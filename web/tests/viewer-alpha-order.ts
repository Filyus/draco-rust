/**
 * What the frame does with alpha modes, and when it copies itself.
 *
 * Both rules were wrong on TransmissionOrderTest, the asset built to ask them.
 *
 * Nothing cut: a MASK material reached the shader with no cutoff at all, so the
 * transparent half of a cut-out texture drew as whatever colour sat under its
 * zero alpha. In that asset — and in every one that stores a cut-out the usual
 * way — that colour is black, and the masked row came out as three black
 * squares over the glass it was meant to be cut around.
 *
 * And nothing behind: the copy a transmissive surface refracts was taken once,
 * after the opaque half and before anything blended, so a blended surface was
 * in no transmissive lookup at all. The blended alpha vanished wherever the
 * glass covered it while the masked and opaque rows showed through correctly —
 * which is exactly the ordering the asset's README says a renderer must get
 * right.
 */
import assert from 'node:assert/strict';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

(globalThis as any).WebGL2RenderingContext = class {};

const here = dirname(fileURLToPath(import.meta.url));
const source = (name: string) => pathToFileURL(resolve(here, '..', 'src', 'viewer', name)).href;
const { alphaModeUniforms, capturePoints, deferredDrawOrder } = await import(source('renderer.ts'));

// --- Alpha modes -----------------------------------------------------------

assert.deepEqual(
    alphaModeUniforms({ alphaMode: 'MASK' }),
    { cutoff: 0.5, opaque: true },
    'MASK cuts at the spec default and is opaque either side of the cut',
);
assert.deepEqual(
    alphaModeUniforms({ alphaMode: 'MASK', alphaCutoff: 0.25 }),
    { cutoff: 0.25, opaque: true },
    'a stated cutoff is the one that cuts',
);
assert.deepEqual(
    alphaModeUniforms({ alphaMode: 'MASK', alphaCutoff: 0 }),
    { cutoff: 0, opaque: true },
    'a zero cutoff discards nothing, which is what the spec says it means',
);
assert.deepEqual(
    alphaModeUniforms({ alphaMode: 'BLEND' }),
    { cutoff: 0, opaque: false },
    'BLEND keeps every texel and the alpha each one carries',
);
assert.deepEqual(
    alphaModeUniforms({ alphaMode: 'OPAQUE', alphaCutoff: 0.5 }),
    { cutoff: 0, opaque: true },
    'OPAQUE ignores the alpha channel rather than cutting on it',
);
assert.deepEqual(
    alphaModeUniforms(undefined),
    { cutoff: 0, opaque: true },
    'a primitive with no material draws as opaque',
);

// --- Deferred order and capture points --------------------------------------

/** A scene of single-primitive meshes, each at its own distance down -Z. */
function sceneAt(entries: Array<{ z: number; material: any }>) {
    const identity = () => new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
    const meshes = entries.map(({ z }) => ({
        aabb: { min: [-1, -1, z - 0.5], max: [1, 1, z + 0.5] },
    }));
    const world = (index: number) => {
        const matrix = identity();
        void index;
        return matrix;
    };
    return {
        scene: {
            meshes,
            materials: entries.map(({ material }) => material),
            renderables: entries.map((_, index) => ({
                meshIndex: index,
                skinIndex: -1,
                node: { world: world(index), weights: undefined },
            })),
        },
        glResources: {
            primitives: entries.map((_, index) => [{ materialIndex: index, uploaded: {} }]),
        },
        // The eye sits on +Z, so a larger z is nearer and the far ones sort first.
        _eye: new Float32Array([0, 0, 10]),
    };
}

const BLEND = { alphaMode: 'BLEND' };
const GLASS = { alphaMode: 'OPAQUE', transmissionFactor: 1 };
const SOLID = { alphaMode: 'OPAQUE' };

// Named for TransmissionOrderTest's three columns: blended alpha behind the
// glass, level with it, and in front of it. The opaque backdrop never defers.
const host = sceneAt([
    { z: -3, material: BLEND },
    { z: 2, material: BLEND },
    { z: 0, material: GLASS },
    { z: -9, material: SOLID },
]);

const order = deferredDrawOrder(host);
assert.deepEqual(
    order.map((draw: any) => host.scene.renderables.indexOf(draw.renderable)),
    [0, 2, 1],
    'what waits is drawn back to front, and an opaque surface does not wait at all',
);

assert.deepEqual(
    capturePoints(order),
    [false, true, false],
    'the copy is taken after the blended surface behind the glass and before the glass',
);

// The ordinary scene — one stretch of transmissive surfaces, nothing blended
// among them — still copies the frame exactly once.
const plain = deferredDrawOrder(sceneAt([
    { z: 0, material: GLASS },
    { z: -1, material: GLASS },
    { z: -2, material: GLASS },
]));
assert.deepEqual(
    capturePoints(plain),
    [true, false, false],
    'a scene with nothing blended behind its glass pays for one copy',
);

// Nothing to defer means nothing to copy: a scene without transmission or
// blending must not blit the frame at all.
assert.deepEqual(capturePoints(deferredDrawOrder(sceneAt([{ z: 0, material: SOLID }]))), []);

console.log('viewer-alpha-order: OK');
