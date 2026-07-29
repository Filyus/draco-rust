/**
 * The reader against the reference, on files nobody wrote.
 *
 * Everything else checking malformed input asks only that nothing panics. That
 * is the weak question. The strong one is whether this reader and Binomial's
 * agree about what a broken file means, and this project is unusually placed to
 * ask it: the reference transcoder is already here as an oracle.
 *
 * A reader that refuses a file the reference accepts loses a texture. One that
 * accepts a file the reference refuses is reading something nobody wrote. Both
 * are silent — no panic, no error — and nothing else here would notice.
 *
 * What is asserted is the sound half: **whenever both accept, the bytes must be
 * identical**. Acceptance itself diverges by design — this reader names plain
 * `vkFormat` files the reference refuses outright, and holds dimensions to its
 * own limit — so those are counted and reported rather than failed, with a
 * ceiling that a regression making the reader wildly more permissive would
 * break.
 *
 * One thing had to be worked around, and it is worth stating plainly because it
 * is a property of the oracle rather than of this reader. Fed a malformed file,
 * Binomial's module can be left in a state where `transcodeImage` returns
 * success and writes nothing: every later call gives back an all-zero image,
 * which looks exactly like this reader inventing pixels. It was found here on
 * the first run, and the standalone check that isolated it was decisive — the
 * same mutant compared clean from a fresh instance and all-zero from a used
 * one. So the pristine seed is transcoded again after every mutant, and a
 * mutant whose comparison came from a degraded instance is discarded rather
 * than believed.
 */
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { TARGET, firstDifference, loadKtx2Module, loadReference } from './ktx2-reference.ts';
import type { ReferenceTranscoder } from './ktx2-reference.ts';

const here = dirname(fileURLToPath(import.meta.url));
/** The small seeds, which are real files and cheap to transcode. */
const SEEDS = resolve(here, '..', '..', 'fuzz', 'seeds', 'ktx2_transcode');

/** Ours and the reference's name for each target, in one place. */
const TARGETS = [
  { name: 'rgba8', reference: TARGET.RGBA32 },
  { name: 'bc1', reference: TARGET.BC1_RGB },
  { name: 'etc1', reference: TARGET.ETC1_RGB },
  { name: 'astc', reference: TARGET.ASTC_4x4_RGBA },
];

/** How many mutants each seed gets. */
const ROUNDS = 400;

/** A fixed stream, so a disagreement is reproducible rather than a rumour. */
function randomBits(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>>= 0);
  };
}

interface KeyValueRange {
  start: number;
  end: number;
}

/**
 * The key/value data, which is out of scope and has to be left alone.
 *
 * It is the one section whose meaning the two readers do not share by design.
 * This reader reports what it finds there - orientation, and whatever else a
 * writer left - and transcodes the same bytes regardless; the reference acts
 * on it, so a garbled key/value section changes its output while leaving this
 * one's identical. Both are then right and different, which is noise rather
 * than a finding. Two mutants of one byte at offset 60 established that: with
 * only the key/value length changed, the reference's block-format output stayed
 * byte-identical to this reader's while its RGBA output did not.
 *
 * Its own range is still checked - `ktx2.rs` refuses a section reaching past
 * the end of the file, which the first version of this gate is what found.
 */
function keyValueRange(original: Uint8Array): KeyValueRange[] {
  const view = new DataView(original.buffer, original.byteOffset, original.byteLength);
  const offset = view.getUint32(56, true);
  const length = view.getUint32(60, true);
  // Both the section and the two header words that locate it: changing where
  // it is has the same effect as changing what it says.
  return [
    { start: 56, end: 64 },
    { start: offset, end: offset + length },
  ];
}

/**
 * One mutant of `original`.
 *
 * Three shapes, because they reach different code. A byte anywhere mostly
 * lands in the payload and exercises the codecs; a word in the header reaches
 * the parser, which is where the two readers most easily disagree; and a
 * truncation is the malformation that costs nothing to produce in the wild.
 */
function mutate(original: Uint8Array, next: () => number, keyValues: KeyValueRange[]): Uint8Array {
  const bytes = new Uint8Array(original);
  const inKeyValues = (at: number) => keyValues.some(({ start, end }) => at >= start && at < end);
  switch (next() % 3) {
    case 0: {
      const flips = 1 + (next() % 4);
      for (let index = 0; index < flips; index++) {
        const offset = next() % bytes.length;
        const value = next() & 0xff;
        if (!inKeyValues(offset)) bytes[offset] = value;
      }
      return bytes;
    }
    case 1: {
      // The header is 80 bytes and the level index 24 more per level. The two
      // words describing the key/value section are skipped along with it.
      const offset = (next() % 26) * 4;
      const extremes = [0, 1, 2, 0xffff, 0x7fffffff, 0xffffffff, next() & 0xffff];
      const value = extremes[next() % extremes.length];
      if (offset !== 56 && offset !== 60) {
        new DataView(bytes.buffer).setUint32(offset, value, true);
      }
      return bytes;
    }
    default:
      return bytes.subarray(0, 1 + (next() % bytes.length));
  }
}

