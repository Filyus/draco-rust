/**
 * Our UASTC transcoder against Binomial's, byte for byte, every mip level.
 *
 * UASTC is the other half of `KHR_texture_basisu`, and it fails differently
 * from ETC1S. Each block stands alone, so a mistake stays local — one wrong
 * mode table entry corrupts only the blocks that use that mode, and on a
 * photograph that can be a handful of texels nobody would spot by looking.
 * Nineteen modes, three partition tables and the ASTC integer sequence
 * encoding is a lot of surface to be quietly slightly wrong on, so the check
 * is every byte rather than an eyeball.
 *
 * `sample_uastc_zstd` is the awkward one on purpose: 1000×1392 is a multiple
 * of neither four nor two, so its right and bottom blocks hang over the edge,
 * and it is Zstd supercompressed, which no ETC1S file is.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import {
  compareAllLevels,
  FIXTURES,
  loadKtx2Module,
  loadReference,
} from './ktx2-reference.mjs';

const FILES = ['2d_uastc', 'sample_uastc_zstd'];

const reference = await loadReference();
if (!reference) {
  console.log('ktx2-uastc: SKIPPED (the reference Basis transcoder is not on this machine)');
  process.exit(0);
}

const ktx2 = await loadKtx2Module();
const compared = await compareAllLevels(ktx2, reference, FILES, 'uastc');

// Which of the nineteen modes these files actually use, and therefore which
// of them any of this verifies. A natural image is nowhere near all of them:
// mode 8 is a flat block and dominates, while several modes an encoder emits
// only for particular content never appear at all. Recording the set here is
// the difference between a known gap and an unnoticed one, and a fixture that
// closes part of it will fail this line rather than pass unnoticed.
const census = new Array(20).fill(0);
for (const name of FILES) {
  const bytes = await readFile(resolve(FIXTURES, `${name}.ktx2`));
  const counts = new ktx2.Ktx2File(new Uint8Array(bytes)).modeCensus();
  counts.forEach((count, mode) => { census[mode] += count; });
}

const covered = census.flatMap((count, mode) => (count > 0 ? [mode] : []));
const missing = [...Array(19).keys()].filter((mode) => !covered.includes(mode));

assert.deepEqual(
  covered,
  [0, 1, 4, 5, 6, 8, 9, 10, 11, 12, 13, 15, 16, 17],
  'the modes these fixtures exercise changed; if a fixture was added, widen this',
);
assert.deepEqual(
  missing,
  [2, 3, 7, 14, 18],
  'the unverified modes changed; every one of these is written from the reference and checked by nothing',
);

console.log(`ktx2-uastc: ${compared} mip levels match the reference transcoder byte for byte`);
console.log(`ktx2-uastc: modes ${covered.join(',')} exercised; ${missing.join(',')} unverified`);
