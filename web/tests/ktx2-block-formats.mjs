/**
 * Our block-format transcoding against Binomial's, byte for byte.
 *
 * Decoding to pixels and transcoding to a block format are different problems.
 * Pixels are read back and compared; blocks are handed straight to the GPU,
 * and a block that is wrong in a way the format still accepts renders as
 * something plausible that nobody will trace back to the transcoder. So the
 * check is the same as for pixels — the whole image, every level, against the
 * reference — but it matters more here, not less.
 *
 * ETC1S to BC1 is a search rather than a conversion: the two formats space
 * their four colours differently, so every block is answered from a table of
 * precomputed nearest endpoint pairs. Agreeing with the reference to the byte
 * is what says the tables and the six branches around them are all right.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import {
  FIXTURES,
  firstDifference,
  loadKtx2Module,
  loadReference,
  TARGET,
} from './ktx2-reference.ts';

/** Which of our targets to compare, and what the reference calls it. */
const PAIRS = [
  { codec: 'etc1s', target: 'bc1', reference: TARGET.BC1_RGB, bytesPerBlock: 8, files: ['facecap', '2d_etc1s', 'sample_etc1s'] },
  { codec: 'etc1s', target: 'bc3', reference: TARGET.BC3_RGBA, bytesPerBlock: 16, files: ['facecap', '2d_etc1s', 'sample_etc1s'] },
  { codec: 'uastc', target: 'bc7', reference: TARGET.BC7_RGBA, bytesPerBlock: 16, files: ['2d_uastc', 'sample_uastc_zstd'] },
  { codec: 'etc1s', target: 'etc1', reference: TARGET.ETC1_RGB, bytesPerBlock: 8, files: ['facecap', '2d_etc1s', 'sample_etc1s'] },
  { codec: 'etc1s', target: 'etc2', reference: TARGET.ETC2_RGBA, bytesPerBlock: 16, files: ['facecap', '2d_etc1s', 'sample_etc1s'] },
  { codec: 'uastc', target: 'astc', reference: TARGET.ASTC_4x4_RGBA, bytesPerBlock: 16, files: ['2d_uastc', 'sample_uastc_zstd'] },
  { codec: 'uastc', target: 'etc1', reference: TARGET.ETC1_RGB, bytesPerBlock: 8, files: ['2d_uastc', 'sample_uastc_zstd'] },
  { codec: 'uastc', target: 'etc2', reference: TARGET.ETC2_RGBA, bytesPerBlock: 16, files: ['2d_uastc', 'sample_uastc_zstd'] },
  { codec: 'etc1s', target: 'astc', reference: TARGET.ASTC_4x4_RGBA, bytesPerBlock: 16, files: ['facecap', '2d_etc1s', 'sample_etc1s'] },
];

const reference = await loadReference();
if (!reference) {
  console.log('ktx2-block-formats: SKIPPED (the reference Basis transcoder is not on this machine)');
  process.exit(0);
}

const ktx2 = await loadKtx2Module();
let compared = 0;

for (const pair of PAIRS) {
  for (const name of pair.files) {
    const bytes = await readFile(resolve(FIXTURES, `${name}.ktx2`));
    const file = new ktx2.Ktx2File(new Uint8Array(bytes));
    assert.equal(file.codec, pair.codec, `${name} should be read as ${pair.codec}`);

    for (let level = 0; level < file.levels; level++) {
      const want = await reference.transcode(name, level, pair.reference);
      const image = file.decode(level, pair.target);
      const got = image.bytes();

      const blocks = Math.ceil(image.width / 4) * Math.ceil(image.height / 4);
      assert.equal(
        got.length,
        blocks * pair.bytesPerBlock,
        `${name} level ${level} is not one ${pair.bytesPerBlock}-byte block per 4×4 texels`,
      );
      const difference = firstDifference(want, got);
      assert.equal(difference, null, `${name} ${pair.target} level ${level}: ${difference}`);
      compared++;
    }
  }
}

console.log(`ktx2-block-formats: ${compared} mip levels match the reference transcoder byte for byte`);