/** What our reader makes of these bytes: null if it refuses them. */
function ours(ktx2: any, bytes: Uint8Array): Map<string, Uint8Array> | null {
  let file;
  try {
    file = new ktx2.Ktx2File(bytes);
  } catch {
    return null;
  }
  const images = new Map<string, Uint8Array>();
  for (const target of TARGETS) {
    try {
      images.set(target.name, file.decode(0, target.name).bytes());
    } catch {
      // A target this file's codec cannot reach, or a level that will not
      // decode. Either way it is a refusal of that image, not of the file.
    }
  }
  return images;
}

/** The same from the reference: null if it refuses the file or the image. */
function theirs(reference: ReferenceTranscoder, bytes: Uint8Array, target: number): Uint8Array | null {
  try {
    return reference.transcodeBytes(bytes, 0, target);
  } catch {
    return null;
  }
}

let reference = await loadReference();
if (!reference) {
  console.log('ktx2-differential: SKIPPED (the reference Basis transcoder is not on this machine)');
  process.exit(0);
}

const ktx2 = await loadKtx2Module();
const seeds = (await readdir(SEEDS)).filter((name) => name.endsWith('.ktx2')).sort();
assert.ok(seeds.length > 0, 'the seeds are missing; regenerate them with ktx2_make_seeds');

let mutants = 0;
let agreed = 0;
let compared = 0;
let onlyOurs = 0;
let onlyTheirs = 0;
let poisoned = 0;
let empty = 0;

for (const name of seeds) {
  const original = new Uint8Array(await readFile(resolve(SEEDS, name)));
  // One stream per seed, so adding a seed does not renumber another's mutants.
  const next = randomBits(0x2545f491 ^ (name.length * 2654435761));

  // What the pristine seed transcodes to, from a fresh instance. It is read
  // back after every mutant: see `poisoned` below.
  reference = await loadReference();
  let canary = reference!.transcodeBytes(original, 0, TARGET.RGBA32);
  const keyValues = keyValueRange(original);

  for (let round = 0; round < ROUNDS; round++) {
    const bytes = mutate(original, next, keyValues);
    mutants++;

    const mine = ours(ktx2, bytes);
    let anyOfMine = false;
    let anyOfTheirs = false;
    const pending: [string, Uint8Array, Uint8Array][] = [];

    for (const target of TARGETS) {
      const want = theirs(reference!, bytes, target.reference);
      const got = mine?.get(target.name) ?? null;
      anyOfMine ||= got !== null;
      anyOfTheirs ||= want !== null;
      // An all-zero image from the reference is not an answer. On a malformed
      // file its `transcodeImage` can return success having written nothing,
      // and comparing against an empty buffer would say this reader invented
      // every pixel. No seed transcodes to zeros - the canary below is the
      // proof - so this discards the reference's non-answers and nothing else.
      if (want !== null && got !== null) {
        if (want.every((byte) => byte === 0)) empty++;
        else pending.push([target.name, want, got]);
      }
    }

    // Whether the oracle is still an oracle. Feeding the reference a malformed
    // file can leave its module reporting success while writing nothing, and
    // an all-zero image would then look like this reader inventing data. So
    // the pristine seed is transcoded again, and if it no longer produces what
    // it did from a fresh instance, this mutant's comparisons are discarded
    // rather than believed, and the instance is replaced.
    const again = theirs(reference!, original, TARGET.RGBA32);
    if (again === null || firstDifference(canary, again) !== null) {
      poisoned++;
      reference = await loadReference();
      canary = reference!.transcodeBytes(original, 0, TARGET.RGBA32);
      continue;
    }

    for (const [target, want, got] of pending) {
      // The sound half: both read the same file, so both must produce the
      // same bytes. This is the whole point - a difference here is a defect
      // in one of the two, and the other one is Binomial's.
      const difference = firstDifference(want, got);
      assert.equal(difference, null, `${name} mutant ${round} into ${target}: ${difference}`);
      compared++;
    }

    if (anyOfMine && anyOfTheirs) agreed++;
    else if (anyOfMine) onlyOurs++;
    else if (anyOfTheirs) onlyTheirs++;
  }
}

// Divergence about acceptance is expected and bounded rather than absent. What
// would be alarming is either side becoming wholesale more permissive, which
// is what these ceilings catch: they are set well above what is measured and
// below what a real regression would produce.
const rate = (count: number) => count / mutants;
assert.ok(
  rate(onlyTheirs) < 0.2,
  `the reference accepted ${onlyTheirs} of ${mutants} mutants this reader refused, which is more than a difference of limits`,
);
assert.ok(
  rate(onlyOurs) < 0.2,
  `this reader accepted ${onlyOurs} of ${mutants} mutants the reference refused, which is more than a difference of limits`,
);
assert.ok(
  compared > mutants / 4,
  `only ${compared} images were comparable out of ${mutants} mutants; the mutations are destroying the file rather than perturbing it`,
);

console.log(
  `ktx2-differential: ${mutants} mutants, ${compared} images byte-identical to the reference; `
  + `both read ${agreed}, only this reader ${onlyOurs}, only the reference ${onlyTheirs}, `
  + `${poisoned} discarded for a degraded oracle, ${empty} for an empty one`,
);
