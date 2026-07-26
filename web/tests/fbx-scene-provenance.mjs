import assert from 'node:assert/strict';

import { buildSceneDocumentWithFbxProvenance } from '../src/fbx-scene-document.ts';
import { cloneFbxSemanticScene } from '../src/fbx-scene-provenance.ts';
import { loadWasm, mixamoFbx, readBytes, sambaFbx } from './fbx-test-utils.mjs';

const fbx = await loadWasm('fbx');
for (const [label, path] of [['Mixamo', mixamoFbx], ['Samba', sambaFbx]]) {
    const parsed = fbx.parse_fbx(await readBytes(path));
    assert.equal(parsed.success, true, `${label} source parse`);
    const { document, provenance } = buildSceneDocumentWithFbxProvenance(parsed);
    assert.equal(document.nodes.length > 0, true, `${label} portable document`);
    assert.equal('sourceScene' in document, false, `${label} portable document must not retain FBX scene data`);
    assert.equal(provenance.format, 'fbx');
    assert.equal(provenance.coordinateSpace.axes, 'fbx-global-settings');
    assert.equal(provenance.coordinateSpace.unitScaleFactor, parsed.scene.globalSettings.unitScaleFactor, `${label} source UnitScaleFactor`);
    assert.deepEqual(provenance.globalSettings, parsed.scene.globalSettings, `${label} global settings sidecar`);
    assert.equal(provenance.animation.evaluator, 'fbx-viewer-bind-rest-v1');
    const clone = cloneFbxSemanticScene(provenance);
    assert.notEqual(clone, provenance.sourceScene, `${label} provenance export scene must detach`);
    assert.equal(clone.rootNodes.length, parsed.scene.rootNodes.length, `${label} source hierarchy`);
    assert.equal(clone.animations.length, parsed.scene.animations.length, `${label} source clips`);
}

console.log('FBX SceneDocument provenance is isolated and serializable');
