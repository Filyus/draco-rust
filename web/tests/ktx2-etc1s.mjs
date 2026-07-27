/**
 * Our ETC1S transcoder against Binomial's, byte for byte, every mip level.
 *
 * ETC1S is the half of Basis Universal that carries almost every KTX2 texture
 * in a glTF file, and it is not a block format that can be checked block by
 * block: the whole image is one entropy-coded stream of codebook indices, each
 * predicted from its neighbours. A single bit read in the wrong order does not
 * corrupt one block, it derails everything after it — which is exactly why the
 * comparison is the entire image and not a sample of it.
 *
 * Every level, not just the base: the small levels are where the coder's edge
 * cases live. A 2×2 image is one block with twelve pixels hanging off its
 * edges, and the last three levels of a 1024² texture are four bytes each.
 */
import assert from 'node:assert/strict';

import { firstDifference, loadKtx2Module, loadReference, TARGET } from './ktx2-reference.mjs';

/** ETC1S fixtures, with and without an alpha slice. */
const FILES = ['facecap', '2d_etc1s', 'sample_etc1s'];

const reference = await loadReference();
if (!reference) {
  console.log('ktx2-etc1s: SKIPPED (the reference Basis transcoder is not on this machine)');
  process.exit(0);
}

const ktx2 = await loadKtx2Module();
let compared = 0;

for (const name of FILES) {
  const bytes = await import('node:fs/promises')
    .then(({ readFile }) => readFile(new URL(`../../testdata/ktx2/${name}.ktx2`, import.meta.url)));
  const file = new ktx2.Ktx2File(new Uint8Array(bytes));

  assert.equal(file.codec, 'etc1s', `${name} should be read as ETC1S`);
  assert.equal(file.levels, await reference.levels(name), `${name} level count`);

  for (let level = 0; level < file.levels; level++) {
    const want = await reference.transcode(name, level, TARGET.RGBA32);
    const image = file.decodeRgba(level);
    const got = image.rgba();

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

console.log(`ktx2-etc1s: ${compared} mip levels match the reference transcoder byte for byte`);
