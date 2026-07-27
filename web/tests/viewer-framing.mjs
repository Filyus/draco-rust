/**
 * Opening framing: the scene bounds the camera and the grid are built from.
 *
 * All three inputs were wrong at the extremes of asset scale.
 *
 * A skinned mesh was first measured through its own node transform, which a
 * skinned draw divides out, so a Mixamo character authored in metres under a
 * 0.01-scaled armature reported a hundredth of what it renders as. Dropping the
 * node transform fixed that case and replaced it with the opposite one: a
 * skinned vertex lands at jointWorld * IBM * position, and treating that as the
 * identity is only right when the rig happens to make it so. Soldier.glb does
 * not — mesh and armature sit under one 0.01-scaled root, so the same character
 * rendered 1.8 units tall while its bind pose measured 183, and the camera was
 * framed a hundred times too far out to see it.
 *
 * And the fit distance had an absolute half-metre floor, which frames a
 * two-centimetre asset as a speck.
 *
 * So the fixtures below carry both rig shapes, with the inverse bind matrices
 * each one really has rather than an identity that stands in for them.
 */
import assert from 'node:assert/strict';

globalThis.WebGL2RenderingContext = class {};
const { Viewer } = await import('../src/viewer.ts');
const { buildSceneGrid } = await import('../src/viewer/renderer.ts');
const { composeMatrix, mat4, vec3 } = await import('../src/math.ts');

const SCALE = 0.01;

function scaledNode(scale) {
    const world = mat4.create();
    composeMatrix(world, [0, 0, 0], [0, 0, 0, 1], [scale, scale, scale]);
    return { name: 'node', trs: null, children: [], meshIndex: 0, skinIndex: 0, world };
}

/** A body-sized mesh box, the way a Mixamo export authors one: in metres. */
function characterMesh({ skinned }) {
    const attributes = skinned
        ? { POSITION: {}, JOINTS_0: {}, WEIGHTS_0: {} }
        : { POSITION: {} };
    return {
        name: 'Ch03',
        primitives: [{ attributes, mode: 4, materialIndex: -1 }],
        aabb: { min: [-0.73, 0, -0.18], max: [0.73, 1.66, 0.21] },
    };
}

/**
 * A rig where the armature is scaled and the bind matrices undo it.
 *
 * This is what Michelle.glb and mixamo.fbx both do: the joint's world carries
 * the 0.01, its inverse bind carries the 100, and their product is the identity
 * — which is why the bind-pose box is the rendered box for these.
 */
function compensatingInverseBind(scale) {
    const inverseBind = mat4.create();
    composeMatrix(inverseBind, [0, 0, 0], [0, 0, 0, 1], [1 / scale, 1 / scale, 1 / scale]);
    return inverseBind;
}

function sceneWith({ skinned, skinIndex = 0, inverseBind = compensatingInverseBind(SCALE) }) {
    const node = scaledNode(SCALE);
    return {
        nodes: [node],
        rootIndices: [0],
        meshes: [characterMesh({ skinned })],
        skins: [{ name: 'skin', joints: [{ node, inverseBind }] }],
        materials: [],
        textures: [],
        animations: [],
        renderables: [{ node, meshIndex: 0, skinIndex }],
        aabb: { min: [-0.5, -0.5, -0.5], max: [0.5, 0.5, 0.5] },
        warnings: [],
    };
}

function probeWith(scene) {
    const probe = Object.create(Viewer.prototype);
    probe.scene = scene;
    probe.canvas = { width: 800, height: 600 };
    probe._scratch = mat4.create();
    probe._basisRight = vec3.create();
    probe._basisUp = vec3.create();
    probe._basisForward = vec3.create();
    probe._pivotScratch = vec3.create();
    probe.camera = {
        target: vec3.create(),
        distance: 3,
        azimuth: 0,
        elevation: 0,
        fov: Math.PI / 4,
        near: 0.05,
        far: 1000,
        minDistance: 0.05,
        maxDistance: 1000,
    };
    return probe;
}

const rounded = (box) => ({
    min: box.min.map((v) => Number(v.toFixed(4))),
    max: box.max.map((v) => Number(v.toFixed(4))),
});

// A skinned mesh renders at jointWorld * IBM, with its node transform divided
// out. Under a scaled armature whose bind matrices undo the scale that product
// is the identity, so the bind-pose box is already world space and measures the
// character at its full 1.66 m rather than at 1.66 cm.
{
    const probe = probeWith(sceneWith({ skinned: true }));
    probe._updateSceneBounds();
    assert.deepEqual(rounded(probe.scene.aabb), {
        min: [-0.73, 0, -0.18],
        max: [0.73, 1.66, 0.21],
    });
}

