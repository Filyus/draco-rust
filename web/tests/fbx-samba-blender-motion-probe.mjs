// Differential Samba Dancing check against Blender's local FBX importer.
// It verifies both the animated limb hierarchy and the Cluster TransformLink
// skin palette, which catches a pose that looks plausible while displacing
// skinned body parts.
import { existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { here, loadWasm, readBytes, sambaFbx } from './fbx-test-utils.mjs';

const blender = process.env.BLENDER || 'C:/Program Files/Blender Foundation/Blender 4.5/blender.exe';
if (!existsSync(blender) || !existsSync(sambaFbx)) {
    console.log('SKIP Blender Samba motion probe (local Blender or fixture missing)');
    process.exit(0);
}

const times = [0, 4.55, 9.1, 13.65, 18.2];
const bones = [
    'mixamorig:Hips', 'mixamorig:Spine', 'mixamorig:Spine1', 'mixamorig:Spine2',
    'mixamorig:LeftShoulder', 'mixamorig:LeftArm', 'mixamorig:LeftForeArm', 'mixamorig:LeftHand',
    'mixamorig:RightShoulder', 'mixamorig:RightArm', 'mixamorig:RightForeArm', 'mixamorig:RightHand',
    'mixamorig:LeftUpLeg', 'mixamorig:LeftLeg', 'mixamorig:LeftFoot',
    'mixamorig:RightUpLeg', 'mixamorig:RightLeg', 'mixamorig:RightFoot',
];
const blenderScript = `
import bpy,json,math
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.fbx(filepath=${JSON.stringify(sambaFbx)})
scene=bpy.context.scene
armature=[obj for obj in scene.objects if obj.type=='ARMATURE'][0]
bones=${JSON.stringify(bones)}
times=${JSON.stringify(times)}
samples=[]
for seconds in times:
    frame=seconds*scene.render.fps/scene.render.fps_base+1
    scene.frame_set(int(math.floor(frame)), subframe=frame-math.floor(frame))
    samples.append([[value for row in armature.pose.bones[name].matrix for value in row] for name in bones])
rest={name:[value for row in armature.pose.bones[name].bone.matrix_local for value in row] for name in bones}
print('DRACO_BLENDER_JSON='+json.dumps({'samples':samples,'rest':rest},separators=(',',':')))
`;
const result = spawnSync(blender, ['--background', '--python-expr', blenderScript], {
    encoding: 'utf8', maxBuffer: 4 * 1024 * 1024,
});
if (result.status !== 0) throw new Error(`Blender FBX import failed:\n${result.stderr || result.stdout}`);
const line = result.stdout.split(/\r?\n/).find((value) => value.startsWith('DRACO_BLENDER_JSON='));
if (!line) throw new Error('Blender did not emit Samba pose samples');
const expected = JSON.parse(line.slice('DRACO_BLENDER_JSON='.length));

const fbx = await loadWasm('fbx');
const { buildSceneFromFbx } = await import(pathToFileURL(resolve(here, '..', 'src', 'mesh-loader.ts')));
const { Viewer } = await import(pathToFileURL(resolve(here, '..', 'src', 'viewer.ts')));
const { invertMat4, multiplyMat4 } = await import(pathToFileURL(resolve(here, '..', 'src', 'mat4.ts')));
const parsed = fbx.parse_fbx(await readBytes(sambaFbx));
const scene = await buildSceneFromFbx(parsed);
const probe = Object.create(Viewer.prototype);
probe.scene = scene;
probe.animation = { clipIndex: 0, time: 0 };
let worstWorld = { error: 0 };
let worstSkin = { error: 0 };
const perBone = new Map(bones.map((bone) => [bone, { error: 0, time: 0 }]));
for (let sample = 0; sample < times.length; sample += 1) {
    if (!probe.seekAnimation(times[sample])) throw new Error(`viewer seek failed at ${times[sample]}`);
    probe._updateWorldMatrices();
    for (let index = 0; index < bones.length; index += 1) {
        const node = scene.nodes.find((candidate) => candidate.name === bones[index]);
        const blenderMatrix = expected.samples[sample][index];
        for (let column = 0; column < 4; column += 1) for (let row = 0; row < 4; row += 1) {
            const error = Math.abs(node.world[column * 4 + row] - blenderMatrix[row * 4 + column]);
            if (error > worstWorld.error) worstWorld = { error, bone: bones[index], time: times[sample] };
            const boneError = perBone.get(bones[index]);
            if (error > boneError.error) {
                boneError.error = error;
                boneError.time = times[sample];
            }
        }
    }
}
for (const skin of scene.skins) for (const joint of skin.joints) {
    const name = joint.node?.name;
    if (!expected.rest[name]) continue;
    const rest = expected.rest[name];
    const inverseRest = invertMat4(Float32Array.from(rest.flatMap((_, index) => rest[(index % 4) * 4 + Math.floor(index / 4)])));
    const actual = multiplyMat4(joint.node.world, joint.inverseBind);
    const expectedPalette = multiplyMat4(joint.node.world, inverseRest);
    for (let index = 0; index < 16; index += 1) {
        const error = Math.abs(actual[index] - expectedPalette[index]);
        if (error > worstSkin.error) worstSkin = { error, bone: name };
    }
}
if (worstWorld.error > 0.05) {
    const channels = (parsed.scene?.animations?.[0]?.channels || [])
        .filter((channel) => bones.includes(channel.nodeName))
        .map((channel) => ({ node: channel.nodeName, path: channel.path, first: Array.from(channel.sampler?.output || []).slice(0, 4) }));
    const largest = [...perBone.entries()]
        .sort((left, right) => right[1].error - left[1].error)
        .slice(0, 6)
        .map(([bone, value]) => ({ bone, ...value }));
    console.error(`Samba composition audit: ${JSON.stringify({ largest, channels })}`);
    throw new Error(`Samba world mismatch: ${worstWorld.bone} at ${worstWorld.time}s differs by ${worstWorld.error}`);
}
if (worstSkin.error > 0.05) throw new Error(`Samba skin bind mismatch: ${worstSkin.bone} differs by ${worstSkin.error}`);
console.log(`PASS Samba Blender motion + skin: ${bones.length} joints × ${times.length} samples, world=${worstWorld.error.toExponential(2)}, skin=${worstSkin.error.toExponential(2)}`);
