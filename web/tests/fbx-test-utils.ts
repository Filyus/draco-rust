import { readFile } from 'node:fs/promises';
import { existsSync, readdirSync } from 'node:fs';
import { delimiter, dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const here = dirname(fileURLToPath(import.meta.url));
export const repoRoot = resolve(here, '..', '..');
export const pkg = resolve(here, '..', 'www', 'pkg');

// Mixamo/Samba/morph fixtures are copyrighted assets that stay off this
// machine's disk and out of the repository, so the paths to them are
// necessarily local. `web/.env` (gitignored; see `web/.env.example`) is where
// a checkout points FBX_FIXTURES etc. at wherever it keeps them; a machine
// without one leaves fixtureRoot resolving under testdata/, which does not
// exist, and skipUnless below reports the gated tests as skipped rather than
// failing on a path built into the code.
try {
    process.loadEnvFile(resolve(here, '..', '.env'));
} catch {
    // No web/.env: the fixture-gated tests below will skip themselves.
}

if (typeof (globalThis as any).WebGL2RenderingContext === 'undefined') {
    (globalThis as any).WebGL2RenderingContext = class {
        static REPEAT = 0x2901;
        static LINEAR_MIPMAP_LINEAR = 0x2703;
        static LINEAR = 0x2601;
    };
}

export const fixtureRoot = process.env.FBX_FIXTURES || resolve(repoRoot, 'testdata', 'external', 'fbx');
export const mixamoFbx = process.env.MIXAMO_FBX || resolve(fixtureRoot, 'mixamo.fbx');
export const morphFbx = process.env.MORPH_FBX || resolve(fixtureRoot, 'morph_test.fbx');
export const sambaFbx = process.env.SAMBA_FBX || resolve(fixtureRoot, 'Samba Dancing.fbx');
export const foxGltf = process.env.FOX_GLTF || resolve(repoRoot, 'testdata', 'Fox', 'glTF', 'Fox.gltf');
export const foxBin = process.env.FOX_BIN || resolve(repoRoot, 'testdata', 'Fox', 'glTF', 'Fox.bin');

/**
 * Where Blender is, without writing down where it is on one machine.
 *
 * Six Blender-gated suites each carried the same absolute path to one
 * developer's install as their fallback, which is a machine's configuration
 * sitting in the repository: right nowhere else, and stale the moment that
 * machine upgrades. `BLENDER` in `web/.env` names it; failing that this looks
 * where the operating system says to look — `PATH`, then the install roots
 * Windows advertises through `PROGRAMFILES` — so a default install is found
 * without anything being hardcoded. `test:no-absolute-paths` keeps it that way.
 *
 * Returns an empty string when there is no Blender, which every caller reports
 * as a skip.
 */
export function blenderExecutable(): string {
    const named = process.env.BLENDER;
    if (named) return existsSync(named) ? named : '';

    const names = process.platform === 'win32' ? ['blender.exe'] : ['blender'];
    for (const directory of (process.env.PATH || '').split(delimiter)) {
        if (!directory) continue;
        for (const name of names) {
            const candidate = resolve(directory, name);
            if (existsSync(candidate)) return candidate;
        }
    }

    // Windows installs Blender per version under Program Files and puts none of
    // it on PATH. Newest version wins, which is what a person would pick.
    const roots = [process.env.PROGRAMFILES, process.env.ProgramW6432, process.env['PROGRAMFILES(X86)']];
    const installs: string[] = [];
    for (const root of roots) {
        const vendor = root && resolve(root, 'Blender Foundation');
        if (!vendor || !existsSync(vendor)) continue;
        for (const entry of readdirSync(vendor)) {
            const candidate = resolve(vendor, entry, 'blender.exe');
            if (existsSync(candidate)) installs.push(candidate);
        }
    }
    installs.sort((left, right) => right.localeCompare(left, 'en', { numeric: true }));
    return installs[0] || '';
}

export function skipUnless(paths: string[], label: string): boolean {
    const missing = paths.filter((path) => !existsSync(path));
    if (missing.length === 0) return false;
    console.log(`SKIP ${label}: missing ${missing.join(', ')}`);
    return true;
}

export async function loadWasm(name: string): Promise<any> {
    const module = await import(pathToFileURL(resolve(pkg, `${name}.js`)).href);
    const wasm = await readFile(resolve(pkg, `${name}_bg.wasm`));
    await module.default({ module_or_path: wasm });
    return module;
}

export async function loadFbxViewerAdapter(): Promise<any> {
    return import(pathToFileURL(resolve(here, '..', 'src', 'mesh-loader.ts')).href);
}

export async function readBytes(path: string): Promise<Uint8Array> {
    return new Uint8Array(await readFile(path));
}

export function verbose(...values: unknown[]): void {
    if (process.env.DRACO_TEST_VERBOSE === '1') console.log(...values);
}

