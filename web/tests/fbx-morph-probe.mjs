// FBX morph preview/write round-trip. Set MORPH_FBX or FBX_FIXTURES to override.
import { loadFbxViewerAdapter, loadWasm, morphFbx, readBytes, skipUnless } from './fbx-test-utils.mjs';

if (skipUnless([morphFbx], 'FBX morph probe')) process.exit(0);
const fbx = await loadWasm('fbx');
const { buildSceneFromFbx } = await loadFbxViewerAdapter();
const parsed = fbx.parse_fbx(await readBytes(morphFbx));
if (!parsed?.scene) throw new Error(`Morph fixture did not produce an FBX scene: ${JSON.stringify(parsed).slice(0, 300)}`);
const scene = await buildSceneFromFbx(parsed);
const sourceTargets = parsed.scene.rootNodes.flatMap((node) => node.meshes || []).reduce((count, mesh) => count + (mesh.morphTargets?.length || 0), 0);
const previewTargets = scene.meshes.flatMap((mesh) => mesh.primitives || []).reduce((count, primitive) => count + (primitive.morphPositions?.length || 0), 0);
if (sourceTargets > 0 && previewTargets !== sourceTargets) throw new Error(`morph preview target count mismatch: ${previewTargets} vs ${sourceTargets}`);
const written = fbx.create_fbx_scene(parsed.scene, {});
if (!written.success || !written.binary_data?.length) throw new Error(`morph FBX write failed: ${written.error}`);
const reparsed = fbx.parse_fbx(written.binary_data);
const roundtripTargets = reparsed.scene?.rootNodes?.flatMap((node) => node.meshes || []).reduce((count, mesh) => count + (mesh.morphTargets?.length || 0), 0) || 0;
if (roundtripTargets !== sourceTargets) throw new Error(`morph round-trip target count mismatch: ${roundtripTargets} vs ${sourceTargets}`);
console.log(`PASS FBX morph: preview/write round-trip (${sourceTargets} targets)`);
