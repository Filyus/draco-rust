/**
 * How Blender is found, on a Unix as well as on Windows.
 *
 * The six Blender-gated suites are the only users, and every one of them skips
 * itself when the resolver returns nothing -- so a resolver that quietly finds
 * nothing on Linux turns those suites off and reports it as a missing
 * dependency. That failure is invisible: the suites still exit zero. Hence a
 * test for the search itself, which needs no Blender to run.
 *
 * The platform is forced rather than detected, so the Unix branch is exercised
 * on Windows and vice versa. `PATH`'s separator comes from `node:path` and is
 * therefore the host's; this builds its `PATH` the same way rather than
 * pretending otherwise.
 */
import assert from 'node:assert/strict';
import { chmodSync, mkdirSync, writeFileSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { delimiter, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { blenderExecutable } from './fbx-test-utils.ts';

const scratch = await mkdtemp(resolve(tmpdir(), 'draco-blender-resolver-'));
const platformDescriptor = Object.getOwnPropertyDescriptor(process, 'platform')!;
const originalPath = process.env.PATH;
const originalBlender = process.env.BLENDER;

function asPlatform(platform: string, body: () => void): void {
    Object.defineProperty(process, 'platform', { value: platform, configurable: true });
    try {
        body();
    } finally {
        Object.defineProperty(process, 'platform', platformDescriptor);
    }
}

/** A runnable file at `directory/name`. */
function executable(directory: string, name: string): string {
    mkdirSync(directory, { recursive: true });
    const path = resolve(directory, name);
    writeFileSync(path, '#!/bin/sh\nexit 0\n');
    chmodSync(path, 0o755);
    return path;
}

try {
    delete process.env.BLENDER;

    // --- Unix: found on PATH under its unsuffixed name.
    const unixBin = resolve(scratch, 'unix-bin');
    const unixBlender = executable(unixBin, 'blender');
    // A decoy earlier on PATH: a *directory* by the right name. `existsSync`
    // accepts it, which is why the resolver asks whether it is a runnable file.
    const decoy = resolve(scratch, 'decoy-bin');
    mkdirSync(resolve(decoy, 'blender'), { recursive: true });

    asPlatform('linux', () => {
        process.env.PATH = [decoy, unixBin].join(delimiter);
        assert.equal(blenderExecutable(), unixBlender, 'Linux: Blender on PATH');

        process.env.PATH = decoy;
        assert.equal(blenderExecutable(), '', 'Linux: a directory named blender is not Blender');

        // Nothing on PATH and no Blender: the Windows install-root search must
        // not run, or a Unix would report whatever a mounted Windows disk holds.
        process.env.PATH = '';
        assert.equal(blenderExecutable(), '', 'Linux: nothing found is empty, not a Windows path');
    });

    // macOS takes the same branch. An `.app` bundle is not on PATH and is not
    // looked for -- BLENDER is how it is named -- but a Homebrew or linked
    // binary is found like any other.
    asPlatform('darwin', () => {
        process.env.PATH = unixBin;
        assert.equal(blenderExecutable(), unixBlender, 'macOS: Blender on PATH');
    });

    // --- Windows: the suffixed name, and `blender` alone is not it.
    const windowsBin = resolve(scratch, 'windows-bin');
    const windowsBlender = executable(windowsBin, 'blender.exe');
    executable(resolve(scratch, 'unsuffixed'), 'blender');
    asPlatform('win32', () => {
        process.env.PATH = [resolve(scratch, 'unsuffixed'), windowsBin].join(delimiter);
        assert.equal(blenderExecutable(), windowsBlender, 'Windows: blender.exe on PATH');
    });

    // --- BLENDER wins wherever it points, and a stale value finds nothing
    // rather than falling through to some other install.
    process.env.PATH = [unixBin, windowsBin].join(delimiter);
    process.env.BLENDER = unixBlender;
    asPlatform('linux', () => {
        assert.equal(blenderExecutable(), unixBlender, 'BLENDER is honoured');
    });
    process.env.BLENDER = resolve(scratch, 'gone', 'blender');
    asPlatform('linux', () => {
        assert.equal(blenderExecutable(), '', 'a stale BLENDER reports nothing, not a fallback');
    });

    console.log('PASS Blender resolver: PATH on Unix and Windows, decoys rejected, BLENDER honoured');
} finally {
    if (originalPath === undefined) delete process.env.PATH;
    else process.env.PATH = originalPath;
    if (originalBlender === undefined) delete process.env.BLENDER;
    else process.env.BLENDER = originalBlender;
    await rm(scratch, { recursive: true, force: true });
}
