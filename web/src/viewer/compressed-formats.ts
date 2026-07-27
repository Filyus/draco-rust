/**
 * Which compressed texture format this GPU can be handed, per source codec.
 *
 * A KTX2 texture arrives in a codec no GPU samples directly, so it has to be
 * turned into something before it is uploaded — either into pixels, which any
 * context takes and which costs eight times the video memory, or into a block
 * format the hardware samples as it is. Which block formats exist is a
 * property of the machine, not of the file, so the answer is read off the
 * context's extension list.
 *
 * The asymmetry is the whole reason this is a ranking rather than a constant.
 * Measured on a desktop with an NVIDIA GPU, Chrome offers the BC family and
 * nothing else; the published survey figures say the same across platforms,
 * and say how lopsided it is:
 *
 * |        | Windows | macOS | Android |  iOS |
 * |--------|--------:|------:|--------:|-----:|
 * | s3tc   |   99.9% | 88.1% |   28.6% | 39.8% |
 * | ETC2   |    2.1% | 88.0% |   99.9% | 100%  |
 * | ASTC   |    2.1% | 88.0% |   99.9% | 100%  |
 *
 * So a phone is precisely the machine that has no BC format and precisely the
 * one that can least afford eight times the video memory.
 */

/** The GL enum for each block format, so nothing here needs a live context. */
export const COMPRESSED_FORMAT = {
  /** `COMPRESSED_RGB_S3TC_DXT1_EXT` */
  bc1: 0x83f0,
  /** `COMPRESSED_RGBA_S3TC_DXT5_EXT` */
  bc3: 0x83f3,
  /** `COMPRESSED_RGBA_BPTC_UNORM_EXT` */
  bc7: 0x8e8c,
  /** `COMPRESSED_RGB8_ETC2` */
  etc1: 0x9274,
  /** `COMPRESSED_RGBA8_ETC2_EAC` */
  etc2: 0x9278,
  /** `COMPRESSED_RGBA_ASTC_4x4_KHR` */
  astc: 0x93b0,
} as const;

/** A source codec, as the KTX2 module names it. */
export type TextureCodec = 'etc1s' | 'uastc';

/** What to ask the transcoder for, and how to upload the result. */
export interface CompressedTarget {
  /** The transcoder's name for the target. */
  name: 'bc1' | 'bc3' | 'bc7' | 'etc1' | 'etc2' | 'astc';
  /** The GL internal format to pass to `compressedTexImage2D`. */
  format: number;
  /** Bytes each 4×4 block occupies. */
  bytesPerBlock: number;
}

/** Every target, in the order they would be preferred. */
const TARGETS: { target: CompressedTarget; extension: string; codecs: TextureCodec[]; alpha: boolean }[] = [
  {
    // First for a texture without alpha: half the video memory of BC3, and
    // nothing is given up when there is no alpha to carry.
    target: { name: 'bc1', format: COMPRESSED_FORMAT.bc1, bytesPerBlock: 8 },
    extension: 'WEBGL_compressed_texture_s3tc',
    codecs: ['etc1s'],
    alpha: false,
  },
  {
    target: { name: 'bc3', format: COMPRESSED_FORMAT.bc3, bytesPerBlock: 16 },
    extension: 'WEBGL_compressed_texture_s3tc',
    codecs: ['etc1s'],
    alpha: true,
  },
  {
    // UASTC goes to BC7 and nowhere else among the BC family. The two formats
    // were designed to correspond, so the transcode keeps what UASTC is chosen
    // for - the precision that makes it worth using over ETC1S on normal maps
    // - which BC1 or BC3 would throw away.
    target: { name: 'bc7', format: COMPRESSED_FORMAT.bc7, bytesPerBlock: 16 },
    extension: 'EXT_texture_compression_bptc',
    codecs: ['uastc'],
    alpha: true,
  },
  {
    // Ahead of ETC because ASTC is the format UASTC is a restricted profile
    // of: the block is rewritten rather than approximated, where ETC would
    // have to re-solve it. A phone with both should take this.
    target: { name: 'astc', format: COMPRESSED_FORMAT.astc, bytesPerBlock: 16 },
    extension: 'WEBGL_compressed_texture_astc',
    codecs: ['uastc'],
    alpha: true,
  },
  {
    // ETC comes after BC only because the two never appear together in
    // practice; where they do, either is a fine answer. For ETC1S this is the
    // only mobile target there is; for UASTC it is the fallback behind ASTC.
    target: { name: 'etc1', format: COMPRESSED_FORMAT.etc1, bytesPerBlock: 8 },
    extension: 'WEBGL_compressed_texture_etc',
    codecs: ['etc1s', 'uastc'],
    alpha: false,
  },
  {
    target: { name: 'etc2', format: COMPRESSED_FORMAT.etc2, bytesPerBlock: 16 },
    extension: 'WEBGL_compressed_texture_etc',
    codecs: ['etc1s', 'uastc'],
    alpha: true,
  },
  {
    // ASTC again, and last, because for ETC1S it is the opposite of what it is
    // for UASTC: four colours on a line have to be solved into two endpoints
    // and a weight, which lands slightly below BC1, where ETC1 is nearly
    // lossless and half the size. This is for a machine with ASTC and no ETC.
    target: { name: 'astc', format: COMPRESSED_FORMAT.astc, bytesPerBlock: 16 },
    extension: 'WEBGL_compressed_texture_astc',
    codecs: ['etc1s'],
    alpha: true,
  },
];

/**
 * The best target for a texture, or null to decode it to pixels instead.
 *
 * Pure so it can be checked without a browser: hand it the extension list a
 * context reports and it answers the same way it would in the viewer.
 */
export function chooseCompressedTarget(
  extensions: readonly string[],
  codec: TextureCodec,
  hasAlpha: boolean,
): CompressedTarget | null {
  const available = new Set(extensions);
  for (const candidate of TARGETS) {
    if (!candidate.codecs.includes(codec)) continue;
    if (hasAlpha && !candidate.alpha) continue;
    if (!available.has(candidate.extension)) continue;
    return candidate.target;
  }
  return null;
}

/**
 * Enable the compressed-format extensions, and report what the context has.
 *
 * The extensions have to be requested before their formats are legal, even
 * though nothing here uses the objects they return.
 */
export function enableCompressedFormats(gl: WebGL2RenderingContext): string[] {
  const supported = gl.getSupportedExtensions() || [];
  const enabled: string[] = [];
  for (const { extension } of TARGETS) {
    if (!supported.includes(extension) || enabled.includes(extension)) continue;
    if (gl.getExtension(extension)) enabled.push(extension);
  }
  return enabled;
}
