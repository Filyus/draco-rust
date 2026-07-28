# Texture fixtures

| file | what it is |
|---|---|
| `quadrants.webp` | 64×64 lossless WebP, four flat quadrants: red, green, blue, yellow |
| `quadrants.avif` | the same image as lossless AVIF |
| `quadrants.png` | the same image as PNG |
| `quadrants-webp.gltf` | one quad using that WebP through `EXT_texture_webp` |
| `quadrants-avif.gltf` | the same quad using the AVIF through `EXT_texture_avif` |

Four quadrants rather than one flat colour, because a solid image proves only
that something arrived. Quadrants prove the decoder put the right pixels in the
right places, and they survive a nearest sample at any size, so a browser test
can name a coordinate and an expected colour without caring about filtering.

The PNG is the same image and exists so a test can state the expected pixels
without asking a WebP decoder what they are — which would be circular in a test
of the WebP path.

Regenerate:

```sh
python tools/make_texture_fixtures.py
```

Both codecs are written losslessly, so the browser test can state its expected
pixels outright instead of allowing a tolerance — where a tolerance would hide
exactly the colour conversion worth catching. WebP reaches that through Pillow;
AVIF does not, because every option Pillow exposes still routes through YUV and
lands a unit or two off on flat colour. So the AVIF is written by `ffmpeg` with
libaom over `gbrp`, which leaves the channels alone, and the script asserts the
file it just wrote decodes back to the pixels that went in. Regenerating the
fixtures therefore needs `ffmpeg` on the path; reading them needs nothing.

`quadrants-webp.gltf` references the image as a separate file rather than
embedding it, so what a browser is asked to decode is the committed fixture
itself and not a copy inside a container. Its UVs run so that the image's top
row — red and green — lands at the top: a viewer that flipped them would show
blue and yellow there, and the browser gate would say so. The extension is
declared *required*, not merely used, because there is no fallback source: a
reader that skips it finds a texture with nothing to sample.

The same holds for `quadrants-avif.gltf`, which is the same document with the
codec swapped: what differs between the two paths is the browser's decoder and
nothing else, so the fixtures differ in nothing else either.

`quadrants.webp` is 66 bytes and `quadrants.avif` is 354. That is not a mistake:
four flat rectangles compress to almost nothing either way, which is convenient
— they are real files the browser really decodes, at a size that costs the
repository nothing. AVIF is the larger of the two because its container carries
boxes a RIFF file does not, which at this scale outweighs the codec entirely.
