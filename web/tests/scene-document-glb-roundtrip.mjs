import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { validateBytes } from 'gltf-validator';

import { buildSceneDocumentFromFbx } from '../www/fbx-scene-document.js';
import { buildSceneDocumentFromGltf } from '../www/gltf-scene-document.js';
import { serializeSceneDocumentToGlb } from '../www/scene-document-gltf.js';
import { invertMat4, multiplyMat4 } from '../www/mat4.js';
import { here, foxBin, foxGltf, loadFbxViewerAdapter, loadWasm, mixamoFbx, readBytes, sambaFbx } from './fbx-test-utils.mjs';

const { Viewer } = await import(pathToFileURL(resolve(here, '..', 'www', 'viewer.js')));
const { buildSceneFromGltf } = await import(pathToFileURL(resolve(here, '..', 'www', 'gltf-loader.js')));
const { buildSceneFromFbx } = await loadFbxViewerAdapter();
const gltf = await loadWasm('gltf');
const fbx = await loadWasm('fbx');
const blender = process.env.BLENDER || 'C:/Program Files/Blender Foundation/Blender 4.5/blender.exe';

const bones = [
    'mixamorig:Hips', 'mixamorig:Spine', 'mixamorig:Spine1', 'mixamorig:Spine2',
    'mixamorig:LeftShoulder', 'mixamorig:LeftArm', 'mixamorig:LeftForeArm', 'mixamorig:LeftHand',
    'mixamorig:RightShoulder', 'mixamorig:RightArm', 'mixamorig:RightForeArm', 'mixamorig:RightHand',
    'mixamorig:LeftUpLeg', 'mixamorig:LeftLeg', 'mixamorig:LeftFoot',
    'mixamorig:RightUpLeg', 'mixamorig:RightLeg', 'mixamorig:RightFoot',
];

function seek(scene, time) {
    const probe = Object.create(Viewer.prototype);
    probe.scene = scene;
    probe.animation = { clipIndex: 0, time: 0 };
    assert.equal(probe.seekAnimation(time), true);
    probe._updateWorldMatrices();
}

function maxSceneDrift(expected, actual) {
    const expectedNodes = new Map(expected.nodes.map((node) => [node.name, node]));
    const actualNodes = new Map(actual.nodes.map((node) => [node.name, node]));
    let worst = 0;
    for (const name of bones) {
        const left = expectedNodes.get(name);
        const right = actualNodes.get(name);
        assert.ok(left && right, `missing animated bone ${name}`);
        for (let index = 0; index < 16; index += 1) worst = Math.max(worst, Math.abs(left.world[index] - right.world[index]));
    }
    return worst;
}

function maxSkinPaletteDrift(expected, actual) {
    const expectedRenderable = expected.renderables.find((item) => item.skinIndex >= 0);
    const actualRenderable = actual.renderables.find((item) => item.skinIndex >= 0);
    assert.ok(expectedRenderable && actualRenderable, 'both scenes need a skinned renderable');
    const expectedJoints = new Map(expected.skins[expectedRenderable.skinIndex].joints.map((joint) => [joint.node.name, joint]));
    const actualJoints = new Map(actual.skins[actualRenderable.skinIndex].joints.map((joint) => [joint.node.name, joint]));
    let worst = 0;
    for (const name of bones) {
        const left = expectedJoints.get(name);
        const right = actualJoints.get(name);
        assert.ok(left && right, `missing skinned joint ${name}`);
        const leftPalette = skinPalette(expectedRenderable.node.world, left.node.world, left.inverseBind);
        const rightPalette = skinPalette(actualRenderable.node.world, right.node.world, right.inverseBind);
        for (let index = 0; index < 16; index += 1) worst = Math.max(worst, Math.abs(leftPalette[index] - rightPalette[index]));
    }
    return worst;
}

function skinPalette(meshWorld, jointWorld, inverseBind) {
    const inverseMesh = invertMat4(meshWorld);
    assert.ok(inverseMesh, 'mesh world must be invertible');
    return multiplyMat4(multiplyMat4(inverseMesh, jointWorld), inverseBind);
}

function normalizeFbxRuntimeToMeters(scene) {
    for (const node of scene.nodes) {
        for (const trs of [node.trs, node.restTrs, node.animationTrs]) {
            if (trs?.translation) for (let index = 0; index < 3; index += 1) trs.translation[index] *= 0.01;
        }
        if (node.localMatrix) for (const index of [12, 13, 14]) node.localMatrix[index] *= 0.01;
    }
    for (const clip of scene.animations) for (const channel of clip.channels) {
        if (channel.path !== 'translation') continue;
        for (let index = 0; index < channel.sampler.output.length; index += 1) channel.sampler.output[index] *= 0.01;
    }
    for (const skin of scene.skins) for (const joint of skin.joints) {
        for (const index of [12, 13, 14]) joint.inverseBind[index] *= 0.01;
    }
}

