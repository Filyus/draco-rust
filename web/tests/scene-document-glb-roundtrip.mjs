import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

import { validateBytes } from 'gltf-validator';

import { buildSceneDocumentFromFbx } from '../src/fbx-scene-document.ts';
import { buildSceneDocumentFromGltf } from '../src/gltf-scene-document.ts';
import { cloneSceneDocument } from '../src/scene-document.ts';
import { sniffMime } from '../src/scene-resources.ts';
import { lowerSceneDocumentToGltf, serializeSceneDocumentToGlb } from '../src/scene-document-gltf.ts';
import { invertMat4, multiplyMat4 } from '../src/mat4.ts';
import { here, foxBin, foxGltf, loadFbxViewerAdapter, loadWasm, mixamoFbx, readBytes, sambaFbx } from './fbx-test-utils.ts';

const { Viewer } = await import(pathToFileURL(resolve(here, '..', 'src', 'viewer.ts')));
const { buildSceneFromGltf } = await import(pathToFileURL(resolve(here, '..', 'src', 'gltf-loader.ts')));
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
meshes=[obj for obj in scene.objects if obj.type=='MESH' and obj.find_armature()==armature]
if not meshes: meshes=[obj for obj in scene.objects if obj.type=='MESH']
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
bounds={mesh.name:[] for mesh in meshes}
for seconds in times:
    frame=seconds*scene.render.fps/scene.render.fps_base+frame_offset
    scene.frame_set(int(math.floor(frame)), subframe=frame-math.floor(frame))
    bpy.context.view_layer.update()
    samples.append([matrix(armature.pose.bones[name].matrix) for name in bones])
    for mesh in meshes:
        evaluated=mesh.evaluated_get(bpy.context.evaluated_depsgraph_get())
        evaluated_mesh=evaluated.to_mesh()
        points=[evaluated.matrix_world @ vertex.co for vertex in evaluated_mesh.vertices]
        bounds[mesh.name].append([[min(point[axis] for point in points) for axis in range(3)],[max(point[axis] for point in points) for axis in range(3)]])
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
    // Blender's FBX importer applies one fixed source-axis placement to the
    // imported scene. Keep that established basis correction, but derive it
    // from a stable named mesh instead of whichever mesh a HashMap happened
    // to expose first.
    let worst = 0;
    let worstCase = { mesh: '', sample: 0, extreme: 0, component: 0 };
    const expectedMeshes = Object.keys(expected.bounds).sort();
    const actualMeshes = Object.keys(actual.bounds).sort();
    assert.deepEqual(actualMeshes, expectedMeshes, 'Blender evaluated mesh identity');
    const basisMesh = expectedMeshes.find((name) => name.endsWith('_Surface') || name.endsWith('Surface')) ?? expectedMeshes[0];
    const rootOffset = expected.bounds[basisMesh][0][0].map((value, component) =>
        actual.bounds[basisMesh][0][0][component] - value);
    for (const mesh of expectedMeshes) {
        for (let sample = 0; sample < expected.bounds[mesh].length; sample += 1) {
            for (let extreme = 0; extreme < 2; extreme += 1) {
                for (let component = 0; component < 3; component += 1) {
                    const error = Math.abs(expected.bounds[mesh][sample][extreme][component] + rootOffset[component] - actual.bounds[mesh][sample][extreme][component]);
                    if (error > worst) {
                        worst = error;
                        worstCase = { mesh, sample, extreme, component };
                    }
                }
            }
        }
    }
    return { worst, rootOffset, ...worstCase };
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

