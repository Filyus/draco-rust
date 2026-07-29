/**
 * The skinning shader blends joint matrices weighted by WEIGHTS_0 and does not
 * renormalize, so a vertex whose weights sum to nearly zero is placed at the
 * model origin and stretches its triangles across the scene. Quantized skins
 * reach the preview that way — kira.glb decodes (byte-identically to upstream
 * Draco) with 19 shirt vertices whose weights sum to 1.3e-29 — so the viewer
 * rebuilds the attribute rather than trusting it.
 */
import assert from 'node:assert/strict';

(globalThis as any).WebGL2RenderingContext = class {};
const { buildNormalizedWeightAttribute } = await import('../src/viewer.ts');

/** A minimal WEIGHTS_0-only primitive stub; not a full ViewerPrimitive. */
const primitive = (
    weights: number[],
    { componentType = 5126, components = 4 }: { componentType?: number; components?: number } = {},
): any => {
    const array = componentType === 5126 ? Float32Array : componentType === 5121
        ? Uint8Array : Uint16Array;
    const bytes = array.from(weights);
    return {
        attributes: {
            WEIGHTS_0: {
                bytes,
                componentType,
                components,
                normalized: componentType !== 5126,
                count: weights.length / components,
            },
        },
    };
};

const valuesOf = (result: any) => Array.from(
    new Float32Array(
        result.attribute.bytes.buffer,
        result.attribute.bytes.byteOffset,
        result.attribute.bytes.byteLength / 4,
    ),
).map((v: any) => Number(v.toFixed(6)));

// A well-formed attribute is passed through untouched, buffer identity and all,
// so ordinary skins pay nothing for this.
{
    const source = primitive([1, 0, 0, 0, 0.5, 0.5, 0, 0]);
    const result = buildNormalizedWeightAttribute(source);
    assert.equal(result.drifted, 0);
    assert.equal(result.attribute, source.attributes.WEIGHTS_0);
}

// Rounding within tolerance is left alone; float32 cannot represent every split
// exactly and those vertices are already in the right place.
{
    const result = buildNormalizedWeightAttribute(primitive([0.3333333, 0.3333333, 0.3333333, 0]));
    assert.equal(result.drifted, 0);
}

// A vertex that lost its influences to quantization must not collapse onto the
// origin: it stays rigidly bound to the joint JOINTS_0.x names.
{
    const result = buildNormalizedWeightAttribute(primitive([1.323e-29, 0, 0, 0]));
    assert.equal(result.drifted, 1);
    assert.deepEqual(valuesOf(result), [1, 0, 0, 0]);
}

// Partial drift is scaled back to unit sum, preserving the influence ratios.
{
    const result = buildNormalizedWeightAttribute(primitive([0.3, 0.1, 0, 0]));
    assert.equal(result.drifted, 1);
    assert.deepEqual(valuesOf(result), [0.75, 0.25, 0, 0]);
}

// One drifted vertex rebuilds the buffer, but sound vertices keep their values.
{
    const result = buildNormalizedWeightAttribute(primitive([1, 0, 0, 0, 0.5, 0.1, 0.1, 0.1]));
    assert.equal(result.drifted, 1);
    assert.deepEqual(valuesOf(result), [1, 0, 0, 0, 0.625, 0.125, 0.125, 0.125]);
    assert.equal(result.attribute!.componentType, 5126);
    assert.equal(result.attribute!.count, 2);
}

// Normalized integer storage is understood and normalizes to float weights.
{
    const result = buildNormalizedWeightAttribute(
        primitive([51, 17, 0, 0], { componentType: 5121 }),
    );
    assert.equal(result.drifted, 1);
    assert.deepEqual(valuesOf(result), [0.75, 0.25, 0, 0]);
}

// Shapes the skin contract does not describe are left for the upload path to
// reject or ignore, rather than being silently reinterpreted.
{
    assert.equal(buildNormalizedWeightAttribute({ attributes: {} } as any).attribute, null);
    const threeComponent = primitive([0.3, 0.1, 0], { components: 3 });
    assert.equal(
        buildNormalizedWeightAttribute(threeComponent).attribute,
        threeComponent.attributes.WEIGHTS_0,
    );
    const unsupported = primitive([0.3, 0.1, 0, 0]);
    unsupported.attributes.WEIGHTS_0.componentType = 5125;
    assert.equal(
        buildNormalizedWeightAttribute(unsupported).attribute,
        unsupported.attributes.WEIGHTS_0,
    );
}

console.log('Viewer skin weight normalization passed');