function blenderSamples(path, format, times) {
    if (!existsSync(blender)) return null;
    const importer = format === 'fbx'
        ? `bpy.ops.import_scene.fbx(filepath=${JSON.stringify(path)})`
        : `bpy.ops.import_scene.gltf(filepath=${JSON.stringify(path)})`;
    const script = `
import bpy,json,math
bpy.ops.wm.read_factory_settings(use_empty=True)
${importer}
scene=bpy.context.scene
armature=[obj for obj in scene.objects if obj.type=='ARMATURE'][0]
mesh=[obj for obj in scene.objects if obj.type=='MESH'][0]
bones=${JSON.stringify(bones)}
times=${JSON.stringify(times)}
frame_offset=${format === 'fbx' ? 1 : 0}
def matrix(value): return [entry for row in value for entry in row]
# Force Blender to instantiate the imported action/NLA state before the
# sampled seeks; otherwise a cold FBX import can leave its first evaluation
# on importer defaults.
scene.frame_set(1)
bpy.context.view_layer.update()
samples=[]
bounds=[]
for seconds in times:
    frame=seconds*scene.render.fps/scene.render.fps_base+frame_offset
    scene.frame_set(int(math.floor(frame)), subframe=frame-math.floor(frame))
    bpy.context.view_layer.update()
    samples.append([matrix(armature.pose.bones[name].matrix) for name in bones])
    evaluated=mesh.evaluated_get(bpy.context.evaluated_depsgraph_get())
    evaluated_mesh=evaluated.to_mesh()
    points=[evaluated.matrix_world @ vertex.co for vertex in evaluated_mesh.vertices]
    bounds.append([[min(point[axis] for point in points) for axis in range(3)],[max(point[axis] for point in points) for axis in range(3)]])
    evaluated.to_mesh_clear()
rest={name:matrix(armature.pose.bones[name].bone.matrix_local) for name in bones}
print('DRACO_BLENDER_JSON='+json.dumps({'samples':samples,'rest':rest,'bounds':bounds},separators=(',',':')))
`;
    const result = spawnSync(blender, ['--background', '--python-expr', script], {
        encoding: 'utf8', maxBuffer: 8 * 1024 * 1024,
    });
    if (result.status !== 0) throw new Error(`Blender ${format} import failed:\n${result.stderr || result.stdout}`);
    const line = result.stdout.split(/\r?\n/).find((value) => value.startsWith('DRACO_BLENDER_JSON='));
    if (!line) throw new Error(`Blender ${format} import did not emit pose samples`);
    return JSON.parse(line.slice('DRACO_BLENDER_JSON='.length));
}

function blenderPalette(pose, rest) {
    return multiplyMat4(toColumnMajor(pose), invertMat4(toColumnMajor(rest)));
}

function toColumnMajor(rowMajor) {
    return Float32Array.from(rowMajor.flatMap((_, index) => rowMajor[(index % 4) * 4 + Math.floor(index / 4)]));
}

function maxBlenderDrift(expected, actual) {
    let world = 0;
    let skin = 0;
    let worstWorld = { sample: 0, bone: '', component: 0 };
    for (let sample = 0; sample < expected.samples.length; sample += 1) {
        for (let bone = 0; bone < bones.length; bone += 1) {
            for (let component = 0; component < 16; component += 1) {
                const error = Math.abs(expected.samples[sample][bone][component] - actual.samples[sample][bone][component]);
                if (error > world) {
                    world = error;
                    worstWorld = { sample, bone: bones[bone], component };
                }
            }
            const expectedPalette = blenderPalette(expected.samples[sample][bone], expected.rest[bones[bone]]);
            const actualPalette = blenderPalette(actual.samples[sample][bone], actual.rest[bones[bone]]);
            for (let component = 0; component < 16; component += 1) skin = Math.max(skin, Math.abs(expectedPalette[component] - actualPalette[component]));
        }
    }
    return { world, skin, worstWorld };
}

function maxBlenderBoundsDrift(expected, actual) {
    // Blender's FBX importer applies a fixed source-axis placement to the
    // whole scene. Compare the authored root motion after that one constant
    // basis offset, just as the established Mixamo FBX differential probe.
    const rootOffset = expected.bounds[0][0].map((value, component) => actual.bounds[0][0][component] - value);
    let worst = 0;
    for (let sample = 0; sample < expected.bounds.length; sample += 1) {
        for (let extreme = 0; extreme < 2; extreme += 1) {
            for (let component = 0; component < 3; component += 1) {
                worst = Math.max(worst, Math.abs(expected.bounds[sample][extreme][component] + rootOffset[component] - actual.bounds[sample][extreme][component]));
            }
        }
    }
    return { worst, rootOffset };
}

