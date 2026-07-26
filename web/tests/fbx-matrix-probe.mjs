// Focused Mixamo FBX import probe. Set MIXAMO_FBX or FBX_FIXTURES to override
// the local fixture path. It exits nonzero for any parsed-fixture failure.
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { mixamoFbx, here, loadFbxViewerAdapter, loadWasm, readBytes, skipUnless, verbose } from './fbx-test-utils.mjs';

if (skipUnless([mixamoFbx], 'Mixamo FBX matrix probe')) process.exit(0);
const fbx = await loadWasm('fbx');
const { buildSceneFromFbx } = await loadFbxViewerAdapter();
const { Viewer } = await import(pathToFileURL(resolve(here, '..', 'www', 'viewer.js')));
const { multiplyMat4, invertMat4 } = await import(pathToFileURL(resolve(here, '..', 'src', 'mat4.ts')));
const parsed = fbx.parse_fbx(await readBytes(mixamoFbx));
if (!parsed || (parsed.meshes?.length || parsed.scene?.rootNodes?.length || 0) === 0) {
    throw new Error(`Mixamo parse failed: ${JSON.stringify(parsed).slice(0, 300)}`);
}
const scene = await buildSceneFromFbx(parsed);
const primitive = scene.meshes[0]?.primitives[0];
if (!primitive?.attributes.TEXCOORD_0 || primitive.attributes.TEXCOORD_0.count !== primitive.attributes.POSITION.count) {
    throw new Error('Mixamo UV corner expansion does not match render vertices');
}
const identity = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
const updateWorld = (node, parent) => {
    node.world = parent ? multiplyMat4(parent, node.localMatrix || identity) : (node.localMatrix || identity);
    for (const child of node.children || []) updateWorld(scene.nodes[child], node.world);
};
for (const root of scene.rootIndices) updateWorld(scene.nodes[root], null);
const renderable = scene.renderables[0];
const joint = scene.skins[renderable.skinIndex]?.joints[0];
const palette = joint && multiplyMat4(multiplyMat4(invertMat4(renderable.node.world), joint.node.world), joint.inverseBind);
const deviation = palette ? Math.max(...palette.map((value, index) => Math.abs(value - (index % 5 === 0 ? 1 : 0)))) : Infinity;
if (deviation > 1e-2) throw new Error(`Mixamo rest skin palette drift: ${deviation}`);

const probe = Object.create(Viewer.prototype);
probe.scene = scene;
probe.animation = { clipIndex: 0, time: 0 };
for (const time of [0, scene.animations[0].duration * 0.5, scene.animations[0].duration]) {
    if (!probe.seekAnimation(time)) throw new Error(`animation seek failed at ${time}`);
    for (const node of scene.nodes) {
        for (const value of [...(node.trs?.translation || []), ...(node.trs?.rotation || []), ...(node.trs?.scale || [])]) {
            if (!Number.isFinite(value)) throw new Error(`non-finite animation transform at ${time}`);
        }
    }
}
verbose({ fixture: mixamoFbx, clips: scene.animations.map((clip) => ({ name: clip.name, duration: clip.duration })) });
console.log(`PASS Mixamo: UV expansion, rest skin palette=${deviation.toExponential(2)}, finite animation seeks`);
