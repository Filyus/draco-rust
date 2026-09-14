/**
 * The reference Basis transcoder, as ground truth for our own.
 *
 * K17: the reference lives in this repository — `tools/basis-cpp-oracle`
 * vendors Binomial's C++ at the revision the Rust port was taken from — and
 * reaches these gates as the `basis-oracle` binary it builds. Before this the
 * gates compared against a prebuilt WASM inside a three.js checkout, so on any
 * machine without that checkout they printed SKIPPED and the byte-exactness
 * claim rested on wherever the maintainer's machine was.
 *
 * Our transcoder is a Rust port of the same algorithm from the same source, so
 * "byte for byte" is the only useful standard: a transcoder that is merely
 * close produces a texture that looks right and is wrong, and nothing
 * downstream would ever notice.
 *
 * The binary is spawned once per `loadReference` in a request/answer session:
 * one question in, one framed answer out, until the gate exits. A fresh
 * session per load is deliberate — the differential gate replaces its
 * reference when it stops trusting it, and while this oracle is stateless per
 * request, the gates' semantics should not lean on that.
 */
import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '..', '..');
const oracle = resolve(repo, 'tools', 'basis-cpp-oracle');
const binary = resolve(oracle, 'target', 'release',
  `basis-oracle${process.platform === 'win32' ? '.exe' : ''}`);

export const FIXTURES = resolve(repo, 'testdata', 'ktx2');
export const PKG = resolve(here, '..', 'www', 'pkg');

/** The transcoder's own name for each output format. */
export const TARGET = {
  ETC1_RGB: 0,
  ETC2_RGBA: 1,
  BC1_RGB: 2,
  BC3_RGBA: 3,
  BC4_R: 4,
  BC5_RG: 5,
  BC7_RGBA: 6,
  ASTC_4x4_RGBA: 10,
  RGBA32: 13,
  ETC2_EAC_R11: 20,
  ETC2_EAC_RG11: 21,
};

/** A request the oracle refuses is a status byte, not a throw. */
const ANSWERED = 1;

export interface ReferenceTranscoder {
  transcode(name: string, level: number, target: number): Promise<Uint8Array>;
  /** The same, for bytes that were built rather than read from a file. */
  transcodeBytes(bytes: Uint8Array, level: number, target: number, name?: string): Promise<Uint8Array>;
  levels(name: string): Promise<number>;
}

/**
 * Build the oracle, and say so when it has to be compiled from scratch.
 *
 * Built every run rather than only when the binary is missing: an existing
 * binary proves a build happened once, not that it was built from the vendored
 * C++ as it stands now, and a gate comparing against a stale reference is worse
 * than one that skips. Cargo answers in well under a second when there is
 * nothing to do, and the first build — about 1.5 MB of C++ — is why CI has its
 * own step for it.
 */
let build: Promise<boolean> | null = null;
function ensureBinary(): Promise<boolean> {
  build ??= buildOracle();
  return build;
}

async function buildOracle(): Promise<boolean> {
  if (!existsSync(binary)) {
    console.log('building the reference oracle: cargo build --release (in tools/basis-cpp-oracle)');
  }
  try {
    await promisify(execFile)('cargo', ['build', '--release'], {
      cwd: oracle,
      // The build is quiet until it is not; a failure's first line is enough.
      maxBuffer: 1 << 24,
    });
  } catch (error) {
    console.log(`the oracle build failed: ${(error as Error).message.split('\n')[0]}`);
    return false;
  }
  return existsSync(binary);
}

/** One framed answer: status byte, u32 little-endian length, payload. */
interface Answer {
  answered: boolean;
  payload: Buffer;
}

/** One request session: a question in, one framed answer out, until closed. */
class Session {
  private readonly child;
  private tail: Promise<unknown> = Promise.resolve();
  /**
   * Why the oracle is gone, once it is.
   *
   * A dead oracle is the one failure that must never read as a slow one: a
   * pending question would otherwise wait on a process that will never answer,
   * and the gate would hang instead of failing. Every exit is recorded here and
   * every waiting question is failed with it, named.
   */
  private departed: string | null = null;

  constructor() {
    this.child = spawn(binary, [], { stdio: ['pipe', 'pipe', 'inherit'] });
    this.child.on('exit', (code, signal) => {
      this.departed = signal
        ? `the reference oracle was killed by ${signal}`
        : `the reference oracle exited with status ${code}`;
      this.child.emit('gone', new Error(this.departed));
    });
  }

  /**
   * Ask one question and read its answer.
   *
   * Requests are serialized on a promise chain: the protocol has no
   * multiplexing, and nothing in the gates needs one. The frame boundary is
   * declared by the answer itself, so the reader takes exactly one answer per
   * question and the stream stays aligned whatever the payload holds.
   */
  ask(line: string, bytes?: Uint8Array): Promise<Answer> {
    const run = this.tail.then(() => this.exchange(line, bytes));
    this.tail = run.catch(() => {});
    return run;
  }

