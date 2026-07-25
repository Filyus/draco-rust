// Verify the source-only semantic FBX path retains authored Model transform
// stacks. This deliberately does not involve SceneDocument: FBX provenance is
// kept outside the portable cross-format contract.
import assert from 'node:assert/strict';

import { loadWasm, mixamoFbx, readBytes, sambaFbx, skipUnless } from './fbx-test-utils.mjs';

if (skipUnless([mixamoFbx, sambaFbx], 'FBX source-provenance round-trip')) process.exit(0);

function allNodes(nodes) {
    return nodes.flatMap((node) => [node, ...allNodes(node.children || [])]);
}

function nodesByName(scene) {
    const nodes = allNodes(scene.rootNodes || []);
    return new Map(nodes.map((node) => [node.name, node]));
}

const fbx = await loadWasm('fbx');
for (const [label, path] of [['Mixamo', mixamoFbx], ['Samba', sambaFbx]]) {
    const bytes = await readBytes(path);
    const source = fbx.parse_fbx(bytes);
    assert.equal(source.success, true, `${label} source parse`);
    const expectedRootOrder = source.scene.rootNodes.map((node) => node.name);
    for (let repeat = 0; repeat < 8; repeat += 1) {
        const repeated = fbx.parse_fbx(bytes);
        assert.equal(repeated.success, true, `${label} repeated source parse ${repeat}`);
        assert.deepEqual(
            repeated.scene.rootNodes.map((node) => node.name),
            expectedRootOrder,
            `${label} source root order must be deterministic`,
        );
    }
    const written = fbx.create_fbx_scene(source.scene, { version: 7500 });
    assert.equal(written.success, true, `${label} semantic FBX write`);
    const output = fbx.parse_fbx(new Uint8Array(written.binary_data));
    assert.equal(output.success, true, `${label} semantic FBX reparse`);
    assert.deepEqual(
        output.scene.globalSettings,
        source.scene.globalSettings,
        `${label} source global coordinate/unit/time settings`,
    );

    const sourceNodes = nodesByName(source.scene);
    const outputNodes = nodesByName(output.scene);
    const sourceStacked = [...sourceNodes.values()].filter((node) => node.transformStack);
    assert.ok(sourceStacked.length > 0, `${label} must expose source Model transform stacks`);
    for (const sourceNode of sourceStacked) {
        const roundtripNode = outputNodes.get(sourceNode.name);
        assert.ok(roundtripNode, `${label} must retain node ${sourceNode.name}`);
        assert.deepEqual(
            roundtripNode.transformStack,
            sourceNode.transformStack,
            `${label} Model transform stack for ${sourceNode.name}`,
        );
    }
    assert.equal(output.scene.animations.length, source.scene.animations.length, `${label} clip count`);
    for (let index = 0; index < source.scene.animations.length; index += 1) {
        assert.equal(
            output.scene.animations[index].duration,
            source.scene.animations[index].duration,
            `${label} clip ${index} duration`,
        );
    }
    console.log(`PASS ${label} source FBX Model stacks: ${sourceStacked.length} nodes, ${source.scene.animations.length} clips`);
}
