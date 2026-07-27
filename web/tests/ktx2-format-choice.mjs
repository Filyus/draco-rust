/**
 * Which block format a texture is transcoded into, given what the GPU offers.
 *
 * The decision is made once per texture from two facts — the machine's
 * extension list and the file's own codec and alpha — and it is pure, so it
 * can be checked here rather than only in a browser that happens to have the
 * extensions the case needs. A phone's answers cannot be observed on a desktop
 * at all, which is exactly why they are worth writing down.
 *
 * The failure this guards is a wrong choice rather than a crash: BC1 for a
 * texture with alpha would drop the alpha silently, and BC3 for one without
 * would double its video memory for nothing.
 */
import assert from 'node:assert/strict';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const { chooseCompressedTarget } = await import(
  pathToFileURL(resolve(here, '..', 'www', 'viewer', 'compressed-formats.js')).href
);

const S3TC = ['WEBGL_compressed_texture_s3tc'];
const BPTC = ['EXT_texture_compression_bptc'];
const ETC = ['WEBGL_compressed_texture_etc'];
const ASTC = ['WEBGL_compressed_texture_astc'];
/** What a phone reports: no BC family at all.  */
const MOBILE = [...ETC, ...ASTC];

const name = (target) => (target ? target.name : 'pixels');

const CASES = [
  // A desktop GPU: the BC family and nothing else. This is what was measured
  // on Chrome with an NVIDIA card, and it is the case that matters most.
  [S3TC, 'etc1s', false, 'bc1', 'without alpha, BC1 is half the memory of BC3 and loses nothing'],
  [S3TC, 'etc1s', true, 'bc3', 'with alpha, only BC3 can carry it'],
  // UASTC goes to BC7 and nowhere else: BC1 or BC3 would throw away the
  // precision it is chosen for in the first place.
  [BPTC, 'uastc', false, 'bc7', 'bptc is what UASTC needs'],
  [BPTC, 'uastc', true, 'bc7', 'BC7 carries alpha, so the answer does not change'],
  [S3TC, 'uastc', false, 'pixels', 'without bptc there is nothing for UASTC to become'],
  [BPTC, 'etc1s', false, 'pixels', 'ETC1S has no BC7 path, and bptc alone offers nothing else'],
  // A phone, which is where the whole ETC and ASTC question comes from: no BC
  // family, so before these targets existed every one of these was pixels.
  [MOBILE, 'etc1s', false, 'etc1', 'ETC1 is the cheapest thing ETC1S can be on a phone'],
  [MOBILE, 'etc1s', true, 'etc2', 'alpha needs ETC2 and its EAC block'],
  [MOBILE, 'uastc', false, 'astc', 'ASTC is the format UASTC is a profile of, so nothing is lost'],
  [MOBILE, 'uastc', true, 'astc', 'and it carries alpha too'],
  [ETC, 'etc1s', false, 'etc1', 'ETC alone is enough for an ETC1S texture'],
  [ETC, 'uastc', false, 'pixels', 'UASTC has no ETC target, so ETC alone leaves it as pixels'],
  [ASTC, 'etc1s', false, 'pixels', 'and ETC1S has no ASTC target'],
  // No compressed format at all.
  [[], 'etc1s', false, 'pixels', 'no compressed format at all'],
  [[], 'uastc', true, 'pixels', 'nor for UASTC'],
];

for (const [extensions, codec, hasAlpha, expected, why] of CASES) {
  const chosen = chooseCompressedTarget(extensions, codec, hasAlpha);
  assert.equal(name(chosen), expected, `${codec}${hasAlpha ? ' with alpha' : ''} on [${extensions}]: ${why}`);
}

// The block size has to match the format, because the upload is sized by it.
assert.equal(chooseCompressedTarget(S3TC, 'etc1s', false).bytesPerBlock, 8);
assert.equal(chooseCompressedTarget(S3TC, 'etc1s', true).bytesPerBlock, 16);
assert.equal(chooseCompressedTarget(BPTC, 'uastc', false).bytesPerBlock, 16);
assert.equal(chooseCompressedTarget(MOBILE, 'etc1s', false).bytesPerBlock, 8);
assert.equal(chooseCompressedTarget(MOBILE, 'uastc', false).bytesPerBlock, 16);

console.log(`ktx2-format-choice: ${CASES.length} cases OK`);
