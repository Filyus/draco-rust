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
import { compareAllLevels, loadKtx2Module, loadReference } from './ktx2-reference.mjs';

const FILES = ['2d_uastc', 'sample_uastc_zstd'];

const reference = await loadReference();
if (!reference) {
  console.log('ktx2-uastc: SKIPPED (the reference Basis transcoder is not on this machine)');
  process.exit(0);
}

const ktx2 = await loadKtx2Module();
const compared = await compareAllLevels(ktx2, reference, FILES, 'uastc');

console.log(`ktx2-uastc: ${compared} mip levels match the reference transcoder byte for byte`);
