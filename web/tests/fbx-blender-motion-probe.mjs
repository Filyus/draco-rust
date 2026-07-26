// Differential Mixamo motion check against Blender's local FBX importer.
//
// This deliberately compares evaluated *world joint positions* rather than
// only sampler counts: a bind-pose-relative animation can look coherent while
// still being a different dance. Blender receives the exact source FBX and
// the viewer receives the same bytes through the WASM reader.
// Run: node tests/fbx-blender-motion-probe.mjs
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
import { here, loadWasm, mixamoFbx, readBytes } from './fbx-test-utils.mjs';

const BLENDER = process.env.BLENDER
    || 'C:/Program Files/Blender Foundation/Blender 4.5/blender.exe';
const MIXAMO = mixamoFbx;
if (!existsSync(BLENDER) || !existsSync(MIXAMO)) {
    console.log('SKIP Blender Mixamo motion probe (local Blender or fixture missing)');
    process.exit(0);
}

if (typeof globalThis.WebGL2RenderingContext === 'undefined') {
    globalThis.WebGL2RenderingContext = class {
        static REPEAT = 0x2901;
        static LINEAR_MIPMAP_LINEAR = 0x2703;
        static LINEAR = 0x2601;
    };
}

const times = [0, 4.2083335, 8.416667, 12.625, 16.833334];
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
bpy.ops.import_scene.fbx(filepath=${JSON.stringify(MIXAMO)})
scene=bpy.context.scene
armature=[obj for obj in scene.objects if obj.type=='ARMATURE'][0]
bones=${JSON.stringify(bones)}
times=${JSON.stringify(times)}
samples=[]
for seconds in times:
    frame=seconds*scene.render.fps/scene.render.fps_base+1
    scene.frame_set(int(math.floor(frame)), subframe=frame-math.floor(frame))
    samples.append([list(armature.pose.bones[name].matrix.to_translation()) for name in bones])
print('DRACO_BLENDER_JSON='+json.dumps(samples,separators=(',',':')))
`;
const blender = spawnSync(BLENDER, ['--background', '--python-expr', blenderScript], {
    encoding: 'utf8', maxBuffer: 4 * 1024 * 1024,
});
if (blender.status !== 0) throw new Error(`Blender FBX import failed:\n${blender.stderr || blender.stdout}`);
const line = blender.stdout.split(/\r?\n/).find((value) => value.startsWith('DRACO_BLENDER_JSON='));
if (!line) throw new Error('Blender did not emit Mixamo world-position samples');
const blenderSamples = JSON.parse(line.slice('DRACO_BLENDER_JSON='.length));

const fbx = await loadWasm('fbx');
const { buildSceneFromFbx } = await import(pathToFileURL(resolve(here, '..', 'src', 'mesh-loader.ts')));
const { Viewer } = await import(pathToFileURL(resolve(here, '..', 'www', 'viewer.js')));
const parsed = fbx.parse_fbx(await readBytes(MIXAMO));
const scene = await buildSceneFromFbx(parsed);
const probe = Object.create(Viewer.prototype);
probe.scene = scene;
probe.animation = { clipIndex: 0, time: 0 };

const viewerSamples = [];
for (const time of times) {
    if (!probe.seekAnimation(time)) throw new Error(`viewer seek failed at ${time}`);
    probe._updateWorldMatrices();
    viewerSamples.push(bones.map((name) => {
        const node = scene.nodes.find((candidate) => candidate.name === name);
        if (!node) throw new Error(`viewer is missing ${name}`);
        return [node.world[12], node.world[13], node.world[14]];
    }));
}

// Skin bind placement is allowed to differ from Blender by a single global
// root translation. Every descendant must otherwise follow the exact Blender
// chain at every sampled time.
const rootOffset = viewerSamples[0][0].map((value, index) => value - blenderSamples[0][0][index]);
let worst = { error: 0, time: 0, bone: '' };
for (let sample = 0; sample < times.length; sample += 1) {
    for (let bone = 0; bone < bones.length; bone += 1) {
        const expected = blenderSamples[sample][bone].map((value, index) => value + rootOffset[index]);
        const actual = viewerSamples[sample][bone];
        const error = Math.hypot(...actual.map((value, index) => value - expected[index]));
        if (error > worst.error) worst = { error, time: times[sample], bone: bones[bone] };
    }
}
if (worst.error > 0.05) {
    throw new Error(`Mixamo world-position mismatch: ${worst.bone} at ${worst.time}s differs by ${worst.error}`);
}
console.log(`PASS Mixamo Blender world motion: ${bones.length} joints × ${times.length} samples, max error=${worst.error.toExponential(2)}`);
