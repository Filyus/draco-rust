/**
 * The reference Basis transcoder, as ground truth for our own.
 *
 * Binomial's own build, the one three.js ships and every KTX2 texture on the
 * web is decoded by. Our transcoder is a Rust port of the same algorithm from
 * the same source, so "byte for byte" is the only useful standard: a
 * transcoder that is merely close produces a texture that looks right and is
 * wrong, and nothing downstream would ever notice.
 *
 * The same arrangement as the C++ Draco bridge — an independent implementation
 * to compare against, not a dependency. It lives outside this repository, so a
 * machine without it skips the gate rather than failing it, and says so.
 */
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

/** Where the three.js checkout keeps Binomial's build. */
const REFERENCE_DIR = 'D:/Projects/Three.ts/three.js-master/examples/jsm/libs/basis';

export const FIXTURES = resolve(here, '..', '..', 'testdata', 'ktx2');
export const PKG = resolve(here, '..', 'www', 'pkg');

/** The transcoder's own name for each output format. */
export const TARGET = {
  ETC1_RGB: 0,
  ETC2_RGBA: 1,
  BC1_RGB: 2,
  BC3_RGBA: 3,
  BC7_RGBA: 6,
  ASTC_4x4_RGBA: 10,
  RGBA32: 13,
};

/**
 * Load the reference transcoder, or explain why the gate cannot run.
 *
 * @returns {Promise<{ transcode(name: string, level: number, target: number): Uint8Array } | null>}
 */
export async function loadReference() {
  let wasmBinary;
  try {
    wasmBinary = await readFile(resolve(REFERENCE_DIR, 'basis_transcoder.wasm'));
  } catch {
    return null;
  }
  // Evaluated by hand rather than required. The transcoder is a CommonJS
  // file, but it sits inside a checkout whose package.json declares modules,
  // so node loads it as ESM and hands back an empty namespace.
  const source = await readFile(resolve(REFERENCE_DIR, 'basis_transcoder.js'), 'utf8');
  const scope = { exports: {} };
  new Function('module', 'exports', 'require', '__filename', '__dirname', source)(
    scope,
    scope.exports,
    createRequire(import.meta.url),
    resolve(REFERENCE_DIR, 'basis_transcoder.js'),
    REFERENCE_DIR,
  );
  const factory = scope.exports;
  const basis = await new Promise((done) => { factory({ wasmBinary }).then(done); });
  basis.initializeBasis();

  return {
    async transcode(name, level, target) {
      const bytes = new Uint8Array(await readFile(resolve(FIXTURES, `${name}.ktx2`)));
      const file = new basis.KTX2File(bytes);
      try {
        if (!file.isValid()) throw new Error(`the reference transcoder rejects ${name}.ktx2`);
        file.startTranscoding();
        const size = file.getImageTranscodedSizeInBytes(level, 0, 0, target);
        const out = new Uint8Array(size);
        if (!file.transcodeImage(out, level, 0, 0, target, 0, -1, -1)) {
          throw new Error(`the reference transcoder failed on ${name}.ktx2 level ${level}`);
        }
        return out;
      } finally {
        file.close();
        file.delete();
      }
    },
    async levels(name) {
      const bytes = new Uint8Array(await readFile(resolve(FIXTURES, `${name}.ktx2`)));
      const file = new basis.KTX2File(bytes);
      try {
        return file.getLevels();
      } finally {
        file.close();
        file.delete();
      }
    },
  };
}

/** Load our own transcoder out of the built WASM package. */
export async function loadKtx2Module() {
  const module = await import(new URL(`file://${resolve(PKG, 'ktx2.js').replace(/\\/g, '/')}`).href);
  await module.default({ module_or_path: await readFile(resolve(PKG, 'ktx2_bg.wasm')) });
  return module;
}

/**
 * Compare every mip level of every named file against the reference.
 *
 * One implementation for both codecs. What differs between ETC1S and UASTC is
 * which files to open and what the file should call itself; the check itself —
 * all of it, every level, byte for byte — is the same question either way, and
 * writing it twice would let the two drift.
 *
 * @returns {Promise<number>} how many levels were compared.
 */
export async function compareAllLevels(ktx2, reference, files, codec) {
  const assert = (await import('node:assert/strict')).default;
  const { readFile } = await import('node:fs/promises');
  let compared = 0;

  for (const name of files) {
    const bytes = await readFile(resolve(FIXTURES, `${name}.ktx2`));
    const file = new ktx2.Ktx2File(new Uint8Array(bytes));

    assert.equal(file.codec, codec, `${name} should be read as ${codec}`);
    assert.equal(file.levels, await reference.levels(name), `${name} level count`);

    for (let level = 0; level < file.levels; level++) {
      const want = await reference.transcode(name, level, TARGET.RGBA32);
      const image = file.decodeRgba(level);
      const got = image.bytes();

      assert.equal(
        got.length,
        image.width * image.height * 4,
        `${name} level ${level} is not width × height × 4 bytes`,
      );
      const difference = firstDifference(want, got);
      assert.equal(difference, null, `${name} level ${level}: ${difference}`);
      compared++;
    }
  }
  return compared;
}

/**
 * Where two byte strings first differ, worded so the failure names a pixel.
 *
 * @returns {string | null} null when they are identical.
 */
export function firstDifference(want, got) {
  if (want.length !== got.length) return `${want.length} bytes expected, ${got.length} produced`;
  for (let index = 0; index < want.length; index++) {
    if (want[index] !== got[index]) {
      const channel = 'RGBA'[index & 3];
      return `first differs at byte ${index} (pixel ${index >> 2}, channel ${channel}): `
        + `expected ${want[index]}, got ${got[index]}`;
    }
  }
  return null;
}
