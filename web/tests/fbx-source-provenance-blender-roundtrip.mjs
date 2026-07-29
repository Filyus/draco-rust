// Differential source FBX -> semantic scene -> typed FBX writer probe.
// The portable SceneDocument is intentionally not involved: this validates
// source-only FBX provenance, including Model transform-stack properties and
// Blender's armature evaluation.
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

import { loadWasm, mixamoFbx, readBytes, sambaFbx, skipUnless } from './fbx-test-utils.ts';

const blender = process.env.BLENDER || 'C:/Program Files/Blender Foundation/Blender 4.5/blender.exe';
if (skipUnless([mixamoFbx, sambaFbx, blender], 'FBX source-provenance Blender round-trip')) process.exit(0);

const bones = [
    'mixamorig:Hips', 'mixamorig:Spine', 'mixamorig:Spine1', 'mixamorig:Spine2',
    'mixamorig:LeftShoulder', 'mixamorig:LeftArm', 'mixamorig:LeftForeArm', 'mixamorig:LeftHand',
    'mixamorig:RightShoulder', 'mixamorig:RightArm', 'mixamorig:RightForeArm', 'mixamorig:RightHand',
    'mixamorig:LeftUpLeg', 'mixamorig:LeftLeg', 'mixamorig:LeftFoot',
    'mixamorig:RightUpLeg', 'mixamorig:RightLeg', 'mixamorig:RightFoot',
];

function blenderSamples(path, times) {
    const script = `
import bpy,json,math
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.fbx(filepath=${JSON.stringify(path)})
scene=bpy.context.scene
armature=[obj for obj in scene.objects if obj.type=='ARMATURE'][0]
meshes=[obj for obj in scene.objects if obj.type=='MESH' and obj.find_armature()==armature]
if not meshes: meshes=[obj for obj in scene.objects if obj.type=='MESH']
bones=${JSON.stringify(bones)}
times=${JSON.stringify(times)}
def matrix(value): return [entry for row in value for entry in row]
def actions(): return sorted([[action.name, list(action.frame_range)] for action in bpy.data.actions])
scene.frame_set(1)
bpy.context.view_layer.update()
samples=[]
bounds={obj.name:[] for obj in meshes}
for seconds in times:
    frame=seconds*scene.render.fps/scene.render.fps_base+1
    scene.frame_set(int(math.floor(frame)), subframe=frame-math.floor(frame))
    bpy.context.view_layer.update()
    samples.append([matrix(armature.matrix_world @ armature.pose.bones[name].matrix) for name in bones])
    for mesh in meshes:
        evaluated=mesh.evaluated_get(bpy.context.evaluated_depsgraph_get())
        evaluated_mesh=evaluated.to_mesh()
        points=[evaluated.matrix_world @ vertex.co for vertex in evaluated_mesh.vertices]
        bounds[mesh.name].append([[min(point[axis] for point in points) for axis in range(3)],[max(point[axis] for point in points) for axis in range(3)]])
        evaluated.to_mesh_clear()
rest={name:matrix(armature.pose.bones[name].bone.matrix_local) for name in bones}
palette={name:matrix(armature.pose.bones[name].matrix @ armature.pose.bones[name].bone.matrix_local.inverted()) for name in bones}
print('DRACO_BLENDER_JSON='+json.dumps({'samples':samples,'rest':rest,'palette':palette,'bounds':bounds,'actions':actions(),'fps':scene.render.fps/scene.render.fps_base},separators=(',',':')))
`;
    const result = spawnSync(blender, ['--background', '--python-expr', script], {
        encoding: 'utf8', maxBuffer: 16 * 1024 * 1024,
    });
    if (result.status !== 0) throw new Error(`Blender FBX import failed:\n${result.stderr || result.stdout}`);
    const line = result.stdout.split(/\r?\n/).find((value) => value.startsWith('DRACO_BLENDER_JSON='));
    if (!line) throw new Error(`Blender FBX import did not emit samples:\n${result.stderr || result.stdout}`);
    return JSON.parse(line.slice('DRACO_BLENDER_JSON='.length));
}

