/**
 * No pointers from the repository into the local-only development tree.
 *
 * The working notes, profiling harnesses and vendored corpora at the top of the
 * repository are excluded from git in `.git/info/exclude`, so `git ls-files`
 * never lists them and a clone never receives them. Nineteen tracked files
 * still cited paths inside that tree: doc comments saying "see this note",
 * measurement attributions naming a script, markdown links, and `path:` fields
 * in the machine-readable status files. Every one of them resolved on exactly
 * one computer and nowhere else -- the same shape as a committed absolute path,
 * which is why this sits beside `no-absolute-paths.ts`.
 *
 * One of those citations pointed at a note that had never been written at all.
 * Nothing could have noticed: a reference to a file outside the repository
 * looks identical whether the file exists or not.
 *
 * The pointers run the other way now. The excluded tree may name tracked files
 * freely -- that direction always resolves -- and it carries its own index of
 * what relates to what. This checks that the reverse never comes back.
 *
 * A bare tool name in prose is out of scope: it does not promise a reader they
 * can open it. What is checked is the path form, which does.
 *
 * The pattern is assembled from fragments rather than written out, so this file
 * is not itself an exception to what it checks.
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
    'cpp', 'cc', 'h', 'hpp',
];

/** Generated or vendored trees: not ours to hold to this. */
const IGNORED = [/(^|\/)node_modules\//, /^web\/www\/pkg\//, /(^|\/)target\//];

const SLASH = String.fromCharCode(47);

/** The excluded directory's name, never written as a path in this file. */
const TREE = 'd' + 'ev';

/**
 * The tree's name used as a path prefix.
 *
 * Anchored on the left by something that is not a word character or a path
 * separator, so a device node redirect keeps working and a longer word ending
 * in the same three letters is not a hit. Anchored on the right by a second
 * separator or a file extension, so prose naming two build profiles at once is
 * not a hit either -- what is banned is a path that continues, because only
 * that claims a file a reader could open.
 */
const REFERENCE = new RegExp(
    `(^|[^\\w${SLASH}.-])${TREE}${SLASH}[\\w.-]+[${SLASH}.]`,
);

const listed = spawnSync('git', ['ls-files', '-z'], { cwd: repoRoot, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
assert.equal(listed.status, 0, `git ls-files: ${listed.stderr}`);
const files = listed.stdout.split('\0').filter(Boolean);
assert.ok(files.length > 100, `expected a populated checkout, saw ${files.length} files`);

const findings: string[] = [];
let scanned = 0;

for (const file of files) {
    if (IGNORED.some((pattern) => pattern.test(file))) continue;
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
        if (REFERENCE.test(line)) findings.push(`${file}:${index + 1}: ${line.trim().slice(0, 160)}`);
    }
}

assert.equal(
    findings.length,
    0,
    `the ${TREE} tree is not in the repository, so nothing tracked can point into it.\n`
        + `State the fact inline, or record the association in that tree's own index:\n${findings.join('\n')}`,
);
console.log(`PASS no ${TREE} tree paths in ${scanned} tracked text files`);
