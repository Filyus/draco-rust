/**
 * The skinning shader holds one fixed array of joint matrices, so a joint index
 * in JOINTS_0 is a slot in it. A skin larger than that array used to be
 * truncated to its first slots, which does not drop the joints a mesh ignores —
 * it drops the ones it uses. One measured character carries 489 joints across
 * four skins while no primitive references more than 90 of them, spread to
 * index 415: seven of its thirteen meshes collapsed.
 *
 * Renumbering each primitive onto the joints it actually references removes the
 * class. These check that the renumbering is faithful — same joints, same
 * vertices, fewer slots — and that it steps aside when it would change nothing.
 */
import assert from 'node:assert/strict';

(globalThis as any).WebGL2RenderingContext = class {};
const { buildJointPalette } = await import('../src/viewer.ts');

/** A minimal JOINTS_0-only primitive stub; not a full ViewerPrimitive. */
const primitive = (joints: number[], componentType = 5123): any => ({
    attributes: {
        JOINTS_0: {
            bytes: (componentType === 5121 ? Uint8Array : Uint16Array).from(joints),
            componentType,
            components: 4,
            normalized: false,
            count: joints.length / 4,
        },
    },
});

const slotsOf = (result: any) => Array.from(result.attribute.bytes as Uint16Array);

// Two vertices reaching past the slot count, between them naming three joints.
{
    const result = buildJointPalette(primitive([300, 5, 300, 415, 415, 5, 5, 5]), 256);
    assert.deepEqual(
        Array.from(result.palette!),
        [300, 5, 415],
        'the palette should hold the joints referenced, in order of first appearance',
    );
    assert.deepEqual(
        slotsOf(result),
        [0, 1, 0, 2, 2, 1, 1, 1],
        'every index should become the slot standing for the same joint',
    );
    assert.equal(result.attribute!.count, 2);
    assert.equal(result.attribute!.components, 4);
}

// Already dense: renumbering would be the identity, so the file's own attribute
// is the one to bind and no palette is needed.
{
    const result = buildJointPalette(primitive([0, 1, 2, 3, 3, 2, 1, 0]), 256);
    assert.equal(result.palette, null, 'a dense attribute should be left alone');
    assert.equal(result.attribute!.componentType, 5123);
}

// An unsigned-byte attribute reads as plain indices, not as normalized values.
{
    const result = buildJointPalette(primitive([200, 200, 7, 7], 5121), 256);
    assert.deepEqual(Array.from(result.palette!), [200, 7]);
    assert.deepEqual(slotsOf(result), [0, 0, 1, 1]);
}

// More joints than slots in one primitive is the case nothing can draw, and the
// attribute is left as authored rather than silently renumbered into a lie.
{
    const many = Array.from({ length: 12 }, (_, i) => i * 10);
    const result = buildJointPalette(primitive(many), 4);
    assert.equal(result.palette, null, 'a primitive over the slot count keeps its indices');
}

// No skinning at all.
{
    const result = buildJointPalette({ attributes: {} } as any, 256);
    assert.equal(result.attribute, null);
    assert.equal(result.palette, null);
}

// Framing reads the same question the palette does. A skin is a rig, not a
// list of what one mesh is attached to: taking the union of the bind box under
// every joint framed a box hundreds of units across for a character built from
// sub-unit meshes, so the model rendered correctly and was a dot in an empty
// view. The joints a mesh names are the ones that can move it.
{
    const { updateWorldMatrices, updateSceneBounds } = await import('../src/viewer/scene-graph.ts');

    const node = (x: number) => ({
        name: `node_${x}`,
        trs: { translation: [x, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1] },
        children: [], world: new Float32Array(16), meshIndex: -1, skinIndex: -1,
    });
    // One joint under the mesh, one two hundred units away that nothing uses.
    const near = node(0);
    const far = node(200);
    const identity = Float32Array.from([1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1]);
    const skinned = primitive([0, 0, 0, 0]);
    skinned.attributes.WEIGHTS_0 = {
        bytes: Float32Array.from([1, 0, 0, 0]), componentType: 5126,
        components: 4, normalized: false, count: 1,
    };
    const mesh = {
        primitives: [skinned],
        aabb: { min: [-0.5, -0.5, -0.5], max: [0.5, 0.5, 0.5] },
    };
    const meshNode = { ...node(0), meshIndex: 0, skinIndex: 0 };
    const scene: any = {
        nodes: [near, far, meshNode],
        rootIndices: [0, 1, 2],
        meshes: [mesh],
        skins: [{ name: 'rig', joints: [
            { node: near, inverseBind: identity },
            { node: far, inverseBind: identity },
        ] }],
        renderables: [{ node: meshNode, meshIndex: 0, skinIndex: 0 }],
    };
    const host: any = { scene, _scratch: new Float32Array(16), _jointScratch: new Float32Array(16) };
    updateWorldMatrices(host);
    updateSceneBounds(host);

    const span = scene.aabb.max[0] - scene.aabb.min[0];
    assert.ok(
        span < 2,
        `framing should follow the joint the mesh uses, not the rig it belongs to; span was ${span}`,
    );
}

console.log('viewer-joint-palette: OK');
