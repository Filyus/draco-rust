/**
 * Morph deltas are sampled from an array texture, so the shader blends a bounded
 * number of targets per frame rather than the four a vertex attribute budget
 * used to allow. Assets exported from a shape-key cycle (Stork.glb keys one of
 * its 13 wing targets per frame) only animate in full when the staged layers
 * follow the weights instead of the first targets of the list.
 */
import assert from 'node:assert/strict';

globalThis.WebGL2RenderingContext = class {};
const { Viewer } = await import('../src/viewer.ts');

const TARGET_COUNT = 40;
const SHADER_LIMIT = 32;

function createMorph(layerCount = TARGET_COUNT, { rejected = [] } = {}) {
    return {
        texture: 'morph-texture',
        width: 64,
        stride: 1,
        layerCount,
        filled: Array.from({ length: layerCount }, (_, i) => !rejected.includes(i)),
    };
}

function weightsAt(entries, length = TARGET_COUNT) {
    const weights = new Float32Array(length);
    for (const [index, value] of entries) weights[index] = value;
    return weights;
}

function staged(probe, count) {
    return Array.from({ length: count }, (_, slot) => [
        probe._morphLayers[slot],
        Number(probe._morphWeights[slot].toFixed(6)),
    ]);
}

const probe = Object.create(Viewer.prototype);

// A single late target must reach the shader — this is the case that used to
// leave the mesh frozen at its rest pose for most of the clip.
{
    const count = probe._selectMorphTargets(createMorph(13), weightsAt([[11, 1]]));
    assert.equal(count, 1);
    assert.deepEqual(staged(probe, 1), [[11, 1]]);
}

// Every target of a one-key-per-target cycle must become visible at some point.
{
    const morph = createMorph(13);
    const seen = new Set();
    for (let key = 0; key < 13; key++) {
        const next = (key + 1) % 13;
        for (const frac of [0, 0.5]) {
            const count = probe._selectMorphTargets(
                morph,
                weightsAt([[key, 1 - frac], [next, frac]]),
            );
            assert.ok(count >= 1 && count <= 2, `unexpected active count ${count}`);
            for (const [layer] of staged(probe, count)) seen.add(layer);
        }
    }
    assert.equal(seen.size, 13, `blended ${seen.size} of 13 targets`);
}

// More than four targets blend at once — the whole point of the texture path.
{
    const count = probe._selectMorphTargets(
        createMorph(),
        weightsAt([[2, 0.1], [4, 0.9], [6, -0.5], [8, 0.3], [12, 0.7], [30, 0.2]]),
    );
    assert.equal(count, 6);
    assert.deepEqual(staged(probe, count), [
        [4, 0.9], [12, 0.7], [6, -0.5], [8, 0.3], [30, 0.2], [2, 0.1],
    ]);
}

// Past the shader loop bound the strongest weights win, in descending order.
{
    const entries = Array.from({ length: TARGET_COUNT }, (_, i) => [i, (i + 1) / TARGET_COUNT]);
    const count = probe._selectMorphTargets(createMorph(), weightsAt(entries));
    assert.equal(count, SHADER_LIMIT);
    const layers = staged(probe, count).map(([layer]) => layer);
    assert.equal(layers[0], TARGET_COUNT - 1);
    assert.equal(layers[SHADER_LIMIT - 1], TARGET_COUNT - SHADER_LIMIT);
}

// Targets whose accessor was rejected upstream are all-zero layers, so they must
// not consume a slot.
{
    const count = probe._selectMorphTargets(
        createMorph(8, { rejected: [1, 2] }),
        weightsAt([[1, 1], [2, 1], [5, 0.4]]),
    );
    assert.equal(count, 1);
    assert.deepEqual(staged(probe, 1), [[5, 0.4]]);
}

// A rest pose clears the staged arrays so no stale delta survives the frame.
{
    probe._selectMorphTargets(createMorph(), weightsAt([[5, 1]]));
    const count = probe._selectMorphTargets(createMorph(), weightsAt([]));
    assert.equal(count, 0);
    assert.equal(probe._morphWeights.some((weight) => weight !== 0), false);
    assert.equal(probe._morphLayers.some((layer) => layer !== 0), false);
}

// Meshes without morph data stage nothing at all.
{
    assert.equal(probe._selectMorphTargets(null, weightsAt([[0, 1]])), 0);
    assert.equal(probe._selectMorphTargets(createMorph(), null), 0);
}

console.log('Viewer morph target selection passed');
