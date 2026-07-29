/**
 * The UASTC block modes no fixture contains.
 *
 * Five of nineteen — 2, 3, 7, 14 and 18 — appear in no KTX2 file available
 * here, so the code that decodes them was written from the reference and
 * checked by nothing. Waiting for an asset that happens to use them is not a
 * plan; a UASTC encoder picks modes by what compresses the image in front of
 * it, and a natural photograph never reaches for a three-subset block.
 *
 * So the blocks are built instead. A UASTC block has no checksum and no
 * redundancy: any 128 bits whose leading code names a mode and whose pattern
 * index exists is a legal block of that mode. That makes random bits, with
 * those two fields fixed, a fair sample of the mode — and the reference is
 * still the oracle, so this asks the same question the other gates do rather
 * than a weaker one.
 *
 * The blocks go into a real file, by overwriting the levels of a fixture whose
 * container is already what we want: same dimensions, same mip chain, same
 * uncompressed UASTC payload, different contents.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { zstdCompressSync } from 'node:zlib';

import { FIXTURES, TARGET, firstDifference, loadKtx2Module, loadReference } from './ktx2-reference.ts';

/** Which mode each of the 128 leading bit patterns selects. */
const HUFF_MODES = [
  11, 0, 10, 3, 11, 15, 12, 7, 11, 18, 10, 5, 11, 14, 12, 9, 11, 0, 10, 4, 11, 16, 12, 8, 11, 18,
  10, 6, 11, 2, 12, 13, 11, 0, 10, 3, 11, 17, 12, 7, 11, 18, 10, 5, 11, 14, 12, 9, 11, 0, 10, 4,
  11, 1, 12, 8, 11, 18, 10, 6, 11, 2, 12, 13, 11, 0, 10, 3, 11, 19, 12, 7, 11, 18, 10, 5, 11, 14,
  12, 9, 11, 0, 10, 4, 11, 16, 12, 8, 11, 18, 10, 6, 11, 2, 12, 13, 11, 0, 10, 3, 11, 17, 12, 7,
  11, 18, 10, 5, 11, 14, 12, 9, 11, 0, 10, 4, 11, 1, 12, 8, 11, 18, 10, 6, 11, 2, 12, 13,
];

/**
 * Where a mode's pattern index sits and how many values it has.
 *
 * The field follows the mode code and the encoder hints, both of which are
 * fixed widths per mode, so its offset is arithmetic rather than a search. A
 * mode with one subset has no pattern at all.
 *
 * Getting either number wrong is caught rather than hidden: too large a count
 * produces a pattern the decoder rejects, and a wrong offset produces a block
 * whose bytes the reference reads differently, which the comparison shows.
 */
const PATTERN = {
  2: { offset: 5 + 15, bits: 5, count: 30 },
  3: { offset: 5 + 15, bits: 4, count: 11 },
  7: { offset: 5 + 15, bits: 5, count: 19 },
  14: null,
  18: null,
};

const MODES = [2, 3, 7, 14, 18];

/** Every target either the file's codec can reach. */
const TARGETS = [
  { name: 'rgba8', reference: TARGET.RGBA32, bytesPerBlock: null },
  { name: 'bc7', reference: TARGET.BC7_RGBA, bytesPerBlock: 16 },
  { name: 'astc', reference: TARGET.ASTC_4x4_RGBA, bytesPerBlock: 16 },
  { name: 'etc1', reference: TARGET.ETC1_RGB, bytesPerBlock: 8 },
  { name: 'etc2', reference: TARGET.ETC2_RGBA, bytesPerBlock: 16 },
];

/** A fixed stream, so a failure is reproducible rather than a one-off. */
function randomBits(seed) {
  let state = seed >>> 0;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    state >>>= 0;
    return state;
  };
}

/** Write `count` bits at `offset`, least significant first, as the format reads them. */
function setBits(block, offset, count, value) {
  for (let bit = 0; bit < count; bit++) {
    const at = offset + bit;
    const mask = 1 << (at & 7);
    if ((value >>> bit) & 1) block[at >> 3] |= mask;
    else block[at >> 3] &= ~mask;
  }
}

