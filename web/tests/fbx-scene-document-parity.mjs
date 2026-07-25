import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { assertValidSceneDocument } from '../www/scene-document.js';
import { buildSceneDocumentFromFbx } from '../www/fbx-scene-document.js';
import { buildViewerSceneFromDocument } from '../www/scene-document-viewer.js';
import { here, loadFbxViewerAdapter, loadWasm, mixamoFbx, readBytes, sambaFbx } from './fbx-test-utils.mjs';

const { buildSceneFromFbx } = await loadFbxViewerAdapter();
const { Viewer } = await import(pathToFileURL(resolve(here, '..', 'www', 'viewer.js')));
const fbx = await loadWasm('fbx');

const bones = [
    'mixamorig:Hips', 'mixamorig:Spine', 'mixamorig:Spine1', 'mixamorig:Spine2',
    'mixamorig:LeftShoulder', 'mixamorig:LeftArm', 'mixamorig:LeftForeArm', 'mixamorig:LeftHand',
    'mixamorig:RightShoulder', 'mixamorig:RightArm', 'mixamorig:RightForeArm', 'mixamorig:RightHand',
    'mixamorig:LeftUpLeg', 'mixamorig:LeftLeg', 'mixamorig:LeftFoot',
    'mixamorig:RightUpLeg', 'mixamorig:RightLeg', 'mixamorig:RightFoot',
];

function sample(scene, time) {
    const probe = Object.create(Viewer.prototype);
    probe.scene = scene;
    probe.animation = { clipIndex: 0, time: 0 };
    assert.equal(probe.seekAnimation(time), true);
    probe._updateWorldMatrices();
    const byName = new Map(scene.nodes.map((node) => [node.name, node]));
    return bones.map((name) => {
        const node = byName.get(name);
        assert.ok(node, `missing ${name}`);
        return Array.from(node.world);
    });
}

function normalizeFbxWorldToMeters(matrix) {
    const output = [...matrix];
    output[12] *= 0.01;
    output[13] *= 0.01;
    output[14] *= 0.01;
    return output;
}

for (const [label, path] of [['Mixamo', mixamoFbx], ['Samba', sambaFbx]]) {
    const parsed = fbx.parse_fbx(await readBytes(path));
    if (!parsed.success || !parsed.scene) throw new Error(`${label} semantic parse failed`);
    const document = buildSceneDocumentFromFbx(parsed);
    const validation = assertValidSceneDocument(document);
    assert.equal(validation.capabilities.skins, true, `${label} must retain skins`);
    assert.equal(validation.capabilities.animations, true, `${label} must retain animation`);
    const portable = buildViewerSceneFromDocument(document);
    const direct = await buildSceneFromFbx(parsed);
    assert.equal(portable.animations.length, direct.animations.length, `${label} clip count`);
    assert.equal(portable.skins.length, direct.skins.length, `${label} skin count`);
    assert.equal(portable.skins[0].joints.length, direct.skins[0].joints.length, `${label} joint count`);
    const times = [0, portable.animations[0].duration * 0.25, portable.animations[0].duration * 0.5, portable.animations[0].duration * 0.75, portable.animations[0].duration];
    let worst = 0;
    for (const time of times) {
        const expected = sample(direct, time).map(normalizeFbxWorldToMeters);
        const actual = sample(portable, time);
        for (let bone = 0; bone < bones.length; bone += 1) {
            for (let component = 0; component < 16; component += 1) {
                if (!Number.isFinite(expected[bone][component]) || !Number.isFinite(actual[bone][component])) {
                    throw new Error(`${label} has a non-finite ${bones[bone]} matrix value at ${time}s component ${component}`);
                }
                worst = Math.max(worst, Math.abs(expected[bone][component] - actual[bone][component]));
            }
        }
    }
    assert.ok(worst < 5e-4, `${label} SceneDocument world drift ${worst}`);
    console.log(`PASS ${label} SceneDocument viewer parity: max world drift=${worst.toExponential(2)}`);
}
