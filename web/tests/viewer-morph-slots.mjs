/**
 * The preview has four morph attribute slots but a mesh may declare many more
 * targets. Assets exported from a shape-key cycle (Stork.glb keys one of its 13
 * wing targets per frame) only animate in full when the slots follow the weights
 * instead of being nailed to targets 0..3.
 */
import assert from 'node:assert/strict';

globalThis.WebGL2RenderingContext = class {};
const { Viewer } = await import('../www/viewer.js');

const POSITION_LOCATIONS = [7, 8, 9, 10];
const NORMAL_LOCATIONS = [11, 12, 13, 14];
const TARGET_COUNT = 13;

/** Minimal GL stub tracking which buffer each attribute location points at. */
function createGlStub() {
    const bound = new Map();
    let arrayBuffer = null;
    return {
        bound,
        bindBuffer(target, buffer) {
            arrayBuffer = buffer;
        },
        enableVertexAttribArray(location) {
            bound.set(location, arrayBuffer);
        },
        disableVertexAttribArray(location) {
            bound.set(location, null);
        },
        vertexAttribPointer(location) {
            bound.set(location, arrayBuffer);
        },
    };
}

function createUploaded({ withNormals = false } = {}) {
    return {
        morphTargets: Array.from({ length: TARGET_COUNT }, (_, i) => ({
            position: { buffer: `pos${i}`, components: 3, componentType: 5126, normalized: false },
            normal: withNormals
                ? { buffer: `nrm${i}`, components: 3, componentType: 5126, normalized: false }
                : null,
        })),
        morphLocations: { positions: POSITION_LOCATIONS, normals: NORMAL_LOCATIONS },
        morphSlots: [-1, -1, -1, -1],
    };
}

function createProbe() {
    const probe = Object.create(Viewer.prototype);
    probe.gl = createGlStub();
    return probe;
}

function weightsAt(entries) {
    const weights = new Float32Array(TARGET_COUNT);
    for (const [index, value] of entries) weights[index] = value;
    return weights;
}

function slotBuffers(probe) {
    return POSITION_LOCATIONS.map((location) => probe.gl.bound.get(location) ?? null);
}

// A single late target must reach slot 0 — this is the case that used to leave
// the mesh frozen at its rest pose for most of the clip.
{
    const probe = createProbe();
    const uploaded = createUploaded();
    const count = probe._bindMorphTargets(uploaded, weightsAt([[11, 1]]));
    assert.equal(count, 1);
    assert.deepEqual(slotBuffers(probe), ['pos11', null, null, null]);
    assert.deepEqual(Array.from(probe._morphWeights), [1, 0, 0, 0]);
}

// Every target of a one-key-per-target cycle must become visible at some point.
{
    const probe = createProbe();
    const uploaded = createUploaded();
    const seen = new Set();
    for (let key = 0; key < TARGET_COUNT; key++) {
        const next = (key + 1) % TARGET_COUNT;
        for (const frac of [0, 0.5]) {
            const count = probe._bindMorphTargets(
                uploaded,
                weightsAt([[key, 1 - frac], [next, frac]]),
            );
            assert.ok(count >= 1 && count <= 2, `unexpected slot count ${count}`);
            for (const buffer of slotBuffers(probe)) {
                if (buffer) seen.add(buffer);
            }
        }
    }
    assert.equal(seen.size, TARGET_COUNT, `bound ${seen.size} of ${TARGET_COUNT} targets`);
}

// Overloaded frames keep the strongest weights, in descending order.
{
    const probe = createProbe();
    const uploaded = createUploaded({ withNormals: true });
    const count = probe._bindMorphTargets(
        uploaded,
        weightsAt([[2, 0.1], [4, 0.9], [6, -0.5], [8, 0.3], [12, 0.7]]),
    );
    assert.equal(count, 4);
    assert.deepEqual(slotBuffers(probe), ['pos4', 'pos12', 'pos6', 'pos8']);
    assert.deepEqual(
        NORMAL_LOCATIONS.map((location) => probe.gl.bound.get(location)),
        ['nrm4', 'nrm12', 'nrm6', 'nrm8'],
    );
    const staged = Array.from(probe._morphWeights).map((value) => Number(value.toFixed(6)));
    assert.deepEqual(staged, [0.9, 0.7, -0.5, 0.3]);
}

// A rest pose releases every slot so no stale delta survives the frame.
{
    const probe = createProbe();
    const uploaded = createUploaded();
    probe._bindMorphTargets(uploaded, weightsAt([[5, 1]]));
    const count = probe._bindMorphTargets(uploaded, weightsAt([]));
    assert.equal(count, 0);
    assert.deepEqual(slotBuffers(probe), [null, null, null, null]);
    assert.deepEqual(Array.from(probe._morphWeights), [0, 0, 0, 0]);
}

// Meshes without morph data must not touch the slots at all.
{
    const probe = createProbe();
    const count = probe._bindMorphTargets({ morphTargets: [] }, null);
    assert.equal(count, 0);
    assert.equal(probe.gl.bound.size, 0);
}

console.log('Viewer morph slot selection passed');