function maxMatrixDrift(expected, actual, property) {
    let worst = { error: 0, sample: 0, bone: '', component: 0 };
    const expectedSamples = property === 'samples' ? expected.samples : [expected[property]];
    const actualSamples = property === 'samples' ? actual.samples : [actual[property]];
    for (let sample = 0; sample < expectedSamples.length; sample += 1) {
        for (let bone = 0; bone < bones.length; bone += 1) {
            const name = bones[bone];
            const left = property === 'samples' ? expectedSamples[sample][bone] : expectedSamples[sample][name];
            const right = property === 'samples' ? actualSamples[sample][bone] : actualSamples[sample][name];
            for (let component = 0; component < 16; component += 1) {
                const error = Math.abs(left[component] - right[component]);
                if (error > worst.error) worst = { error, sample, bone: name, component };
            }
        }
    }
    return worst;
}

function maxBoundsDrift(expected, actual) {
    let worst = { error: 0, mesh: '', sample: 0, extreme: 0, axis: 0 };
    assert.deepEqual(Object.keys(actual.bounds).sort(), Object.keys(expected.bounds).sort(), 'skinned mesh names');
    for (const [mesh, expectedSamples] of Object.entries(expected.bounds)) {
        for (let sample = 0; sample < expectedSamples.length; sample += 1) {
            for (let extreme = 0; extreme < 2; extreme += 1) for (let axis = 0; axis < 3; axis += 1) {
                const error = Math.abs(expectedSamples[sample][extreme][axis] - actual.bounds[mesh][sample][extreme][axis]);
                if (error > worst.error) worst = { error, mesh, sample, extreme, axis };
            }
        }
    }
    return worst;
}

const fbx = await loadWasm('fbx');
for (const [label, path] of [['Mixamo', mixamoFbx], ['Samba', sambaFbx]]) {
    const source = fbx.parse_fbx(await readBytes(path));
    assert.equal(source.success, true, `${label} source parse`);
    assert.equal(source.scene.animations.length, 1, `${label} source clip selection`);
    const duration = source.scene.animations[0].duration;
    const output = fbx.create_fbx_scene(source.scene, { version: 7500 });
    assert.equal(output.success, true, `${label} typed semantic write`);
    const temp = await mkdtemp(resolve(tmpdir(), 'draco-fbx-source-provenance-'));
    try {
        const outputPath = resolve(temp, `${label}.fbx`);
        await writeFile(outputPath, Buffer.from(output.binary_data));
        const times = [0, duration * 0.25, duration * 0.5, duration * 0.75, duration];
        const expected = blenderSamples(path, times);
        const actual = blenderSamples(outputPath, times);
        assert.equal(actual.actions.length, expected.actions.length, `${label} selected Blender clip count`);
        // The writer currently uses Blender's default TimeMode (25 fps),
        // while the source can declare a different display rate. FBX key
        // times are seconds-based KTime values, so compare action span in
        // seconds rather than its UI frame labels or layer-generated name.
        const sourceAction = expected.actions[0][1];
        const outputAction = actual.actions[0][1];
        const sourceSeconds = (sourceAction[1] - sourceAction[0]) / expected.fps;
        const outputSeconds = (outputAction[1] - outputAction[0]) / actual.fps;
        assert.ok(Math.abs(outputSeconds - sourceSeconds) < 1e-4, `${label} selected Blender clip duration`);
        const world = maxMatrixDrift(expected, actual, 'samples');
        const rest = maxMatrixDrift(expected, actual, 'rest');
        const palette = maxMatrixDrift(expected, actual, 'palette');
        const bounds = maxBoundsDrift(expected, actual);
        assert.ok(rest.error < 1e-4, `${label} bone rest drift ${rest.error} at ${rest.bone}, component ${rest.component}`);
        assert.ok(world.error < 1e-4, `${label} node world drift ${world.error} at ${world.bone}, sample ${world.sample}, component ${world.component}`);
        // Semantic bind matrices are f32 while Blender evaluates FBX matrix
        // arrays as doubles; retain the established cross-format tolerance.
        assert.ok(palette.error < 5e-4, `${label} skin palette drift ${palette.error} at ${palette.bone}, component ${palette.component}`);
        assert.ok(bounds.error < 5e-4, `${label} evaluated skinned bounds drift ${bounds.error} on ${bounds.mesh}, sample ${bounds.sample}`);
        console.log(`PASS ${label} source FBX -> semantic -> writer -> Blender: world=${world.error.toExponential(2)}, palette=${palette.error.toExponential(2)}, bounds=${bounds.error.toExponential(2)}`);
    } finally {
        await rm(temp, { recursive: true, force: true });
    }
}