async function assertValidGlb(bytes, name) {
    assert.deepEqual(Array.from(bytes.slice(0, 4)), [0x67, 0x6c, 0x54, 0x46], `${name} must be a GLB`);
    const report = await validateBytes(bytes, { uri: `${name}.glb` });
    assert.equal(report.issues.numErrors, 0, `${name} GLB validation: ${JSON.stringify(report.issues.messages)}`);
}

const foxResources = {
    'Fox.bin': await readBytes(foxBin),
    'Texture.png': new Uint8Array(await readFile(resolve(here, '..', '..', 'testdata', 'Fox', 'glTF', 'Texture.png'))),
};
const foxDocument = buildSceneDocumentFromGltf(await readBytes(foxGltf), foxResources, gltf);
const foxOutput = serializeSceneDocumentToGlb(foxDocument, gltf);
await assertValidGlb(foxOutput.binary, 'fox-scene-document');
const foxRoundtrip = buildSceneDocumentFromGltf(foxOutput.binary, {}, gltf);
assert.equal(foxRoundtrip.nodes.length, foxDocument.nodes.length, 'Fox hierarchy must survive GLB serialization');
assert.equal(foxRoundtrip.skins[0].joints.length, foxDocument.skins[0].joints.length, 'Fox skin must survive GLB serialization');
assert.equal(foxRoundtrip.animations.length, foxDocument.animations.length, 'Fox clips must survive GLB serialization');
assert.equal(foxRoundtrip.resources[0].mimeType, 'image/png', 'Fox texture bytes must survive GLB serialization');

for (const [label, path] of [['Mixamo', mixamoFbx], ['Samba', sambaFbx]]) {
    const parsed = fbx.parse_fbx(await readBytes(path));
    if (!parsed.success || !parsed.scene) throw new Error(`${label} semantic FBX parse failed`);
    const portable = buildSceneDocumentFromFbx(parsed);
    const output = serializeSceneDocumentToGlb(portable, gltf);
    await assertValidGlb(output.binary, `${label}-scene-document`);
    const expected = await buildSceneFromFbx(parsed);
    const actual = await buildSceneFromGltf(output.binary, {}, gltf);
    normalizeFbxRuntimeToMeters(expected);
    assert.equal(actual.animations.length, expected.animations.length, `${label} clip count`);
    assert.equal(actual.skins[0].joints.length, expected.skins[0].joints.length, `${label} skin joint count`);
    const duration = expected.animations[0].duration;
    assert.ok(Math.abs(actual.animations[0].duration - duration) < 1e-6, `${label} seconds-based duration`);
    let worstWorld = 0;
    let worstSkin = 0;
    for (const time of [0, duration * 0.25, duration * 0.5, duration * 0.75, duration]) {
        seek(expected, time);
        seek(actual, time);
        worstWorld = Math.max(worstWorld, maxSceneDrift(expected, actual));
        worstSkin = Math.max(worstSkin, maxSkinPaletteDrift(expected, actual));
    }
    assert.ok(worstWorld < 5e-4, `${label} world transform GLB drift ${worstWorld}`);
    assert.ok(worstSkin < 5e-4, `${label} skin palette GLB drift ${worstSkin}`);
    if (existsSync(blender)) {
        const temp = await mkdtemp(resolve(tmpdir(), 'draco-scene-document-'));
        try {
            const glbPath = resolve(temp, `${label}.glb`);
            await writeFile(glbPath, output.binary);
            const blenderTimes = [0, duration * 0.25, duration * 0.5, duration * 0.75, duration];
            const expectedBlender = blenderSamples(path, 'fbx', blenderTimes);
            const actualBlender = blenderSamples(glbPath, 'glb', blenderTimes);
            const blenderDrift = maxBlenderDrift(expectedBlender, actualBlender);
            const blenderBounds = maxBlenderBoundsDrift(expectedBlender, actualBlender);
            assert.ok(blenderBounds.worst < 5e-4, `${label} Blender evaluated mesh bounds GLB drift ${blenderBounds.worst}; root offset=${blenderBounds.rootOffset}; imported armature pose basis differs by world=${blenderDrift.world}, skin=${blenderDrift.skin}`);
            console.log(`PASS ${label} Blender FBX -> GLB evaluated mesh: bounds=${blenderBounds.worst.toExponential(2)}, root offset=${blenderBounds.rootOffset.map((value) => value.toExponential(2))} (armature-basis diagnostic world=${blenderDrift.world.toExponential(2)}, skin=${blenderDrift.skin.toExponential(2)})`);
        } finally {
            await rm(temp, { recursive: true, force: true });
        }
    }
    console.log(`PASS ${label} SceneDocument -> GLB: world=${worstWorld.toExponential(2)}, skin=${worstSkin.toExponential(2)}`);
}

console.log('SceneDocument GLB structural and animation roundtrip passed');
