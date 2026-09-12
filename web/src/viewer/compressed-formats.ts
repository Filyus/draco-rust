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
  /** `COMPRESSED_RED_RGTC1`, WebGL2 core */
  bc4: 0x8dbb,
  /** `COMPRESSED_RG_RGTC2`, WebGL2 core */
  bc5: 0x8dbc,
  /** `COMPRESSED_RGBA_BPTC_UNORM_EXT` */
  bc7: 0x8e8c,
  /** `COMPRESSED_RGB8_ETC2` */
  etc1: 0x9274,
  /** `COMPRESSED_R11_EAC` */
  eacR11: 0x9270,
  /** `COMPRESSED_RG11_EAC` */
  eacRg11: 0x9271,
  /** `COMPRESSED_RGBA8_ETC2_EAC` */
  etc2: 0x9278,
  /** `COMPRESSED_RGBA_ASTC_4x4_KHR` */
  astc: 0x93b0,
} as const;

/** A source codec, as the KTX2 module names it. */
export type TextureCodec = 'etc1s' | 'uastc';

/**
 * What slot a texture is sampled through.
 *
 * `'normal'` is the one slot where a two-channel format beats every
 * three-channel one: a tangent-space normal stores X and Y and the shader
 * reconstructs Z, so BC5 keeps both at eight bits a channel where BC1 spends
 * its three on two. Every other slot — base color, emissive, the extension
 * textures — is `'color'`.
 */
export type TextureUsage = 'color' | 'normal';

/** What to ask the transcoder for, and how to upload the result. */
export interface CompressedTarget {
  /** The transcoder's name for the target. */
  name: 'bc1' | 'bc3' | 'bc4' | 'bc5' | 'bc7' | 'etc1' | 'etc2' | 'eac_r11' | 'eac_rg11' | 'astc';
  /** The GL internal format to pass to `compressedTexImage2D`. */
  format: number;
  /** Bytes each 4×4 block occupies. */
  bytesPerBlock: number;
}

/** Every target, in the order they would be preferred for its usage. */
interface Candidate {
  target: CompressedTarget;
  extension: string;
  codecs: TextureCodec[];
  alpha: boolean;
  usage: TextureUsage;
  /**
   * The candidate is meaningless without alpha, so a texture that has none
   * never takes it even though nothing forbids it: BC5's second channel is
   * the alpha the encoder put the normal's Y in.
   */
  requiresAlpha?: boolean;
}

const TARGETS: Candidate[] = [
  {
    // A normal map through BC1 falls apart: five bits a channel is exactly
    // where the eye is most sensitive on lighting. BC5 keeps two channels at
    // eight bits, and its green half reads the alpha the encoder put the
    // normal's Y in - hence the alpha requirement. Keyed to s3tc because the
    // machine that accelerates S3TC is the one that accelerates RGTC; the
    // formats themselves are WebGL2 core.
    target: { name: 'bc5', format: COMPRESSED_FORMAT.bc5, bytesPerBlock: 16 },
    extension: 'WEBGL_compressed_texture_s3tc',
    codecs: ['etc1s', 'uastc'],
    alpha: true,
    requiresAlpha: true,
    usage: 'normal',
  },
  {
    // The phone's BC5, and the same reasoning on the other hardware family.
    target: { name: 'eac_rg11', format: COMPRESSED_FORMAT.eacRg11, bytesPerBlock: 16 },
    extension: 'WEBGL_compressed_texture_etc',
    codecs: ['etc1s', 'uastc'],
    alpha: true,
    requiresAlpha: true,
    usage: 'normal',
  },
  {
    // First for a texture without alpha: half the video memory of BC3, and
    // nothing is given up when there is no alpha to carry.
    target: { name: 'bc1', format: COMPRESSED_FORMAT.bc1, bytesPerBlock: 8 },
    extension: 'WEBGL_compressed_texture_s3tc',
    codecs: ['etc1s'],
    alpha: false,
    usage: 'color',
  },
  {
    target: { name: 'bc3', format: COMPRESSED_FORMAT.bc3, bytesPerBlock: 16 },
    extension: 'WEBGL_compressed_texture_s3tc',
    codecs: ['etc1s'],
    alpha: true,
    usage: 'color',
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
    usage: 'color',
  },
  {
    // Ahead of ETC because ASTC is the format UASTC is a restricted profile
    // of: the block is rewritten rather than approximated, where ETC would
    // have to re-solve it. A phone with both should take this.
    target: { name: 'astc', format: COMPRESSED_FORMAT.astc, bytesPerBlock: 16 },
    extension: 'WEBGL_compressed_texture_astc',
    codecs: ['uastc'],
    alpha: true,
    usage: 'color',
  },
  {
    // ETC comes after BC only because the two never appear together in
    // practice; where they do, either is a fine answer. For ETC1S this is the
    // only mobile target there is; for UASTC it is the fallback behind ASTC.
    target: { name: 'etc1', format: COMPRESSED_FORMAT.etc1, bytesPerBlock: 8 },
    extension: 'WEBGL_compressed_texture_etc',
    codecs: ['etc1s', 'uastc'],
    alpha: false,
    usage: 'color',
  },
  {
    target: { name: 'etc2', format: COMPRESSED_FORMAT.etc2, bytesPerBlock: 16 },
    extension: 'WEBGL_compressed_texture_etc',
    codecs: ['etc1s', 'uastc'],
    alpha: true,
    usage: 'color',
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
    usage: 'color',
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
  usage: TextureUsage = 'color',
): CompressedTarget | null {
  const available = new Set(extensions);
  // The usage pass first, then color: a normal map the two-channel formats
  // cannot answer - no alpha to carry the normal's Y, or an extension list
  // with nothing that takes them - still renders, and the color formats
  // remain the honest ranking for it. It must never widen the other way: a
  // color texture has no business in a two-channel format.
  for (const pass of [usage, 'color'] as const) {
    for (const candidate of TARGETS) {
      if (candidate.usage !== pass) continue;
      if (!candidate.codecs.includes(codec)) continue;
      if (hasAlpha && !candidate.alpha) continue;
      if (candidate.requiresAlpha && !hasAlpha) continue;
      if (!available.has(candidate.extension)) continue;
      return candidate.target;
    }
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
