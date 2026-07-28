"""Write the texture fixtures: four quadrants, lossless, so a decode is checkable.

    python tools/make_texture_fixtures.py


Four flat colours rather than one, because a solid image proves only that
something arrived: quadrants prove the decoder put the right pixels in the
right places, and they survive a nearest sample at any size.
"""
import os

from PIL import Image

SIZE = 64
QUADRANTS = [
    ((0, 0), (220, 40, 40, 255)),      # top left, red
    ((SIZE // 2, 0), (40, 200, 60, 255)),   # top right, green
    ((0, SIZE // 2), (50, 90, 230, 255)),   # bottom left, blue
    ((SIZE // 2, SIZE // 2), (240, 220, 40, 255)),  # bottom right, yellow
]

image = Image.new('RGBA', (SIZE, SIZE))
for (x, y), colour in QUADRANTS:
    for row in range(y, y + SIZE // 2):
        for column in range(x, x + SIZE // 2):
            image.putpixel((column, row), colour)

root = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..')
path = os.path.join(root, 'testdata', 'textures', 'quadrants.webp')
os.makedirs(os.path.dirname(path), exist_ok=True)
image.save(path, 'WEBP', lossless=True, quality=100, method=6)

with open(path, 'rb') as handle:
    data = handle.read()
assert data[:4] == b'RIFF' and data[8:12] == b'WEBP', 'not a WebP after all'
print(f'{len(data)} bytes, RIFF/WEBP confirmed -> {path}')

# And a PNG of the same image, so a test can state the expected pixels without
# depending on a WebP decoder to say what they are.
png = path.replace('.webp', '.png')
image.save(png, 'PNG')
print(f'{os.path.getsize(png)} bytes -> {png}')
