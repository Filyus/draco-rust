/**
 * Which block format a texture is transcoded into, given what the GPU offers.
 *
 * The decision is made once per texture from two facts — the machine's
 * extension list and the file's own codec and alpha — and it is pure, so it
 * can be checked here rather than only in a browser that happens to have the
 * extensions the case needs. A phone's answers cannot be observed on a desktop
 * at all, which is exactly why they are worth writing down.
 *
 * The failure this guards is a wrong choice rather than a crash: BC1 for a
 * texture with alpha would drop the alpha silently, and BC3 for one without
 * would double its video memory for nothing.
 */
import assert from 'node:assert/strict';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import type {
  chooseCompressedTarget as ChooseCompressedTarget,
  CompressedTarget,
  TextureCodec,
  TextureUsage,
} from '../src/viewer/compressed-formats.ts';

const here = dirname(fileURLToPath(import.meta.url));
const { chooseCompressedTarget } = await import(
  pathToFileURL(resolve(here, '..', 'www', 'viewer', 'compressed-formats.js')).href
) as { chooseCompressedTarget: typeof ChooseCompressedTarget };

const S3TC = ['WEBGL_compressed_texture_s3tc'];
/** RGTC, which carries BC4 and BC5 and is a separate extension from S3TC. */
const RGTC = ['EXT_texture_compression_rgtc'];
/** What a desktop GPU reports: the two BC extensions arrive together. */
const DESKTOP = [...S3TC, ...RGTC];
const BPTC = ['EXT_texture_compression_bptc'];
const ETC = ['WEBGL_compressed_texture_etc'];
const ASTC = ['WEBGL_compressed_texture_astc'];
/** What a phone reports: no BC family at all.  */
const MOBILE = [...ETC, ...ASTC];

const name = (target: CompressedTarget | null) => (target ? target.name : 'pixels');

type Case = [extensions: string[], codec: TextureCodec, hasAlpha: boolean, expected: string, why: string];

const CASES: Case[] = [
  // A desktop GPU: the BC family and nothing else. This is what was measured
  // on Chrome with an NVIDIA card, and it is the case that matters most.
  [S3TC, 'etc1s', false, 'bc1', 'without alpha, BC1 is half the memory of BC3 and loses nothing'],
  [S3TC, 'etc1s', true, 'bc3', 'with alpha, only BC3 can carry it'],
  // UASTC goes to BC7 and nowhere else: BC1 or BC3 would throw away the
  // precision it is chosen for in the first place.
  [BPTC, 'uastc', false, 'bc7', 'bptc is what UASTC needs'],
  [BPTC, 'uastc', true, 'bc7', 'BC7 carries alpha, so the answer does not change'],
  [S3TC, 'uastc', false, 'pixels', 'without bptc there is nothing for UASTC to become'],
  [BPTC, 'etc1s', false, 'pixels', 'ETC1S has no BC7 path, and bptc alone offers nothing else'],
  // A phone, which is where the whole ETC and ASTC question comes from: no BC
  // family, so before these targets existed every one of these was pixels.
  [MOBILE, 'etc1s', false, 'etc1', 'ETC1 is the cheapest thing ETC1S can be on a phone'],
  [MOBILE, 'etc1s', true, 'etc2', 'alpha needs ETC2 and its EAC block'],
  [MOBILE, 'uastc', false, 'astc', 'ASTC is the format UASTC is a profile of, so nothing is lost'],
  [MOBILE, 'uastc', true, 'astc', 'and it carries alpha too'],
  [ETC, 'etc1s', false, 'etc1', 'ETC alone is enough for an ETC1S texture'],
  [ETC, 'uastc', false, 'etc1', 'UASTC reaches ETC too, which is what a phone without ASTC takes'],
  [ETC, 'uastc', true, 'etc2', 'and ETC2 when it carries alpha'],
  [ASTC, 'etc1s', false, 'astc', 'ETC1S reaches ASTC too, for a machine with no ETC'],
  [MOBILE, 'etc1s', false, 'etc1', 'but where both are offered ETC1 wins: half the size and nearly lossless'],
  // No compressed format at all.
  [[], 'etc1s', false, 'pixels', 'no compressed format at all'],
  [[], 'uastc', true, 'pixels', 'nor for UASTC'],
];

for (const [extensions, codec, hasAlpha, expected, why] of CASES) {
  const chosen = chooseCompressedTarget(extensions, codec, hasAlpha);
  assert.equal(name(chosen), expected, `${codec}${hasAlpha ? ' with alpha' : ''} on [${extensions}]: ${why}`);
}

// Normal maps, which is the one slot where a two-channel format beats every
// three-channel one: a tangent-space normal stores X and Y and the shader
// reconstructs Z, so BC5 keeps both at eight bits where BC1 spends its three
// channels on two at five.
type NormalCase = [extensions: string[], codec: TextureCodec, expected: string, why: string];

