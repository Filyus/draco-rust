/**
 * What a selection has to be read as, before anything is opened.
 *
 * Two things were wrong and both were invisible while a selection meant a few
 * files picked by hand: every file was read whether the model named it or not,
 * and each landed under its bare filename. The first makes a dropped folder
 * unaffordable; the second makes any document that writes a path — a
 * `textures/` subfolder, a `../glTF/mesh.bin` — fail to find its own
 * companions, because the resolver looks the URI up as it was written.
 *
 * The fixtures here are the real ones on disk, read through a stand-in for
 * `File` that counts its reads: the count is the assertion that nothing extra
 * was opened.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');

const { findModels, readModel, resolveUriPath } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'app', 'model-intake.ts')).href
);

/** A `File` that remembers whether anyone asked for its bytes. */
const opened = [];
function entry(path, diskPath) {
  const name = path.split('/').pop();
  return {
    path,
    file: {
      name,
      size: 0,
      async arrayBuffer() {
        opened.push(path);
        const bytes = await readFile(diskPath ?? resolve(repoRoot, path));
        return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
      },
    },
  };
}

// ---- where a written URI points -------------------------------------------
assert.equal(resolveUriPath('glTF', 'mesh.bin'), 'glTF/mesh.bin');
assert.equal(resolveUriPath('glTF', 'textures/wood.png'), 'glTF/textures/wood.png');
// The case a file picker could not supply, and the reason this exists.
assert.equal(resolveUriPath('glTF-instancing', '../glTF/DamagedHelmet.bin'), 'glTF/DamagedHelmet.bin');
assert.equal(resolveUriPath('a/b', '../../c/d.bin'), 'c/d.bin');
assert.equal(resolveUriPath('', './mesh.bin'), 'mesh.bin');
// Above the root there is nothing to find, because nothing above it was given.
assert.equal(resolveUriPath('glTF', '../../escape.bin'), null);
assert.equal(resolveUriPath('', '../escape.bin'), null);
// A percent-encoded URI names the same file as the plain one.
assert.equal(resolveUriPath('glTF', 'two%20words.bin'), 'glTF/two words.bin');

// ---- which file in a folder is the model ----------------------------------
const folder = [
  entry('glTF/DamagedHelmet.gltf'),
  entry('glTF/DamagedHelmet.bin'),
  entry('glTF/Default_albedo.jpg'),
  entry('glTF-instancing/DamagedHelmetGpuInstancing.gltf'),
  entry('README.md'),
];
const found = findModels(folder);
assert.deepEqual(
  found.map((candidate) => candidate.path),
  ['glTF/DamagedHelmet.gltf', 'glTF-instancing/DamagedHelmetGpuInstancing.gltf'],
  'both models are offered, and the shorter path leads',
);
assert.deepEqual(findModels([entry('README.md')]), [], 'nothing to open is not something to open');

// A folder holding one scene several ways puts the plain variant first.
const variants = ['glTF-Binary/X.glb', 'glTF-Draco/X.gltf', 'glTF/X.gltf', 'glTF-KTX-BasisU/X.gltf'];
assert.equal(findModels(variants.map((path) => entry(path)))[0].path, 'glTF/X.gltf');

// ---- reading only what the document names ---------------------------------
const textures = resolve(repoRoot, 'testdata', 'textures');
const selection = [
  entry('quadrants-webp.gltf', resolve(textures, 'quadrants-webp.gltf')),
  entry('quadrants.webp', resolve(textures, 'quadrants.webp')),
  // Named by nothing, and therefore never opened.
  entry('quadrants.avif', resolve(textures, 'quadrants.avif')),
  entry('quadrants.png', resolve(textures, 'quadrants.png')),
];
opened.length = 0;
const read = await readModel(selection[0], selection);
assert.deepEqual(
  opened,
  ['quadrants-webp.gltf', 'quadrants.webp'],
  'the model and its one image, and nothing else in the selection',
);
assert.deepEqual(Object.keys(read.resources), ['quadrants.webp']);
assert.deepEqual(read.missing, []);
assert.ok(read.data.length > 0);

// A named file that is not there is reported rather than guessed at.
opened.length = 0;
const short = await readModel(selection[0], [selection[0]]);
assert.deepEqual(short.missing, ['quadrants.webp'], 'the URI the selection did not contain');
assert.deepEqual(Object.keys(short.resources), []);

// ---- the URI is the key, exactly as the document wrote it -----------------
// The Rust resolver looks it up as-is, so a nested path has to survive whole.
const nested = [
  entry('model.gltf', resolve(textures, 'quadrants-webp.gltf')),
  entry('quadrants.webp', resolve(textures, 'quadrants.webp')),
];
const flat = await readModel(nested[0], nested);
assert.ok('quadrants.webp' in flat.resources, 'keyed by the written URI, not by a normalized path');

// ---- OBJ needs a second round ---------------------------------------------
// The OBJ names a library and the library names the textures, so the images
// are unknowable until the .mtl itself has been read.
const objRoot = resolve(repoRoot, 'testdata');
const objSelection = [
  entry('mat_test.obj', resolve(objRoot, 'mat_test.obj')),
  entry('mat_test.mtl', resolve(objRoot, 'mat_test.mtl')),
  // Named by nothing in either file, so it must stay shut.
  entry('this_is_png.jpg', resolve(objRoot, 'this_is_png.jpg')),
];
opened.length = 0;
const obj = await readModel(objSelection[0], objSelection);
assert.deepEqual(opened, ['mat_test.obj', 'mat_test.mtl'], 'the model and its material library');
assert.ok('mat_test.mtl' in obj.resources);
// mat_test.mtl names black.png, which this selection does not hold. Reporting
// it is what proves the library was not merely fetched but read: without the
// second round there is nothing to be missing.
assert.deepEqual(obj.missing, ['black.png'], 'the texture the library names and the selection lacks');

// And with the texture present it is fetched, under the name the MTL used.
const objComplete = [...objSelection, entry('black.png', resolve(objRoot, 'test.png'))];
const complete = await readModel(objComplete[0], objComplete);
assert.deepEqual(complete.missing, []);
assert.ok('black.png' in complete.resources);

console.log('model-intake: OK');
