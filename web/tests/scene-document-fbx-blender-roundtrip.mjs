// Blender differential for the portable FBX writer boundary. Source FBX
// provenance is retained only as an optional adapter input; the output still
// comes from SceneDocument mesh/skin/clip data.
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { buildSceneDocumentWithFbxProvenance } from '../www/fbx-scene-document.js';
import { buildSceneDocumentFromGltf } from '../www/gltf-scene-document.js';
import { buildFbxSceneFromDocument } from '../www/fbx-scene-document-writer.js';
import { foxBin, foxGltf, here, loadWasm, mixamoFbx, readBytes, sambaFbx, skipUnless } from './fbx-test-utils.mjs';

const blender = process.env.BLENDER || 'C:/Program Files/Blender Foundation/Blender 4.5/blender.exe';
if (skipUnless([mixamoFbx, sambaFbx], 'SceneDocument FBX Blender round-trip') || !existsSync(blender)) process.exit(0);

const fbx = await loadWasm('fbx');
const scratch = await mkdtemp(resolve(tmpdir(), 'draco-scene-document-fbx-'));
const bones = [
    'mixamorig:Hips', 'mixamorig:Spine', 'mixamorig:Spine1', 'mixamorig:Spine2',
    'mixamorig:LeftArm', 'mixamorig:LeftForeArm', 'mixamorig:LeftHand',
    'mixamorig:RightArm', 'mixamorig:RightForeArm', 'mixamorig:RightHand',
    'mixamorig:LeftUpLeg', 'mixamorig:LeftLeg', 'mixamorig:LeftFoot',
    'mixamorig:RightUpLeg', 'mixamorig:RightLeg', 'mixamorig:RightFoot',
];
// Generate the script with paths embedded separately so Blender can reset and
// import each file in one deterministic process.
function blenderScript(source, output) {
    return `
import bpy,json,math
bones=${JSON.stringify(bones)}
def snap(path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.fbx(filepath=path)
    scene=bpy.context.scene
    armature=[obj for obj in scene.objects if obj.type=='ARMATURE'][0]
    fps=scene.render.fps/scene.render.fps_base
    duration=max(0.0,(scene.frame_end-1)/fps)
    times=[0,duration*0.25,duration*0.5,duration*0.75,duration]
    scene.frame_set(1)
    bpy.context.view_layer.update()
    samples=[]
    for seconds in times:
        frame=seconds*fps+1
        scene.frame_set(int(math.floor(frame)), subframe=frame-math.floor(frame))
        bpy.context.view_layer.update()
        samples.append({name:[value for row in armature.pose.bones[name].matrix for value in row] for name in bones if name in armature.pose.bones})
    return {'bones':list(armature.data.bones.keys()),'samples':samples,'meshes':len([obj for obj in scene.objects if obj.type=='MESH']),'duration':duration}
source=${JSON.stringify(source)}
output=${JSON.stringify(output)}
print('DRACO_BLENDER_JSON='+json.dumps({'source':snap(source),'output':snap(output)},separators=(',',':')))
`;
}

try {
    for (const [label, path] of [['Mixamo', mixamoFbx], ['Samba', sambaFbx]]) {
        const parsed = fbx.parse_fbx(await readBytes(path));
        const { document, provenance } = buildSceneDocumentWithFbxProvenance(parsed);
        const scene = buildFbxSceneFromDocument(document, { provenance });
        const written = fbx.create_fbx_scene(scene, { version: 7500 });
        assert.equal(written.success, true, `${label} writer`);
        const output = resolve(scratch, `${label}.fbx`);
        await writeFile(output, Buffer.from(written.binary_data));
        const result = spawnSync(blender, ['--background', '--python-expr', blenderScript(path, output)], { encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 });
        if (result.status !== 0) throw new Error(`Blender ${label} import failed:\n${result.stderr || result.stdout}`);
        const line = result.stdout.split(/\r?\n/).find((value) => value.startsWith('DRACO_BLENDER_JSON='));
        assert.ok(line, `${label} Blender output: ${result.stderr || result.stdout}`);
        const snapshots = JSON.parse(line.slice('DRACO_BLENDER_JSON='.length));
        assert.ok(snapshots.output.meshes > 0, `${label} exported meshes`);
        const names = Object.keys(snapshots.source.samples[0]).filter((name) => snapshots.output.samples[0][name]);
        let worst = 0;
        for (let sample = 0; sample < snapshots.source.samples.length; sample += 1) {
            for (const name of names) {
                const expected = snapshots.source.samples[sample][name];
                const actual = snapshots.output.samples[sample][name];
                for (let index = 0; index < 16; index += 1) worst = Math.max(worst, Math.abs(expected[index] - actual[index]));
            }
        }
        assert.ok(worst < 0.05, `${label} Blender world drift ${worst}`);
        console.log(`PASS ${label} SceneDocument -> FBX -> Blender: ${names.length} bones, world=${worst.toExponential(2)}`);
    }
    const foxDocument = buildSceneDocumentFromGltf(
        await readBytes(foxGltf),
        { 'Fox.bin': await readBytes(foxBin) },
        await loadWasm('gltf'),
    );
    const foxScene = buildFbxSceneFromDocument(foxDocument);
    const foxWritten = fbx.create_fbx_scene(foxScene, { version: 7500 });
    assert.equal(foxWritten.success, true, `Fox writer: ${foxWritten.error || ''}`);
    const foxOutput = resolve(scratch, 'Fox.fbx');
    await writeFile(foxOutput, Buffer.from(foxWritten.binary_data));
    const foxScript = `import bpy,json\nbpy.ops.wm.read_factory_settings(use_empty=True)\nbpy.ops.import_scene.fbx(filepath=${JSON.stringify(foxOutput)})\nprint('DRACO_FOX_BLENDER_JSON='+json.dumps({'meshes':len([obj for obj in bpy.context.scene.objects if obj.type=='MESH']),'armatures':len([obj for obj in bpy.context.scene.objects if obj.type=='ARMATURE'])},separators=(',',':')))\n`;
    const foxResult = spawnSync(blender, ['--background', '--python-expr', foxScript], { encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 });
    assert.equal(foxResult.status, 0, `Fox Blender import: ${foxResult.stderr || foxResult.stdout}`);
    const foxLine = foxResult.stdout.split(/\r?\n/).find((value) => value.startsWith('DRACO_FOX_BLENDER_JSON='));
    assert.ok(foxLine, 'Fox Blender output');
    const foxSnapshot = JSON.parse(foxLine.slice('DRACO_FOX_BLENDER_JSON='.length));
    assert.ok(foxSnapshot.meshes > 0, 'Fox exported mesh import');
    console.log(`PASS Fox SceneDocument -> FBX -> Blender: meshes=${foxSnapshot.meshes}, armatures=${foxSnapshot.armatures}`);
} finally {
    await rm(scratch, { recursive: true, force: true });
}