const NORMAL_CASES: NormalCase[] = [
  [DESKTOP, 'etc1s', 'bc5', 'the desktop answer: eight bits a channel instead of five'],
  [DESKTOP, 'uastc', 'bc5', 'UASTC reaches BC5 too, and the same reasoning wins'],
  // RGTC is its own extension: a context that reports S3TC and not RGTC would
  // reject the upload, so the ranking must not reach BC5 on that machine.
  [S3TC, 'etc1s', 'bc3', 'without RGTC there is no BC5 to upload, whatever S3TC offers'],
  [MOBILE, 'etc1s', 'eac_rg11', 'the phone answer, on the other hardware family'],
  [MOBILE, 'uastc', 'eac_rg11', 'and it reaches UASTC as well'],
  [BPTC, 'etc1s', 'pixels', 'bptc offers no two-channel format, and nothing color either'],
];

for (const [extensions, codec, expected, why] of NORMAL_CASES) {
  const chosen = chooseCompressedTarget(extensions, codec, true, 'normal' as TextureUsage);
  assert.equal(name(chosen), expected, `normal ${codec} on [${extensions}]: ${why}`);
}

// A normal map without alpha has nowhere to put the normal's Y, so the
// two-channel formats cannot answer and the ranking falls back to color.
assert.equal(
  name(chooseCompressedTarget(DESKTOP, 'etc1s', false, 'normal' as TextureUsage)),
  'bc1',
  'an alpha-less normal map falls back to the color ranking',
);

// The same fallback on a machine with neither two-channel format: a normal
// map still renders, through whatever color format the machine takes.
assert.equal(
  name(chooseCompressedTarget(ASTC, 'etc1s', true, 'normal' as TextureUsage)),
  'astc',
  'an unanswered normal map falls back to color rather than to pixels',
);
assert.equal(
  name(chooseCompressedTarget(BPTC, 'etc1s', true, 'normal' as TextureUsage)),
  'pixels',
  'and where the color ranking has no answer either, pixels',
);

// And a texture used as color only never widens into a two-channel format:
// one image, one uploaded format, and BC3 for a color texture with alpha.
assert.equal(
  name(chooseCompressedTarget(DESKTOP, 'etc1s', true, 'color' as TextureUsage)),
  'bc3',
  'color usage stays in the color ranking',
);

// The block size has to match the format, because the upload is sized by it.
assert.equal(chooseCompressedTarget(S3TC, 'etc1s', false)?.bytesPerBlock, 8);
assert.equal(chooseCompressedTarget(S3TC, 'etc1s', true)?.bytesPerBlock, 16);
assert.equal(chooseCompressedTarget(BPTC, 'uastc', false)?.bytesPerBlock, 16);
assert.equal(chooseCompressedTarget(MOBILE, 'etc1s', false)?.bytesPerBlock, 8);
assert.equal(chooseCompressedTarget(MOBILE, 'uastc', false)?.bytesPerBlock, 16);

// Which textures may be answered as normal maps at all. The ranking above is
// only ever as right as this classification: one image is uploaded in one
// block format, so a texture a material also samples as color must not be
// called a normal map, whatever else references it.
const { normalOnlyTextureIndices } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'scene-document-textures.ts')).href
) as { normalOnlyTextureIndices: (scene: any) => Set<number> };

const usageScene = (materials: unknown[]) => ({ materials } as any);

assert.deepEqual(
  [...normalOnlyTextureIndices(usageScene([{ normalTexture: { index: 3 } }]))],
  [3],
  'a texture sampled only through normalTexture is a normal map',
);
assert.deepEqual(
  // `baseColorTexture` is flattened to a bare index by both producers of
  // ViewerScene.materials, which is the reading this gate exists to hold.
  [...normalOnlyTextureIndices(usageScene([{ normalTexture: { index: 3 }, baseColorTexture: 3 }]))],
  [],
  'a texture also sampled as base color is color, flattened index and all',
);
assert.deepEqual(
  [...normalOnlyTextureIndices(usageScene([
    { normalTexture: { index: 3 } },
    { baseColorTexture: 3 },
  ]))],
  [],
  'and across materials too: one image, one uploaded format',
);
assert.deepEqual(
  // Numbers that are not texture slots: a factor of 3 must not claim texture 3.
  [...normalOnlyTextureIndices(usageScene([
    { normalTexture: { index: 3 }, metallic: 3, roughness: 3, baseColorTexCoord: 3 },
  ]))],
  [3],
  'a numeric field that is not a texture slot is not a reference',
);
assert.deepEqual(
  [...normalOnlyTextureIndices(usageScene([
    { normalTexture: { index: 3 }, somethingUnknownTexture: { index: 3 } },
  ]))],
  [],
  'an unknown binding counts as color, so an unknown slot never widens',
);

console.log(`ktx2-format-choice: ${CASES.length} color and ${NORMAL_CASES.length + 3} normal cases OK, `
  + 'and 5 usage classifications');
