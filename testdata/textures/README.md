# Texture fixtures

| file | what it is |
|---|---|
| `quadrants.webp` | 64×64 lossless WebP, four flat quadrants: red, green, blue, yellow |
| `quadrants.png` | the same image as PNG |

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

`quadrants.webp` is 66 bytes. That is not a mistake: a lossless WebP of four
flat rectangles compresses to almost nothing, which is convenient — it is a real
file the browser really decodes, at a size that costs the repository nothing.