// Texture-info settings and texture source extensions belong to the portable
// contract, not to the FBX or browser viewer boundary.  Exercise every core
// material slot through the typed glTF lowering/import path.
const texturedDocument = cloneSceneDocument(foxDocument);
// Both alternate sources are real files rather than declared types over
// borrowed bytes. Sniffing them is part of what this checks: a resource whose
// MIME type was guessed from its content and one whose type was stated have to
// reach the same extension, or the writer and the reader disagree about a file
// nobody looked inside.
const ktxResource = texturedDocument.resources.length;
texturedDocument.resources.push({
    name: 'sample.ktx2',
    mimeType: 'image/ktx2',
    bytes: new Uint8Array(await readFile(resolve(here, '..', '..', 'testdata', 'ktx2', '2d_etc1s.ktx2'))),
});
const webpResource = texturedDocument.resources.length;
const webpBytes = new Uint8Array(await readFile(resolve(here, '..', '..', 'testdata', 'textures', 'quadrants.webp')));
assert.equal(sniffMime(webpBytes), 'image/webp', 'the fixture must be sniffed as WebP from its own bytes');
assert.equal(sniffMime(new Uint8Array(await readFile(resolve(here, '..', '..', 'testdata', 'ktx2', '2d_etc1s.ktx2')))), 'image/ktx2', 'and the KTX2 fixture as KTX2');
texturedDocument.resources.push({ name: 'quadrants.webp', mimeType: 'image/webp', bytes: webpBytes });
const avifResource = texturedDocument.resources.length;
const avifBytes = new Uint8Array(await readFile(resolve(here, '..', '..', 'testdata', 'textures', 'quadrants.avif')));
// AVIF and HEIC are the same container with different brands, so sniffing has
// to read past the box type to the brand. A fixture that only proved `ftyp`
// would pass for either.
assert.equal(sniffMime(avifBytes), 'image/avif', 'the fixture must be sniffed as AVIF from its own bytes');
texturedDocument.resources.push({ name: 'quadrants.avif', mimeType: 'image/avif', bytes: avifBytes });
const ktxTexture = texturedDocument.textures.length;
texturedDocument.textures.push({ name: 'ktx', resource: ktxResource, sampler: {} });
const webpTexture = texturedDocument.textures.length;
texturedDocument.textures.push({ name: 'webp', resource: webpResource, sampler: {} });
const avifTexture = texturedDocument.textures.length;
texturedDocument.textures.push({ name: 'avif', resource: avifResource, sampler: {} });
texturedDocument.materials.push({
    name: 'avif-texture-info',
    baseColorFactor: [1, 1, 1, 1], metallicFactor: 1, roughnessFactor: 1, emissiveFactor: [0, 0, 0],
    baseColorTexture: { texture: avifTexture, texCoord: 0 },
});
texturedDocument.materials.push({
    name: 'portable-texture-info',
    baseColorFactor: [1, 1, 1, 1], metallicFactor: 1, roughnessFactor: 1, emissiveFactor: [0, 0, 0],
    baseColorTexture: { texture: 0, texCoord: 1, transform: { offset: [0.25, 0.5], scale: [2, 3], rotation: 0.125, texCoord: 2 } },
    metallicRoughnessTexture: { texture: ktxTexture, texCoord: 3, transform: { offset: [0.1, 0.2], scale: [0.5, 0.75], rotation: 0.25 } },
    normalTexture: { texture: webpTexture, texCoord: 1, scale: 0.6, transform: { offset: [0.2, 0.3], scale: [1.5, 1.25], rotation: 0.5 } },
    emissiveTexture: { texture: 0, texCoord: 2, transform: { offset: [0.4, 0.5], scale: [0.25, 0.5], rotation: 0.75 } },
    occlusionTexture: { texture: ktxTexture, texCoord: 1, strength: 0.4, transform: { offset: [0.5, 0.6], scale: [0.75, 0.5], rotation: 1 } },
});
const texturedLowered = lowerSceneDocumentToGltf(texturedDocument);
const texturedManifest = JSON.parse(new TextDecoder().decode(texturedLowered.json));
assert.deepEqual(new Set(texturedManifest.extensionsUsed), new Set(['KHR_texture_transform', 'KHR_texture_basisu', 'EXT_texture_webp', 'EXT_texture_avif']));
// The writer emits no JPEG or PNG fallback beside an alternate image source,
// and both extensions say what that costs: a reader that skips the extension
// finds a texture with no source at all, so neither may be declared optional.
// KHR_texture_transform may, because a slot without it renders untransformed
// rather than untextured. The official validator does not check this, so the
// statement lives here.
assert.deepEqual(
    new Set(texturedManifest.extensionsRequired),
    new Set(['KHR_texture_basisu', 'EXT_texture_webp', 'EXT_texture_avif']),
    'an image source with no fallback cannot be an optional extension',
);
const portableMaterial = texturedManifest.materials.at(-1);
assert.equal(portableMaterial.normalTexture.scale, 0.6);
assert.equal(portableMaterial.occlusionTexture.strength, 0.4);
assert.equal(portableMaterial.normalTexture.extensions.KHR_texture_transform.rotation, 0.5);
assert.equal(portableMaterial.occlusionTexture.extensions.KHR_texture_transform.texCoord, undefined);
assert.equal(texturedManifest.textures[ktxTexture].extensions.KHR_texture_basisu.source >= 0, true);
assert.equal(texturedManifest.textures[webpTexture].extensions.EXT_texture_webp.source >= 0, true);
assert.equal(texturedManifest.textures[avifTexture].extensions.EXT_texture_avif.source >= 0, true);
const texturedRoundtrip = buildSceneDocumentFromGltf(texturedLowered.json, texturedLowered.resources, gltf);
const roundtripMaterial = texturedRoundtrip.materials.at(-1);
assert.deepEqual(roundtripMaterial.baseColorTexture.transform, { offset: [0.25, 0.5], scale: [2, 3], rotation: 0.125, texCoord: 2 });
assert.equal(roundtripMaterial.normalTexture.scale, 0.6);
assert.equal(roundtripMaterial.occlusionTexture.strength, 0.4);
assert.equal(texturedRoundtrip.resources.some((resource) => resource.mimeType === 'image/ktx2'), true);
assert.equal(texturedRoundtrip.resources.some((resource) => resource.mimeType === 'image/webp'), true);
assert.equal(texturedRoundtrip.resources.some((resource) => resource.mimeType === 'image/avif'), true);

