// Verify the two FBX fixes against real fixtures:
//   1. Reader handles compressed FBX arrays (mixamo.fbx) — was failing with
//      "FBX array compression not supported".
//   2. Writer emits column-major matrices (Blender reads them correctly).
// Run: node tests/fbx-matrix-probe.mjs
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

// gltf-loader.js references WebGL2RenderingContext at module top-level for the
// browser preview path. Stub it so the FBX-scene builder (which has no GL
// dependency) can be loaded in Node.
if (typeof globalThis.WebGL2RenderingContext === 'undefined') {
    globalThis.WebGL2RenderingContext = class { static REPEAT = 0x2901; static LINEAR_MIPMAP_LINEAR = 0x2703; static LINEAR = 0x2601; };
}

const here = dirname(fileURLToPath(import.meta.url));
const pkg = resolve(here, '..', 'www', 'pkg');

async function load(name) {
    const module = await import(pathToFileURL(resolve(pkg, `${name}.js`)));
    const wasm = await readFile(resolve(pkg, `${name}_bg.wasm`));
    await module.default({ module_or_path: wasm });
    return module;
}

const fbx = await load('fbx');
const gltf = await load('gltf');
const { buildSceneFromFbx } = await import(pathToFileURL(resolve(here, '..', 'www', 'mesh-loader.js')));
const { Viewer } = await import(pathToFileURL(resolve(here, '..', 'www', 'viewer.js')));
const { multiplyMat4, invertMat4 } = await import(pathToFileURL(resolve(here, '..', 'www', 'mat4.js')));

// --- Fix 1: compressed FBX arrays ---------------------------------------
const FIXTURES = 'D:/Projects/Three.ts/examples/models/fbx';
for (const file of ['mixamo.fbx', 'morph_test.fbx']) {
    const path = `${FIXTURES}/${file}`;
    let bytes;
    try { bytes = await readFile(path); } catch { console.log(`SKIP ${file} (not present)`); continue; }
    try {
        const parsed = fbx.parse_fbx(new Uint8Array(bytes));
        const ok = parsed && (parsed.meshes?.length || parsed.scene?.rootNodes?.length || 0) > 0;
        console.log(`${ok ? 'PASS' : 'FAIL'} ${file}: parsed=${ok ? 'yes' : 'no'}, meshes=${parsed?.meshes?.length ?? 0}, warnings=${parsed?.warnings?.length ?? 0}`);
        if (!ok) console.log('  result:', JSON.stringify(parsed).slice(0, 300));
        if (file === 'mixamo.fbx' && ok) {
            const scene = await buildSceneFromFbx(parsed);
            const primitive = scene.meshes[0]?.primitives[0];
            if (!primitive?.attributes.TEXCOORD_0
                || primitive.attributes.TEXCOORD_0.count !== primitive.attributes.POSITION.count) {
                throw new Error('Mixamo UV corner expansion does not match render vertices');
            }
            const identity = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
            const updateWorld = (node, parent) => {
                node.world = parent
                    ? multiplyMat4(parent, node.localMatrix || identity)
                    : (node.localMatrix || identity);
                for (const child of node.children || []) updateWorld(scene.nodes[child], node.world);
            };
            for (const root of scene.rootIndices) updateWorld(scene.nodes[root], null);
            const renderable = scene.renderables[0];
            const joint = scene.skins[renderable.skinIndex]?.joints[0];
            const palette = joint && multiplyMat4(
                multiplyMat4(invertMat4(renderable.node.world), joint.node.world),
                joint.inverseBind,
            );
            const deviation = palette
                ? Math.max(...palette.map((value, index) => Math.abs(value - (index % 5 === 0 ? 1 : 0))))
                : Infinity;
            if (deviation > 1e-2) throw new Error(`Mixamo rest skin palette drift: ${deviation}`);
            console.log(`PASS ${file}: UV expansion and rest bind palette deviation=${deviation.toExponential(2)}`);

            // FBX cubic curves expose values and tangents separately.  The
            // viewer adapter must not feed those value-only arrays into its
            // glTF cubic layout (or Euler keys into quaternion cubic
            // interpolation), which previously produced NaN rotations and
            // made the character fly out of frame midway through playback.
            const probe = Object.create(Viewer.prototype);
            probe.scene = scene;
            probe.animation = { clipIndex: 0, time: 0 };
            for (const time of [0, scene.animations[0].duration * 0.5, scene.animations[0].duration]) {
                if (!probe.seekAnimation(time)) throw new Error(`animation seek failed at ${time}`);
                for (const animatedNode of scene.nodes) {
                    for (const value of [
                        ...(animatedNode.trs?.translation || []),
                        ...(animatedNode.trs?.rotation || []),
                        ...(animatedNode.trs?.scale || []),
                    ]) {
                        if (!Number.isFinite(value)) {
                            throw new Error(`non-finite animation transform at ${time}`);
                        }
                    }
                }
            }
            console.log(`PASS ${file}: animation seeks remain finite through the full clip`);
        }
        if (file === 'morph_test.fbx' && ok && parsed.scene) {
            const morphScene = await buildSceneFromFbx(parsed);
            const sourceTargets = parsed.scene.rootNodes
                .flatMap((node) => node.meshes || [])
                .reduce((sum, mesh) => sum + (mesh.morphTargets?.length || 0), 0);
            const gpuTargets = morphScene.meshes
                .flatMap((mesh) => mesh.primitives || [])
                .reduce((sum, primitive) => sum + (primitive.morphPositions?.length || 0), 0);
            if (sourceTargets > 0 && gpuTargets !== sourceTargets) {
                throw new Error(`morph preview target count mismatch: ${gpuTargets} vs ${sourceTargets}`);
            }
            const rewritten = fbx.create_fbx_scene(parsed.scene, {});
            if (!rewritten.success || !rewritten.binary_data?.length) {
                throw new Error(`morph FBX write failed: ${rewritten.error}`);
            }
            const reparsed = fbx.parse_fbx(rewritten.binary_data);
            const roundtripTargets = reparsed.scene?.rootNodes
                ?.flatMap((node) => node.meshes || [])
                .reduce((sum, mesh) => sum + (mesh.morphTargets?.length || 0), 0) || 0;
            if (roundtripTargets !== sourceTargets) {
                throw new Error(`morph round-trip target count mismatch: ${roundtripTargets} vs ${sourceTargets}`);
            }
            console.log(`PASS ${file}: morph targets preview/write round-trip (${sourceTargets})`);
        }
    } catch (error) {
        console.log(`FAIL ${file}: ${error}`);
    }
}

