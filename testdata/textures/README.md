# Texture fixtures

| file | what it is |
|---|---|
| `quadrants.webp` | 64×64 lossless WebP, four flat quadrants: red, green, blue, yellow |
| `quadrants.png` | the same image as PNG |
| `quadrants-webp.gltf` | one quad using that WebP through `EXT_texture_webp` |

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

`quadrants-webp.gltf` references the image as a separate file rather than
embedding it, so what a browser is asked to decode is the committed fixture
itself and not a copy inside a container. Its UVs run so that the image's top
row — red and green — lands at the top: a viewer that flipped them would show
blue and yellow there, and the browser gate would say so. The extension is
declared *required*, not merely used, because there is no fallback source: a
reader that skips it finds a texture with nothing to sample.

`quadrants.webp` is 66 bytes. That is not a mistake: a lossless WebP of four
flat rectangles compresses to almost nothing, which is convenient — it is a real
file the browser really decodes, at a size that costs the repository nothing.
