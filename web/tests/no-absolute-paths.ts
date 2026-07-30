/**
 * No machine's own paths in the repository's code.
 *
 * Six Blender-gated suites each carried `C:/Program Files/...` to one
 * developer's install as their fallback. It reads as a convenience and is
 * really a machine's configuration committed as source: correct on exactly one
 * computer, silently stale after an upgrade there, and a skip everywhere else
 * that looks like a missing dependency. The same shape had already leaked in for
 * fixture directories before `web/.env` existed.
 *
 * So paths that name a place on a disk live in `.env` — gitignored — and are
 * documented in `.env.example`, which holds no real ones. Everything else has to
 * resolve its inputs at runtime: from an environment variable, from `PATH`, from
 * a directory the operating system advertises, or relative to the file asking.
 *
 * The patterns here are assembled from fragments rather than written out, so
 * this file is not itself an exception to what it checks.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { repoRoot } from './fbx-test-utils.ts';

/** Text files worth reading. Anything else is data or a build product. */
const EXTENSIONS = [
    'ts', 'tsx', 'js', 'mjs', 'cjs', 'rs', 'json', 'yml', 'yaml',
    'toml', 'ps1', 'sh', 'py', 'html', 'css', 'md', 'example',
];

/**
 * Paths that are allowed to name a disk, and why.
 *
 * `.env.example` documents the variables; the placeholders in it are the point.
 */
const ALLOWED = [/(^|\/)\.env(\.|$)/];

/** Generated or vendored trees: not ours to hold to this. */
const IGNORED = [/(^|\/)node_modules\//, /^web\/www\/pkg\//, /(^|\/)target\//];

/**
 * Filesystem roots, without the slash that would make them absolute here.
 *
 * Deliberately not every path starting with `/`: the shell serves pages, and
 * `/index.html` or a `url(/…)` in CSS is a URL, not a place on a disk.
 */
const POSIX_ROOTS = [
    'usr', 'opt', 'home', 'Users', 'Applications', 'Library',
    'var', 'etc', 'mnt', 'media', 'private', 'root', 'srv',
];

const SLASH = String.fromCharCode(47);
const DRIVE = /(^|[^A-Za-z0-9_])[A-Za-z]:[\\/]/;
const POSIX = new RegExp(`(^|['"\`(=\\s])${SLASH}(${POSIX_ROOTS.join('|')})${SLASH}`);

/**
 * A `/tmp/…` literal is the same mistake in a portable disguise: it is a real
 * place, wrong on Windows, and `os.tmpdir()` is the answer. Checked separately
 * only because it needs a word boundary the roots above get from their slash.
 */
const TEMP = new RegExp(`(^|['"\`(=\\s])${SLASH}tmp${SLASH}`);

const listed = spawnSync('git', ['ls-files', '-z'], { cwd: repoRoot, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
assert.equal(listed.status, 0, `git ls-files: ${listed.stderr}`);
const files = listed.stdout.split('\0').filter(Boolean);
assert.ok(files.length > 100, `expected a populated checkout, saw ${files.length} files`);

const findings: string[] = [];
let scanned = 0;

for (const file of files) {
    if (IGNORED.some((pattern) => pattern.test(file))) continue;
    if (ALLOWED.some((pattern) => pattern.test(file))) continue;
    const extension = file.slice(file.lastIndexOf('.') + 1);
    if (!EXTENSIONS.includes(extension)) continue;

    let text: string;
    try {
        text = readFileSync(resolve(repoRoot, file), 'utf8');
    } catch {
        continue; // Listed but not checked out, e.g. a sparse checkout.
    }
    scanned += 1;
    const lines = text.split(/\r?\n/);
    for (let index = 0; index < lines.length; index += 1) {
        const line = lines[index];
        const pattern = [DRIVE, POSIX, TEMP].find((candidate) => candidate.test(line));
        if (pattern) findings.push(`${file}:${index + 1}: ${line.trim().slice(0, 160)}`);
    }
}

assert.equal(
    findings.length,
    0,
    `absolute paths belong in web/.env, not in the code:\n${findings.join('\n')}`,
);
console.log(`PASS no absolute paths in ${scanned} tracked text files`);