// --- Fix 2: Fox.gltf → FBX → re-read; check matrix is column-major -------
const FOX = 'D:/Projects/draco-rust/testdata/Fox/glTF/Fox.gltf';
let foxBytes;
try { foxBytes = await readFile(FOX); } catch { console.log('SKIP Fox (not present)'); process.exit(0); }
const foxBin = await readFile('D:/Projects/draco-rust/testdata/Fox/glTF/Fox.bin');
const { buildFbxSceneFromGltf } = await import(pathToFileURL(resolve(here, '..', 'www', 'gltf-loader.js')));

const scene = buildFbxSceneFromGltf(
    new Uint8Array(foxBytes),
    { 'Fox.bin': new Uint8Array(foxBin) },
    gltf,
    { legacyCompatibility: true },
);

// Pick a non-identity node matrix and check it round-trips through our writer.
const findNonIdentity = (nodes) => {
    for (const node of nodes) {
        if (node.matrix && node.matrix.some((v, i) => Math.abs(v - (i % 5 === 0 ? 1 : 0)) > 0.001)) return node;
        if (node.children) { const found = findNonIdentity(node.children); if (found) return found; }
    }
    return null;
};
// Prefer a node that actually has a non-zero translation, so we can also
// validate where the translation lands in the encoded matrix.
const findWithTranslation = (nodes) => {
    let best = null;
    let bestMag = 0;
    const visit = (list) => {
        for (const node of list) {
            if (node.matrix) {
                const mag = Math.hypot(node.matrix[12], node.matrix[13], node.matrix[14]);
                if (mag > bestMag && node.matrix.some((v, i) => Math.abs(v - (i % 5 === 0 ? 1 : 0)) > 0.001)) {
                    bestMag = mag; best = node;
                }
            }
            if (node.children) visit(node.children);
        }
    };
    visit(nodes);
    return best || findNonIdentity(nodes);
};
const node = findWithTranslation(scene.rootNodes);
if (!node) { console.log('FAIL Fox: no non-identity node matrix found'); process.exit(1); }

const before = node.matrix;
console.log(`Fox node "${node.name}" matrix (row-major, our convention):`);
for (let r = 0; r < 4; r++) console.log('  ', before.slice(r * 4, r * 4 + 4).map((v) => +v.toFixed(4)));

// Write the scene to FBX bytes and re-read it. If the matrix convention is
// consistent, the re-read matrices should match. The translation should be in
// the bottom row (indices 12,13,14).
const fbxBytes = fbx.create_fbx_scene(scene, { version: 7500, legacyCompatibility: true });
if (!fbxBytes?.success) { console.log('FAIL Fox: create_fbx_scene failed:', fbxBytes?.error); process.exit(1); }
const reparsed = fbx.parse_fbx(new Uint8Array(fbxBytes.binary_data));
const findNode = (nodes, name) => {
    for (const n of nodes) { if (n.name === name) return n; if (n.children) { const f = findNode(n.children, name); if (f) return f; } }
    return null;
};
const reparsedNode = findNode(reparsed.scene.rootNodes, node.name);
if (!reparsedNode?.matrix) { console.log(`FAIL Fox: re-parsed node "${node.name}" has no matrix`); process.exit(1); }