  private exchange(line: string, bytes?: Uint8Array): Promise<Answer> {
    return new Promise((done, fail) => {
      const { stdout, stdin } = this.child;
      let buffer = Buffer.alloc(0);
      // A question asked after the oracle is already gone fails at once
      // rather than waiting for output that cannot come.
      if (this.departed) {
        fail(new Error(this.departed));
        return;
      }
      const cleanup = () => {
        stdout.off('data', onData);
        stdout.off('error', onFail);
        stdout.off('end', onEnd);
        this.child.off('error', onFail);
        this.child.off('gone', onFail);
      };
      const onData = (chunk: Buffer) => {
        buffer = Buffer.concat([buffer, chunk]);
        // The header is one status byte and a u32 length; the answer is
        // complete only once the payload has arrived in full.
        if (buffer.length < 5) return;
        const length = buffer.readUInt32LE(1);
        if (buffer.length < 5 + length) return;
        cleanup();
        done({ answered: buffer[0] === ANSWERED, payload: buffer.subarray(5, 5 + length) });
      };
      const onFail = (error: Error) => {
        cleanup();
        fail(error);
      };
      // Every way the answer can fail to arrive: the pipe ending, the child
      // exiting, and an error on either. Without these three the reader would
      // sit on a promise nothing will ever settle.
      const onEnd = () => onFail(new Error(
        this.departed ?? 'the reference oracle closed its output mid-answer',
      ));
      stdout.on('data', onData);
      stdout.on('error', onFail);
      stdout.on('end', onEnd);
      this.child.on('error', onFail);
      this.child.on('gone', onFail);
      // Header first, then the payload the header announced — the oracle
      // reads its request bytes before it writes anything back.
      stdin.write(line + '\n');
      if (bytes) stdin.write(bytes);
    });
  }

  close() {
    this.child.stdin.end();
    this.child.kill();
  }
}

// The last session created, so a re-load replaces rather than accumulates:
// the differential gate reloads its reference whenever it stops trusting it,
// and an orphaned session would outlive its usefulness to the gate's exit.
let current: Session | null = null;

/**
 * Load the reference transcoder, or explain why the gate cannot run.
 */
export async function loadReference(): Promise<ReferenceTranscoder | null> {
  if (!await ensureBinary()) return null;
  current?.close();
  const session = new Session();
  current = session;

  const transcodeBytes = async (
    bytes: Uint8Array,
    level: number,
    target: number,
    name = 'the given bytes',
  ): Promise<Uint8Array> => {
    const answer = await session.ask(
      `T ${level} ${target} ${bytes.length}`,
      bytes,
    );
    if (!answer.answered) {
      throw new Error(`the reference transcoder refuses ${name}`);
    }
    return new Uint8Array(answer.payload);
  };

  return {
    async transcode(name, level, target) {
      return transcodeBytes(
        new Uint8Array(await readFile(resolve(FIXTURES, `${name}.ktx2`))),
        level,
        target,
        `${name}.ktx2`,
      );
    },
    transcodeBytes,
    async levels(name) {
      const answer = await session.ask(`L ${resolve(FIXTURES, `${name}.ktx2`)}`);
      if (!answer.answered) throw new Error(`the reference transcoder refuses ${name}`);
      return answer.payload.readUInt32LE(0);
    },
  };
}

/**
 * End the current session.
 *
 * A spawned child with open pipes keeps its parent's event loop alive, so a
 * gate that finished its work would hang on the way out without this. Every
 * gate calls it as its last step; a gate that dies mid-run takes the child
 * down with it, which is the behaviour it would have had without a session.
 */
export function closeReference() {
  current?.close();
  current = null;
}

/** Load our own transcoder out of the built WASM package. */
export async function loadKtx2Module(): Promise<any> {
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
 * @returns how many levels were compared.
 */
export async function compareAllLevels(
  ktx2: any,
  reference: ReferenceTranscoder,
  files: string[],
  codec: string,
): Promise<number> {
  let compared = 0;

  for (const name of files) {
    const bytes = await readFile(resolve(FIXTURES, `${name}.ktx2`));
    const file = new ktx2.Ktx2File(new Uint8Array(bytes));

    assert.equal(file.codec, codec, `${name} should be read as ${codec}`);
    assert.equal(file.levels, await reference.levels(name), `${name} level count`);

    for (let level = 0; level < file.levels; level++) {
      const want = await reference.transcode(name, level, TARGET.RGBA32);
      const image = file.decodeRgba(level);
      const got: Uint8Array = image.bytes();

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
 * @returns null when they are identical.
 */
export function firstDifference(want: Uint8Array, got: Uint8Array): string | null {
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