/** One legal block of the given mode, filled otherwise at random. */
function buildBlock(mode, next) {
  const block = new Uint8Array(16);
  for (let index = 0; index < 16; index++) block[index] = next() & 0xff;

  // The mode is selected by the low seven bits, so those are not free.
  const prefix = HUFF_MODES.indexOf(mode);
  assert.notEqual(prefix, -1, `no leading bit pattern selects mode ${mode}`);
  setBits(block, 0, 7, prefix);

  const pattern = PATTERN[mode];
  if (pattern) setBits(block, pattern.offset, pattern.bits, next() % pattern.count);
  return block;
}

/** Read the level index of a KTX2 file: where each level's bytes are. */
function levelIndex(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const levels = Math.max(1, view.getUint32(40, true));
  assert.equal(view.getUint32(44, true), 2, 'this gate expects a Zstd-supercompressed fixture');
  const index = [];
  for (let level = 0; level < levels; level++) {
    const at = 80 + level * 24;
    index.push({
      offset: Number(view.getBigUint64(at, true)),
      length: Number(view.getBigUint64(at + 8, true)),
      uncompressed: Number(view.getBigUint64(at + 16, true)),
    });
  }
  return index;
}

/**
 * The same file with every level's blocks replaced.
 *
 * The levels are Zstd, so this is not a patch in place: each one is built,
 * compressed, and the index rewritten around the new sizes. Everything ahead
 * of the payload — the header, the format description, the key/value data —
 * is carried through untouched, which is the point. The container stays a real
 * one and only what the blocks say changes.
 */
function rebuild(original, index, build) {
  const storage = index.map((level, at) => ({ ...level, at })).sort((a, b) => a.offset - b.offset);
  const base = storage[0].offset;
  const payloads = new Map();
  let cursor = base;
  for (const level of storage) {
    const raw = new Uint8Array(level.uncompressed);
    for (let at = 0; at < raw.length; at += 16) raw.set(build(), at);
    const packed = new Uint8Array(zstdCompressSync(raw));
    payloads.set(level.at, { offset: cursor, packed, uncompressed: raw.length });
    cursor += packed.length;
  }

  const bytes = new Uint8Array(cursor);
  bytes.set(original.subarray(0, base));
  const view = new DataView(bytes.buffer);
  for (const [level, { offset, packed, uncompressed }] of payloads) {
    bytes.set(packed, offset);
    const at = 80 + level * 24;
    view.setBigUint64(at, BigInt(offset), true);
    view.setBigUint64(at + 8, BigInt(packed.length), true);
    view.setBigUint64(at + 16, BigInt(uncompressed), true);
  }
  return bytes;
}

const reference = await loadReference();
if (!reference) {
  console.log('ktx2-uastc-modes: SKIPPED (the reference Basis transcoder is not on this machine)');
  process.exit(0);
}

const ktx2 = await loadKtx2Module();
const original = new Uint8Array(await readFile(resolve(FIXTURES, '2d_uastc.ktx2')));
const index = levelIndex(original);

let compared = 0;
let blocks = 0;

for (const mode of MODES) {
  // One stream per mode, so adding a mode does not renumber another's blocks.
  const next = randomBits(0x9e3779b9 ^ (mode * 2654435761));
  const bytes = rebuild(original, index, () => {
    blocks++;
    return buildBlock(mode, next);
  });

  const file = new ktx2.Ktx2File(bytes);
  const census = file.modeCensus();
  assert.equal(
    census.reduce((total, count, at) => (at === mode ? total : total + count), 0),
    0,
    `every block of the built file should be mode ${mode}`,
  );
  assert.ok(census[mode] > 0, `the built file should hold mode ${mode} blocks`);

  for (let level = 0; level < file.levels; level++) {
    for (const target of TARGETS) {
      const want = reference.transcodeBytes(bytes, level, target.reference, `mode ${mode}`);
      const image = file.decode(level, target.name);
      const got = image.bytes();
      if (target.bytesPerBlock) {
        const count = Math.ceil(image.width / 4) * Math.ceil(image.height / 4);
        assert.equal(
          got.length,
          count * target.bytesPerBlock,
          `mode ${mode} ${target.name} level ${level} is not one block per 4×4 texels`,
        );
      }
      const difference = firstDifference(want, got);
      assert.equal(difference, null, `mode ${mode} ${target.name} level ${level}: ${difference}`);
      compared++;
    }
  }
}

console.log(
  `ktx2-uastc-modes: modes ${MODES.join(',')} exercised over ${blocks} built blocks, `
  + `${compared} images match the reference transcoder byte for byte`,
);
