/**
 * What an FBX geometry without polygons has to become.
 *
 * FBX stores a curve the same way it stores a mesh: a Geometry object with
 * control points and no `PolygonVertexIndex`. A document mesh must carry at
 * least one primitive, so a scene holding any -- a chain along a spline, a
 * lattice -- used to fail validation as a whole and the file refused to open,
 * naming mesh indices that meant nothing to whoever authored it.
 */
import assert from 'node:assert/strict';

import { assertValidSceneDocument } from '../src/scene-document.ts';
import { buildSceneDocumentFromFbx } from '../src/fbx-scene-document.ts';

/** A triangle, with one morph target at half weight. */
function bodyMesh() {
    return {
        name: 'Body',
        positions: [0, 0, 0, 1, 0, 0, 1, 1, 0],
        normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
        indices: [0, 1, 2],
        material: 0,
        morphTargets: [{
            name: 'Smile',
            defaultWeight: 50,
            renderPointIndices: [0],
            renderPositionDeltas: [0, 1, 0],
        }],
    };
}

/** A curve: control points, no polygons, nothing to draw. */
function curveGeometry(name: string) {
    return { name, positions: [], indices: [], normals: [], uvs: [], controlPoints: [0, 0, 0, 0, 1, 0, 0, 2, 0] };
}

function parsedScene(rootNodes: unknown[]) {
    return {
        success: true,
        scene: {
            globalSettings: { unitScaleFactor: 1, upAxis: 1, upAxisSign: 1, frontAxis: 2, frontAxisSign: 1 },
            rootNodes,
            materials: [{ name: 'Skin' }],
            textures: [],
            animations: [],
        },
        warnings: [],
    };
}

// A curve beside a mesh: the mesh survives, the curve leaves a warning and no
// mesh on its node, and the node itself stays -- it can be a parent, and it is
// what an animation channel names.
{
    const parsed = parsedScene([{
        id: 1,
        name: 'Root',
        children: [
            { id: 2, name: 'Body', meshes: [bodyMesh()], children: [] },
            { id: 3, name: 'ChainCurve.015', meshes: [curveGeometry('ChainCurve.015')], children: [] },
        ],
    }]);
    const document = buildSceneDocumentFromFbx(parsed);
    assertValidSceneDocument(document);
    assert.equal(document.meshes.length, 1, 'the curve must not become a mesh');
    assert.equal(document.meshes[0].primitives.length, 1);
    assert.equal(document.nodes.length, 3, 'the curve keeps its node');
    const curveNode = document.nodes.find((node) => node.name === 'ChainCurve.015');
    assert.ok(curveNode, 'the curve node is missing');
    assert.equal(curveNode.mesh, undefined, 'the curve node must carry no mesh');
    assert.equal(document.nodes.find((node) => node.name === 'Body')?.mesh, 0);
    assert.ok(
        document.warnings.some((warning) => warning.includes('ChainCurve.015') && warning.includes('no polygons')),
        `the dropped curve must be named in the warnings: ${JSON.stringify(document.warnings)}`,
    );
}

// A curve ahead of a mesh on the same node: what binds to the node is the
// geometry that survived, with its own morph weights rather than the first
// geometry's.
{
    const parsed = parsedScene([{
        id: 1,
        name: 'Pendant',
        meshes: [curveGeometry('ChainCurve'), bodyMesh()],
        children: [],
    }]);
    const document = buildSceneDocumentFromFbx(parsed);
    assertValidSceneDocument(document);
    assert.equal(document.meshes.length, 1);
    assert.equal(document.nodes[0].mesh, 0, 'the surviving geometry binds to the node');
    assert.deepEqual(document.nodes[0].weights, [0.5], 'morph weights follow the bound geometry');
}

// A scene of nothing but curves stays a scene: no mesh, no error.
{
    const parsed = parsedScene([{ id: 1, name: 'Chain', meshes: [curveGeometry('Chain')], children: [] }]);
    const document = buildSceneDocumentFromFbx(parsed);
    assertValidSceneDocument(document);
    assert.equal(document.meshes.length, 0);
    assert.equal(document.nodes.length, 1);
}

console.log('PASS FBX curve geometry: polygon-less geometries are dropped, their nodes and neighbours kept');
