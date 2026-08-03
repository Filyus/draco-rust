/**
 * No `font-size` in the panel stylesheet reads smaller than 0.7rem, except a
 * documented allowlist.
 *
 * The floor came from a specific complaint -- the quantization legend and
 * hint text sat at 0.65rem, smaller than everything readable around them --
 * and fixing only the two flagged rules would have left every other rule at
 * or below that size for the next person to notice one at a time. A handful
 * of rules are legitimately smaller: an uppercase, letter-spaced section
 * title reads fine well under 0.7rem because the tracking does the work a
 * larger size would otherwise do, a badge holding one short number is a
 * shape to recognize rather than a sentence to read, and a monospace log is
 * expected to run dense. Those are named in ALLOWLIST with the reason, so a
 * new violation still fails here and an allowlisted selector that stops
 * existing is caught too -- an unused entry hides the day it comes back
 * smaller than intended.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const css = await readFile(resolve(here, '..', 'www', 'style.css'), 'utf8');

const MIN_REM = 0.7;

const ALLOWLIST: Record<string, string> = {
  '.card h2': 'uppercase, letter-spaced section header -- tracking compensates for size',
  '.scene-panel-title': 'uppercase, letter-spaced section header -- tracking compensates for size',
  '.scene-info-title': 'uppercase, letter-spaced section header -- tracking compensates for size',
  '.file-model-caption': 'uppercase, letter-spaced caption',
  '.scene-variant-caption': 'uppercase, letter-spaced caption',
  '.scene-tree-orphans-title': 'uppercase, letter-spaced subsection title',
  '.scene-tree-badge': 'compact inline badge, recognized by shape more than read',
  '.scene-warning-index': 'a one- or two-digit number inside a 17px circle',
  '.scene-warnings-count': 'compact count pill',
  '.console-output': 'monospace log output, expected to run dense',
};

// Comments are stripped first so a rem value mentioned only in prose (this
// file's own doc comments included, if this test is ever quoted there) is
// never mistaken for a live declaration.
const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, '');

/**
 * One rule: everything up to its first `{` as the selector list, then its
 * declarations up to the next `}`. This stylesheet nests braces only inside
 * `@keyframes`/`@media` wrappers, never inside an ordinary rule, so a match
 * that assumes no nesting still lands on every real rule -- an at-rule
 * wrapper simply fails to match as a unit and the scan advances past it to
 * the rules inside, the same way it already skips right past comments.
 */
const RULE = /([^{}]+)\{([^{}]*)\}/g;

const violations: string[] = [];
const seenAllowlisted = new Set<string>();

for (const match of withoutComments.matchAll(RULE)) {
  const [, rawSelector, body] = match;
  const sizeMatch = body.match(/font-size:\s*([\d.]+)(rem|em|px)\b/);
  if (!sizeMatch) continue;
  const [, value, unit] = sizeMatch;
  const rem = unit === 'px' ? Number(value) / 16 : Number(value);
  if (rem >= MIN_REM) continue;

  const selectors = rawSelector.split(',').map((s) => s.trim()).filter(Boolean);
  const allowlisted = selectors.find((selector) => selector in ALLOWLIST);
  if (allowlisted) {
    seenAllowlisted.add(allowlisted);
    continue;
  }
  violations.push(`${selectors.join(', ')} -> font-size: ${value}${unit} (${rem.toFixed(3)}rem, floor is ${MIN_REM}rem)`);
}

assert.equal(
  violations.length,
  0,
  `font-size below the ${MIN_REM}rem legibility floor with no ALLOWLIST entry in tests/style-legibility.ts:\n`
  + violations.join('\n'),
);

for (const [selector, reason] of Object.entries(ALLOWLIST)) {
  assert.ok(
    seenAllowlisted.has(selector),
    `ALLOWLIST entry "${selector}" (${reason}) matched nothing under ${MIN_REM}rem in style.css -- `
    + 'the rule was removed, renamed, or is no longer small, so remove this entry',
  );
}

console.log(`style-legibility: no font-size below ${MIN_REM}rem outside the documented allowlist`);
