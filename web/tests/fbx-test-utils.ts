import { readFile } from 'node:fs/promises';
import { accessSync, constants, existsSync, readdirSync, statSync } from 'node:fs';
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
 * where the operating system says to look, without anything being hardcoded:
 *
 *   everywhere -- `PATH`, which is where a distribution package or a snap puts
 *                 it, and where a tarball install is reached from once it has
 *                 been linked or added there.
 *   Windows    -- additionally the per-version install directories under the
 *                 root `PROGRAMFILES` advertises, because the installer adds
 *                 none of it to `PATH`. Newest version wins.
 *
 * Two cases it does not find, deliberately: a macOS `.app` bundle, and a Flatpak
 * that is only reachable as `flatpak run org.blender.Blender`. Both would mean
 * writing down a fixed location or a launcher's syntax, which is the thing this
 * replaced -- `BLENDER` covers them, and `.env.example` says so.
 *
 * Returns an empty string when there is no Blender, which every caller reports
 * as a skip.
 */
export function blenderExecutable(): string {
    const named = process.env.BLENDER;
    if (named) return existsSync(named) ? named : '';

    const windows = process.platform === 'win32';
    for (const directory of (process.env.PATH || '').split(delimiter)) {
        if (!directory) continue;
        const candidate = resolve(directory, windows ? 'blender.exe' : 'blender');
        if (isExecutableFile(candidate)) return candidate;
    }
    if (!windows) return '';

    // The Windows installer keeps each version in its own directory and adds
    // none of them to PATH. Newest wins, which is what a person would pick.
    const roots = [process.env.PROGRAMFILES, process.env.ProgramW6432, process.env['PROGRAMFILES(X86)']];
    const installs: string[] = [];
    for (const root of roots) {
        const vendor = root && resolve(root, 'Blender Foundation');
        if (!vendor || !existsSync(vendor)) continue;
        for (const entry of readdirSync(vendor)) {
            const candidate = resolve(vendor, entry, 'blender.exe');
            if (isExecutableFile(candidate)) installs.push(candidate);
        }
    }
    installs.sort((left, right) => right.localeCompare(left, 'en', { numeric: true }));
    return installs[0] || '';
}

/**
 * A file that can actually be run, rather than merely a name that exists.
 *
 * `existsSync` alone is enough on Windows and is not on a Unix: a directory
 * called `blender` on `PATH` would satisfy it, and so would a file without its
 * executable bit -- both of which would be handed to `spawnSync` and fail as
 * something other than "no Blender here".
 */
function isExecutableFile(path: string): boolean {
    try {
        if (!statSync(path).isFile()) return false;
        if (process.platform !== 'win32') accessSync(path, constants.X_OK);
        return true;
    } catch {
        return false;
    }
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