const after = reparsedNode.matrix;
let maxDiff = 0;
for (let i = 0; i < 16; i++) maxDiff = Math.max(maxDiff, Math.abs(before[i] - after[i]));
console.log(`Fox round-trip max matrix diff: ${maxDiff.toExponential(3)}`);
console.log(maxDiff < 1e-3 ? 'PASS Fox: matrix round-trips consistently' : 'FAIL Fox: matrix drift after round-trip');

// Write the FBX to disk so Blender can validate it end-to-end.
const { writeFile, mkdir } = await import('node:fs/promises');
const scratchDir = resolve(here, '..', '..', '.scratch');
await mkdir(scratchDir, { recursive: true });
const outPath = resolve(scratchDir, 'fox_export.fbx');
await writeFile(outPath, Buffer.from(fbxBytes.binary_data));
console.log(`Wrote Fox FBX to ${outPath}`);
console.log(`  non-identity node "${node.name}": bottom-row T=[${after[12].toFixed(3)}, ${after[13].toFixed(3)}, ${after[14].toFixed(3)}]`);

// Dump the skin cluster data we generate for the hip joint so we can compare
// against what Blender reconstructs. joint.bind is the joint world bind matrix
// (TransformLink) that Blender uses for bone positions.
const skinMesh = scene.rootNodes.flatMap((n) => n.children || []).flatMap((n) => n.meshes || []).find((m) => m.skin);
if (skinMesh?.skin?.clusters) {
    const hipCluster = skinMesh.skin.clusters.find((c) => c.jointNodeId != null);
    console.log(`\nSkin has ${skinMesh.skin.clusters.length} clusters, bindPose has ${skinMesh.skin.bindPose?.length || 0} entries`);
    // Show first 3 bind pose entries to verify positions are encoded.
    for (const entry of (skinMesh.skin.bindPose || []).slice(0, 5)) {
        const m = entry.matrix;
        console.log(`  bindPose[${entry.nodeId}] T=(${m[12].toFixed(2)}, ${m[13].toFixed(2)}, ${m[14].toFixed(2)})`);
    }
    // Find the cluster for b_Hip_01 (id 5 based on the earlier dump).
    const hip = skinMesh.skin.clusters.find((c) => c.jointNodeId === 5);
    if (hip) {
        console.log(`\nb_Hip_01 cluster TransformLink (jointBindTransform):`);
        for (let r = 0; r < 4; r++) console.log('  ', hip.jointBindTransform.slice(r * 4, r * 4 + 4).map((v) => +v.toFixed(4)));
        console.log(`b_Hip_01 cluster Transform (meshBindTransform):`);
        for (let r = 0; r < 4; r++) console.log('  ', hip.meshBindTransform.slice(r * 4, r * 4 + 4).map((v) => +v.toFixed(4)));
    } else {
        console.log('\nb_Hip_01 (id=5) cluster not found; available joint ids:', skinMesh.skin.clusters.map((c) => c.jointNodeId).slice(0, 10));
    }
} else {
    console.log('\nNo skin found on meshes');
}

// Compare the IBM-derived joint bind matrix against the world matrix. They
// should be identical (both equal the joint's rest world transform in FBX
// coordinates). A mismatch here points to a convention bug in buildFbxSkins.
console.log('\n=== Direct IBM-vs-world comparison for b_Hip_01 ===');
{
    const doc = JSON.parse(new TextDecoder().decode(gltf.GltfAsset.withResources(
        new Uint8Array(foxBytes), { 'Fox.bin': new Uint8Array(foxBin) }, '2.1',
    ).json()));
    const skin = doc.skins[0];
    const ibmAccessor = skin.inverseBindMatrices;
    const packed = gltf.GltfAsset.withResources(
        new Uint8Array(foxBytes), { 'Fox.bin': new Uint8Array(foxBin) }, '2.1',
    );
    const accessor = packed.readAccessor(ibmAccessor);
    const componentType = accessor.componentType();
    const count = accessor.count();
    const bytes = new Uint8Array(accessor.bytes());
    accessor.free();
    packed.free();
    console.log(`IBM accessor: componentType=${componentType}, count=${count}, byteLen=${bytes.length}`);
    // b_Hip_01 is joint index 2 in the skin (joints[2]); node id is joints[2]+1.
    const hipJointIndex = skin.joints.indexOf(skin.joints.find((j) => doc.nodes[j].name === 'b_Hip_01'));
    console.log(`b_Hip_01 joint index in skin: ${hipJointIndex}, node id: ${skin.joints[hipJointIndex] + 1}`);
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const ibm = [];
    for (let i = 0; i < 16; i++) ibm.push(view.getFloat32((hipJointIndex * 16 + i) * 4, true));
    console.log('IBM (column-major, glTF):');
    for (let r = 0; r < 4; r++) console.log('  ', [ibm[r], ibm[r + 4], ibm[r + 8], ibm[r + 12]].map((v) => +v.toFixed(4)));
}
