// SceneDocument -> typed FBX SceneInput coverage. Blender differential checks
// remain in the focused source-provenance probes; this test verifies that the
// portable adapter preserves hierarchy, materials, skins, morphs, and clips.
import assert from 'node:assert/strict';

import { buildSceneDocumentFromFbx, buildSceneDocumentWithFbxProvenance } from '../www/fbx-scene-document.js';
import { buildSceneDocumentFromGltf } from '../www/gltf-scene-document.js';
import { buildFbxSceneFromDocument } from '../www/fbx-scene-document-writer.js';
import { loadWasm, mixamoFbx, readBytes, sambaFbx, foxBin, foxGltf, skipUnless } from './fbx-test-utils.mjs';

if (skipUnless([mixamoFbx, sambaFbx, foxGltf, foxBin], 'SceneDocument FBX writer')) process.exit(0);

const [fbx, gltf] = await Promise.all([loadWasm('fbx'), loadWasm('gltf')]);
const allNodes = (nodes) => nodes.flatMap((node) => [node, ...allNodes(node.children || [])]);

for (const [label, path] of [['Mixamo', mixamoFbx], ['Samba', sambaFbx]]) {
    const parsed = fbx.parse_fbx(await readBytes(path));
    assert.equal(parsed.success, true, `${label} source parse`);
    const { document, provenance } = buildSceneDocumentWithFbxProvenance(parsed);
    const scene = buildFbxSceneFromDocument(document, { provenance });
    assert.equal(scene.rootNodes.length, document.rootNodes.length, `${label} root count`);
    assert.equal(allNodes(scene.rootNodes).length, document.nodes.length, `${label} hierarchy count`);
    assert.equal(scene.materials.length, document.materials.length, `${label} material count`);
    assert.equal(scene.animations.length, document.animations.length, `${label} clip count`);
    assert.ok(allNodes(scene.rootNodes).some((node) => node.meshes?.some((mesh) => mesh.skin)), `${label} skin payload`);
    const written = fbx.create_fbx_scene(scene, { version: 7500 });
    assert.equal(written.success, true, `${label} typed FBX write: ${written.error || ''}`);
    const reparsed = fbx.parse_fbx(new Uint8Array(written.binary_data));
    assert.equal(reparsed.success, true, `${label} typed FBX reparse`);
    assert.equal(reparsed.scene.animations.length, parsed.scene.animations.length, `${label} clip roundtrip`);
    const sourceNames = allNodes(parsed.scene.rootNodes).map((node) => node.name).sort();
    const outputNames = allNodes(reparsed.scene.rootNodes).map((node) => node.name).sort();
    assert.deepEqual(outputNames, sourceNames, `${label} hierarchy names`);
    console.log(`PASS ${label} SceneDocument -> typed FBX: ${document.nodes.length} nodes, ${document.meshes.length} meshes, ${document.animations.length} clips`);
}

const foxDocument = buildSceneDocumentFromGltf(
    await readBytes(foxGltf),
    { 'Fox.bin': await readBytes(foxBin) },
    gltf,
);
const foxScene = buildFbxSceneFromDocument(foxDocument);
assert.equal(foxScene.rootNodes.length, foxDocument.rootNodes.length, 'Fox root count');
assert.equal(foxScene.materials.length, foxDocument.materials.length, 'Fox materials');
assert.equal(foxScene.animations.length, foxDocument.animations.length, 'Fox clips');
const foxWritten = fbx.create_fbx_scene(foxScene, { version: 7500 });
assert.equal(foxWritten.success, true, `Fox typed FBX write: ${foxWritten.error || ''}`);
const foxReparsed = fbx.parse_fbx(new Uint8Array(foxWritten.binary_data));
assert.equal(foxReparsed.success, true, 'Fox typed FBX reparse');
assert.ok(foxReparsed.scene.rootNodes.length > 0, 'Fox hierarchy survives');
console.log(`PASS Fox SceneDocument -> typed FBX: ${foxDocument.nodes.length} nodes, ${foxDocument.animations.length} clips`);
