// Focused glTF -> FBX -> FBX matrix round-trip. Debug export is opt-in via
// DRACO_WRITE_DEBUG_ARTIFACTS=1; it never writes .scratch during normal tests.
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { foxBin, foxGltf, here, loadWasm, readBytes, skipUnless, verbose } from './fbx-test-utils.ts';
import { pathToFileURL } from 'node:url';

if (skipUnless([foxGltf, foxBin], 'Fox FBX round-trip probe')) process.exit(0);
const [fbx, gltf] = await Promise.all([loadWasm('fbx'), loadWasm('gltf')]);
const { buildFbxSceneFromGltf } = await import(pathToFileURL(resolve(here, '..', 'src', 'gltf-loader.ts')).href);
const scene = buildFbxSceneFromGltf(await readBytes(foxGltf), { 'Fox.bin': await readBytes(foxBin) }, gltf, { legacyCompatibility: true });
const findWithTranslation = (nodes: any[]): any => {
    let best: any = null;
    let bestMagnitude = 0;
    for (const node of nodes) {
        if (node.matrix) {
            const magnitude = Math.hypot(node.matrix[12], node.matrix[13], node.matrix[14]);
            if (magnitude > bestMagnitude) { best = node; bestMagnitude = magnitude; }
        }
        const child = findWithTranslation(node.children || []);
        if (child && Math.hypot(child.matrix[12], child.matrix[13], child.matrix[14]) > bestMagnitude) best = child;
    }
    return best;
};
const node = findWithTranslation(scene.rootNodes);
if (!node?.matrix) throw new Error('Fox has no translated node matrix');
const written = fbx.create_fbx_scene(scene, { version: 7500, legacyCompatibility: true });
if (!written.success) throw new Error(`Fox FBX write failed: ${written.error}`);
const reparsed = fbx.parse_fbx(new Uint8Array(written.binary_data));
const findByName = (nodes: any[], name: string): any => nodes.flatMap((candidate) => [candidate, ...findAll(candidate.children || [])]).find((candidate) => candidate.name === name);
const findAll = (nodes: any[]): any[] => nodes.flatMap((candidate) => [candidate, ...findAll(candidate.children || [])]);
const after = findByName(reparsed.scene?.rootNodes || [], node.name)?.matrix;
if (!after) throw new Error(`Fox reparse omitted node ${node.name}`);
const maxDiff = Math.max(...node.matrix.map((value: number, index: number) => Math.abs(value - after[index])));
if (maxDiff >= 1e-3) throw new Error(`Fox matrix drift after round-trip: ${maxDiff}`);
if (process.env.DRACO_WRITE_DEBUG_ARTIFACTS === '1') {
    const scratch = resolve(here, '..', '..', '.scratch');
    await mkdir(scratch, { recursive: true });
    await writeFile(resolve(scratch, 'fox_export.fbx'), Buffer.from(written.binary_data));
}
verbose({ fixture: foxGltf, node: node.name, maxDiff });
console.log(`PASS Fox FBX matrix round-trip: max diff=${maxDiff.toExponential(3)}`);