// Punctual lights are the first thing the contract carries that is neither
// geometry nor material, and the writer has to put them back where the
// extension states them: at the root, with the placing node pointing at one.
const lightsSource = new Uint8Array(await readFile(resolve(
    here, '..', '..', 'testdata', 'KhronosSampleModels', 'PointLightIntensityTest',
    'glTF_Binary', 'PointLightIntensityTest.glb',
)));
const litDocument = buildSceneDocumentFromGltf(lightsSource, {}, gltf);
assert.equal(litDocument.lights.length, 8, 'the asset declares eight punctual lights');
assert.equal(
    litDocument.nodes.filter((node) => node.light !== undefined).length,
    8,
    'each light is placed by a node',
);
const litOutput = serializeSceneDocumentToGlb(litDocument, gltf);
await assertValidGlb(litOutput.binary, 'punctual-lights');
const litManifest = JSON.parse(new TextDecoder().decode(lowerSceneDocumentToGltf(litDocument).json));
assert.ok(
    litManifest.extensionsUsed.includes('KHR_lights_punctual'),
    'a scene with lights declares the extension it needs to be read back',
);
assert.equal(litManifest.extensions.KHR_lights_punctual.lights.length, 8);
const litRoundtrip = buildSceneDocumentFromGltf(litOutput.binary, {}, gltf);
// Against the asset's own numbers rather than against the document that was
// just written: comparing a reader with itself passes even when both halves
// have dropped the same field.
assert.deepEqual(
    litRoundtrip.lights.map((light) => [light.type, light.intensity, light.range]),
    Array.from({ length: 8 }, () => ['point', 1, 1.125]),
    'every light survives the round trip with the values the asset states',
);
assert.equal(
    litRoundtrip.nodes.filter((node) => node.light !== undefined).length,
    8,
    'and each is still placed by the node that placed it',
);

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
            assert.ok(blenderBounds.worst < 5e-4, `${label} Blender evaluated mesh bounds GLB drift ${blenderBounds.worst} on ${blenderBounds.mesh} sample ${blenderBounds.sample}; root offset=${blenderBounds.rootOffset}; imported armature pose basis differs by world=${blenderDrift.world}, skin=${blenderDrift.skin}`);
            console.log(`PASS ${label} Blender FBX -> GLB evaluated mesh: bounds=${blenderBounds.worst.toExponential(2)} on ${blenderBounds.mesh} sample ${blenderBounds.sample}, root offset=${blenderBounds.rootOffset.map((value) => value.toExponential(2))} (armature-basis diagnostic world=${blenderDrift.world.toExponential(2)}, skin=${blenderDrift.skin.toExponential(2)})`);
        } finally {
            await rm(temp, { recursive: true, force: true });
        }
    }
    console.log(`PASS ${label} SceneDocument -> GLB: world=${worstWorld.toExponential(2)}, skin=${worstSkin.toExponential(2)}`);
}

console.log('SceneDocument GLB structural and animation roundtrip passed');
