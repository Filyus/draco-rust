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
 * Measured on a desktop with an NVIDIA GPU: Chrome offers the BC family and
 * nothing else — no ETC, no ASTC. A phone would be the other way round. That
 * asymmetry is the whole reason this is a ranking rather than a constant.
 */

/** The GL enum for each block format, so nothing here needs a live context. */
export const COMPRESSED_FORMAT = {
  /** `COMPRESSED_RGB_S3TC_DXT1_EXT` */
  bc1: 0x83f0,
} as const;

/** A source codec, as the KTX2 module names it. */
export type TextureCodec = 'etc1s' | 'uastc';

/** What to ask the transcoder for, and how to upload the result. */
export interface CompressedTarget {
  /** The transcoder's name for the target. */
  name: 'bc1';
  /** The GL internal format to pass to `compressedTexImage2D`. */
  format: number;
  /** Bytes each 4×4 block occupies. */
  bytesPerBlock: number;
}

/** Every target, in the order they would be preferred. */
const TARGETS: { target: CompressedTarget; extension: string; codecs: TextureCodec[]; alpha: boolean }[] = [
  {
    target: { name: 'bc1', format: COMPRESSED_FORMAT.bc1, bytesPerBlock: 8 },
    extension: 'WEBGL_compressed_texture_s3tc',
    // BC1 carries no alpha, so a texture that has meaningful alpha cannot use
    // it. That case wants BC3, which is not transcoded yet, so it falls back
    // to pixels rather than silently losing its alpha.
    codecs: ['etc1s'],
    alpha: false,
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
