// Structural inventory for a second animated FBX. It intentionally does not
// assert motion correctness: Samba Dancing currently has a known inverted-arm
// visual mismatch and remains a focused follow-up transform-semantics target.
// Set SAMBA_FBX or FBX_FIXTURES to override the local fixture path.
import { loadFbxViewerAdapter, loadWasm, readBytes, sambaFbx, skipUnless, verbose } from './fbx-test-utils.ts';

if (skipUnless([sambaFbx], 'Samba Dancing FBX scene probe')) process.exit(0);
const fbx = await loadWasm('fbx');
const { buildSceneFromFbx } = await loadFbxViewerAdapter();
const parsed = fbx.parse_fbx(await readBytes(sambaFbx));
if (!parsed?.scene?.rootNodes?.length) throw new Error('Samba Dancing did not produce a semantic FBX scene');
const scene = await buildSceneFromFbx(parsed);
if (scene.nodes.length === 0 || scene.renderables.length === 0 || scene.skins.length === 0) {
    throw new Error(`Samba Dancing scene adaptation is incomplete: ${JSON.stringify({ nodes: scene.nodes.length, renderables: scene.renderables.length, skins: scene.skins.length })}`);
}
if (scene.animations.length === 0 || scene.animations.some((clip) => !Number.isFinite(clip.duration) || clip.duration <= 0 || clip.channels.length === 0)) {
    throw new Error('Samba Dancing animation clips are missing or invalid');
}
for (const clip of scene.animations) {
    for (const channel of clip.channels) {
        if (channel.sampler.input.some((value) => !Number.isFinite(value))
            || channel.sampler.output.some((value) => !Number.isFinite(value))) {
            throw new Error(`Samba Dancing contains non-finite ${channel.path} samples`);
        }
    }
}
verbose({ fixture: sambaFbx, nodes: scene.nodes.length, skins: scene.skins.length, clips: scene.animations.map((clip) => ({ name: clip.name, duration: clip.duration, channels: clip.channels.length })) });
console.log(`OBSERVED Samba Dancing scene boundary: ${scene.nodes.length} nodes, ${scene.skins.length} skins, ${scene.animations.length} animation clips (not a motion-acceptance test)`);
