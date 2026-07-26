import { readFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

if (typeof globalThis.WebGL2RenderingContext === 'undefined') {
    globalThis.WebGL2RenderingContext = class {
        static REPEAT = 0x2901;
        static LINEAR_MIPMAP_LINEAR = 0x2703;
        static LINEAR = 0x2601;
    };
}

export const here = dirname(fileURLToPath(import.meta.url));
export const repoRoot = resolve(here, '..', '..');
export const pkg = resolve(here, '..', 'www', 'pkg');
export const fixtureRoot = process.env.FBX_FIXTURES || 'D:/Projects/Three.ts/examples/models/fbx';
export const mixamoFbx = process.env.MIXAMO_FBX || resolve(fixtureRoot, 'mixamo.fbx');
export const morphFbx = process.env.MORPH_FBX || resolve(fixtureRoot, 'morph_test.fbx');
export const sambaFbx = process.env.SAMBA_FBX || resolve(fixtureRoot, 'Samba Dancing.fbx');
export const foxGltf = process.env.FOX_GLTF || resolve(repoRoot, 'testdata', 'Fox', 'glTF', 'Fox.gltf');
export const foxBin = process.env.FOX_BIN || resolve(repoRoot, 'testdata', 'Fox', 'glTF', 'Fox.bin');

export function skipUnless(paths, label) {
    const missing = paths.filter((path) => !existsSync(path));
    if (missing.length === 0) return false;
    console.log(`SKIP ${label}: missing ${missing.join(', ')}`);
    return true;
}

export async function loadWasm(name) {
    const module = await import(pathToFileURL(resolve(pkg, `${name}.js`)));
    const wasm = await readFile(resolve(pkg, `${name}_bg.wasm`));
    await module.default({ module_or_path: wasm });
    return module;
}

export async function loadFbxViewerAdapter() {
    return import(pathToFileURL(resolve(here, '..', 'src', 'mesh-loader.ts')));
}

export async function readBytes(path) {
    return new Uint8Array(await readFile(path));
}

export function verbose(...values) {
    if (process.env.DRACO_TEST_VERBOSE === '1') console.log(...values);
}
