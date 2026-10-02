// Stamps a built site with its build id, so a browser holds one build's files
// together: `node scripts/stamp-build.mjs <site-dir> <build-id>`.
//
// The page loads its code as ES modules that import each other by relative
// path, and the per-format wasm modules on demand. Served with a cache
// lifetime and no say over its headers, as GitHub Pages serves it, a page
// cached from one deploy could run its scripts against the modules of the next
// and fail inside the wasm. So `index.html` gets:
//
// - `<meta name="build-id">`, which `src/app/modules.ts` puts on the wasm
//   modules' URLs;
// - an import map giving every script of the site the same `?v=` query, which
//   reaches the imports an entry script's query cannot;
// - that query on the entry script and the stylesheet.
//
// A page and everything it loads then share one query: all of it is cached or
// all of it is fetched. The dev server serves the site unstamped.

import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';

const [siteDir, buildId] = process.argv.slice(2);
if (!siteDir || !buildId || !/^[0-9A-Za-z._-]+$/.test(buildId)) {
  console.error('usage: node scripts/stamp-build.mjs <site-dir> <build-id>');
  process.exit(2);
}

function scripts(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return scripts(path);
    return name.endsWith('.js') ? [path] : [];
  });
}

const query = `?v=${buildId}`;
const imports = Object.fromEntries(
  scripts(siteDir)
    .map((path) => `./${relative(siteDir, path).split(sep).join('/')}`)
    .sort()
    .map((url) => [url, `${url}${query}`]),
);

const indexPath = join(siteDir, 'index.html');
let html = readFileSync(indexPath, 'utf8');

// Each anchor must be there exactly once: a page that changed shape fails the
// deploy rather than going out half stamped.
function replaceOnce(anchor, replacement) {
  const count = html.split(anchor).length - 1;
  if (count !== 1) {
    console.error(`index.html has ${count} of ${JSON.stringify(anchor)}, not one`);
    process.exit(1);
  }
  html = html.replace(anchor, replacement);
}

replaceOnce(
  '<meta charset="UTF-8">',
  `<meta charset="UTF-8">\n    <meta name="build-id" content="${buildId}">`,
);
replaceOnce('<link rel="stylesheet" href="style.css">', `<link rel="stylesheet" href="style.css${query}">`);
// The import map has to precede every module script and import.
replaceOnce(
  '<script type="module" src="app.js"></script>',
  `<script type="importmap">${JSON.stringify({ imports })}</script>\n` +
    `    <script type="module" src="app.js${query}"></script>`,
);

writeFileSync(indexPath, html);
console.log(`stamped ${indexPath} with ${buildId}: ${Object.keys(imports).length} scripts mapped`);