// The other rig shape, and the one that made Soldier.glb invisible: mesh and
// armature under a single scaled root, so the bind matrices carry no
// compensating scale and jointWorld * IBM keeps the 0.01. The bind pose reads
// 1.66 and the draw puts 1.66 cm on screen, so the bounds have to say 1.66 cm.
// Asserting the bind-pose box here is what left the camera a hundred times too
// far out to see anything.
{
    const probe = probeWith(sceneWith({ skinned: true, inverseBind: mat4.create() }));
    probe._updateSceneBounds();
    assert.deepEqual(rounded(probe.scene.aabb), {
        min: [-0.0073, 0, -0.0018],
        max: [0.0073, 0.0166, 0.0021],
    });
}

// Every joint counts, not the first one. A vertex is a convex blend of its
// joints' results, so the bound is the union of the bind box under each
// jointWorld * IBM — a rig whose second joint sits a metre along +X can put
// geometry there, and a bound taken from joint zero alone would clip it.
{
    const scene = sceneWith({ skinned: true });
    const offsetWorld = mat4.create();
    composeMatrix(offsetWorld, [1, 0, 0], [0, 0, 0, 1], [SCALE, SCALE, SCALE]);
    scene.skins[0].joints.push({
        node: { ...scaledNode(SCALE), world: offsetWorld },
        inverseBind: compensatingInverseBind(SCALE),
    });
    const probe = probeWith(scene);
    probe._updateSceneBounds();
    assert.deepEqual(rounded(probe.scene.aabb), {
        min: [-0.73, 0, -0.18],
        max: [1.73, 1.66, 0.21],
    });
}

// The rigid path is untouched: a mesh with no skin is still measured through
// the node hierarchy that places it.
{
    const probe = probeWith(sceneWith({ skinned: false, skinIndex: -1 }));
    probe._updateSceneBounds();
    assert.deepEqual(rounded(probe.scene.aabb), {
        min: [-0.0073, 0, -0.0018],
        max: [0.0073, 0.0166, 0.0021],
    });
}

// The renderer only skins a primitive that carries influences and falls back to
// the node transform otherwise; the bounds follow the same test, so a skin
// index alone does not move them.
{
    const probe = probeWith(sceneWith({ skinned: false }));
    probe._updateSceneBounds();
    assert.deepEqual(rounded(probe.scene.aabb), {
        min: [-0.0073, 0, -0.0018],
        max: [0.0073, 0.0166, 0.0021],
    });
}

// A 2 cm asset — BoomBox is authored at that size — is framed by its own
// radius. The old absolute floor left the camera 25 model diameters away.
{
    const probe = probeWith(sceneWith({ skinned: false, skinIndex: -1 }));
    probe.scene.meshes[0].aabb = { min: [-0.0099, -0.0098, -0.0101], max: [0.0099, 0.0098, 0.0101] };
    probe.scene.nodes[0].world = mat4.create();
    probe._updateSceneBounds();
    probe._fitCameraToScene();

    const box = probe.scene.aabb;
    const radius = Math.hypot(
        box.max[0] - box.min[0],
        box.max[1] - box.min[1],
        box.max[2] - box.min[2],
    ) * 0.5;
    const fitFov = probe.camera.fov; // Vertical, on a landscape canvas.
    assert.ok(Math.abs(probe.camera.distance - (radius / Math.sin(fitFov * 0.5)) * 1.12) < 1e-9);
    // The whole model subtends a good part of the frame, rather than 4% of it.
    assert.ok(probe.camera.distance < radius * 4, `distance ${probe.camera.distance}`);
    // Dolly and clip limits scale with the model too, so zooming in still works.
    assert.ok(probe.camera.minDistance < probe.camera.distance * 0.01);
    assert.ok(probe.camera.near < radius * 0.01);
}

// The grid follows the same bounds: cells sized to the model, laid just under
// it. A fixed cell size or a fixed drop would swallow a centimetre-sized asset.
{
    const probe = probeWith(sceneWith({ skinned: false, skinIndex: -1 }));
    probe.scene.meshes[0].aabb = { min: [-0.0099, -0.0098, -0.0101], max: [0.0099, 0.0098, 0.0101] };
    probe.scene.nodes[0].world = mat4.create();
    probe._updateSceneBounds();

    let positions = null;
    probe.gl = {
        ARRAY_BUFFER: 0,
        STATIC_DRAW: 1,
        createBuffer: () => 'buffer',
        bindBuffer: () => {},
        bufferData: (_target, data) => { positions = data; },
    };
    buildSceneGrid(probe);

    const xs = [];
    const ys = [];
    for (let i = 0; i < positions.length; i += 3) {
        xs.push(positions[i]);
        ys.push(positions[i + 1]);
    }
    const uniqueX = [...new Set(xs.map((v) => Number(v.toFixed(6))))].sort((a, b) => a - b);
    const step = uniqueX[1] - uniqueX[0];
    assert.ok(step > 0.0005 && step < 0.005, `grid step ${step}`);
    // Under the model, by a hair on the model's own scale.
    const gridY = ys[0];
    assert.ok(gridY < probe.scene.aabb.min[1], `grid y ${gridY}`);
    assert.ok(Math.abs((probe.scene.aabb.min[1] - gridY) - step * 0.01) < step * 1e-4);
}

console.log('viewer framing ok');
