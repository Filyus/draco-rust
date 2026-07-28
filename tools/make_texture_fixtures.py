"""Write the texture fixtures: four quadrants, lossless, so a decode is checkable.

    python tools/make_texture_fixtures.py


Four flat colours rather than one, because a solid image proves only that
something arrived: quadrants prove the decoder put the right pixels in the
right places, and they survive a nearest sample at any size.
"""
import os
import subprocess
import tempfile

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

# ---------------------------------------------------------------------------
# And the same image as AVIF.
#
# Through ffmpeg rather than Pillow, which can write AVIF but only through a
# YUV conversion: every option it exposes still lands a unit or two off on flat
# colour, and these fixtures exist so a test can state its expected pixels
# instead of asking the decoder what they are. libaom over `gbrp` keeps the
# channels as they are, so the file is exact - and the assertion below is what
# says so, rather than the flag.

avif = path.replace('.webp', '.avif')
with tempfile.TemporaryDirectory() as scratch:
    source = os.path.join(scratch, 'source.png')
    image.save(source, 'PNG')
    subprocess.run(
        ['ffmpeg', '-v', 'error', '-y', '-i', source,
         '-c:v', 'libaom-av1', '-pix_fmt', 'gbrp', '-lossless', '1',
         '-f', 'avif', avif],
        check=True,
    )

with open(avif, 'rb') as handle:
    data = handle.read()
assert data[4:8] == b'ftyp' and data[8:12] in (b'avif', b'avis'), 'not an AVIF after all'
decoded = Image.open(avif).convert('RGBA')
assert decoded.tobytes() == image.tobytes(), 'AVIF did not survive its own encode'
print(f'{len(data)} bytes, ftyp/avif confirmed, pixels exact -> {avif}')

# ---------------------------------------------------------------------------
# A glTF per alternate image source, each using its codec's extension.
#
# One quad, one material, one texture, and the image beside it as a separate
# file rather than embedded - so that what a browser is asked to decode is the
# committed fixture itself and not a copy of it inside a container.

import base64
import json
import struct

positions = [
    (-1.0, -1.0, 0.0),
    (1.0, -1.0, 0.0),
    (1.0, 1.0, 0.0),
    (-1.0, 1.0, 0.0),
]
# The image's top row is red and green, so v runs down: a viewer that flips it
# would put blue and yellow on top, which the browser test would catch.
uvs = [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)]
indices = [0, 1, 2, 0, 2, 3]

buffer = bytearray()
position_offset = len(buffer)
for x, y, z in positions:
    buffer += struct.pack('<fff', x, y, z)
uv_offset = len(buffer)
for u, v in uvs:
    buffer += struct.pack('<ff', u, v)
index_offset = len(buffer)
for index in indices:
    buffer += struct.pack('<H', index)
while len(buffer) % 4:
    buffer.append(0)

def write_fixture(name, extension, uri, mime):
    document = {
        'asset': {'version': '2.0', 'generator': 'tools/make_texture_fixtures.py'},
        'extensionsUsed': [extension],
        'extensionsRequired': [extension],
        'scene': 0,
        'scenes': [{'nodes': [0]}],
        'nodes': [{'mesh': 0, 'name': 'quad'}],
        'meshes': [{
            'name': 'quad',
            'primitives': [{
                'attributes': {'POSITION': 0, 'TEXCOORD_0': 1},
                'indices': 2,
                'material': 0,
            }],
        }],
        'materials': [{
            'name': name,
            'pbrMetallicRoughness': {
                'baseColorTexture': {'index': 0},
                'metallicFactor': 0.0,
                'roughnessFactor': 1.0,
            },
        }],
        # The extension is on the texture, and there is no fallback source: a
        # reader that skips it finds a texture with nothing to sample, which is
        # why the extension is required rather than merely used.
        'textures': [{'extensions': {extension: {'source': 0}}}],
        'images': [{'uri': uri, 'mimeType': mime}],
        'buffers': [{
            'byteLength': len(buffer),
            'uri': 'data:application/octet-stream;base64,'
                   + base64.b64encode(bytes(buffer)).decode('ascii'),
        }],
        'bufferViews': [
            {'buffer': 0, 'byteOffset': position_offset, 'byteLength': 4 * 3 * 4, 'target': 34962},
            {'buffer': 0, 'byteOffset': uv_offset, 'byteLength': 4 * 2 * 4, 'target': 34962},
            {'buffer': 0, 'byteOffset': index_offset, 'byteLength': 6 * 2, 'target': 34963},
        ],
        'accessors': [
            {'bufferView': 0, 'componentType': 5126, 'count': 4, 'type': 'VEC3',
             'min': [-1.0, -1.0, 0.0], 'max': [1.0, 1.0, 0.0]},
            {'bufferView': 1, 'componentType': 5126, 'count': 4, 'type': 'VEC2'},
            {'bufferView': 2, 'componentType': 5123, 'count': 6, 'type': 'SCALAR'},
        ],
    }

    gltf = os.path.join(root, 'testdata', 'textures', f'quadrants-{name}.gltf')
    with open(gltf, 'w', encoding='utf-8') as handle:
        json.dump(document, handle, indent=2)
        handle.write('\n')
    print(f'{os.path.getsize(gltf)} bytes -> {gltf}')


write_fixture('webp', 'EXT_texture_webp', 'quadrants.webp', 'image/webp')
write_fixture('avif', 'EXT_texture_avif', 'quadrants.avif', 'image/avif')
