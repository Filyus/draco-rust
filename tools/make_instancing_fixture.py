"""Write InstancedQuadsQuantized.gltf: InstancedQuads with a quantized rotation.

    python tools/make_instancing_fixture.py

EXT_mesh_gpu_instancing permits ROTATION as normalized BYTE or SHORT as well
as FLOAT, and nothing in any public sample corpus uses either - the one
Khronos instancing asset is float, and so is every other instancing file
anyone ships. A reader can therefore be wrong about half the extension's
accessor table and never find out from an asset.

So this is the same four quads as InstancedQuads.gltf, differing in exactly
one thing: the rotations are normalized SHORT. The two files composing to the
same matrices is what the gate asserts, and it can only assert that because
nothing else about them differs.
"""
import base64
import io
import json
import os
import struct

QUADS = 4

# The same instance transforms InstancedQuads.gltf states, so the two files are
# comparable value by value.
TRANSLATIONS = [(-2.25, 0.0, 0.0), (-0.75, 0.0, 0.0), (0.75, 0.0, 0.0), (2.25, 0.0, 0.0)]
SCALES = [(0.4, 0.4, 1.0), (0.6, 0.6, 1.0), (0.8, 0.8, 1.0), (1.0, 1.0, 1.0)]
ROTATIONS = [
    (0.0, 0.0, 0.0, 1.0),
    (0.0, 0.0, 0.19509032, 0.98078528),
    (0.0, 0.0, 0.38268343, 0.92387953),
    (0.0, 0.0, 0.55557023, 0.83146961),
]

# One unit quad, as in the float fixture.
POSITIONS = [(-0.5, -0.5, 0.0), (0.5, -0.5, 0.0), (0.5, 0.5, 0.0), (-0.5, 0.5, 0.0)]
NORMALS = [(0.0, 0.0, 1.0)] * 4
INDICES = [0, 1, 2, 0, 2, 3]


def quantize(value):
    """A unit-interval float as a normalized SHORT, the way the spec reads it back.

    Rounding rather than truncating: the extension defines the decode as
    `max(c / 32767, -1)`, so the encode that loses least is the nearest
    representable value, not the one below it.
    """
    return max(-32768, min(32767, int(round(value * 32767))))


buffer = bytearray()
views = []


def append(payload, target=None):
    while len(buffer) % 4:
        buffer.append(0)
    offset = len(buffer)
    buffer.extend(payload)
    views.append({'buffer': 0, 'byteOffset': offset, 'byteLength': len(payload)}
                 | ({'target': target} if target else {}))
    return len(views) - 1


positions = append(b''.join(struct.pack('<fff', *v) for v in POSITIONS), 34962)
normals = append(b''.join(struct.pack('<fff', *v) for v in NORMALS), 34962)
indices = append(b''.join(struct.pack('<H', i) for i in INDICES), 34963)
translations = append(b''.join(struct.pack('<fff', *v) for v in TRANSLATIONS))
# The one difference from InstancedQuads.gltf.
rotations = append(b''.join(struct.pack('<hhhh', *(quantize(c) for c in q)) for q in ROTATIONS))
scales = append(b''.join(struct.pack('<fff', *v) for v in SCALES))

document = {
    'asset': {
        'version': '2.0',
        'generator': 'tools/make_instancing_fixture.py',
        'copyright': 'Authored for this repository; CC0 1.0 Universal.',
    },
    'extensionsUsed': ['EXT_mesh_gpu_instancing'],
    'extensionsRequired': ['EXT_mesh_gpu_instancing'],
    'scene': 0,
    'scenes': [{'nodes': [0]}],
    'nodes': [{
        'name': 'InstancedQuadsQuantized',
        'mesh': 0,
        'extensions': {'EXT_mesh_gpu_instancing': {'attributes': {
            'TRANSLATION': 3, 'ROTATION': 4, 'SCALE': 5,
        }}},
    }],
    'meshes': [{
        'name': 'quad',
        'primitives': [{'attributes': {'POSITION': 0, 'NORMAL': 1}, 'indices': 2, 'material': 0}],
    }],
    'materials': [{
        'name': 'quad',
        'pbrMetallicRoughness': {
            'baseColorFactor': [0.8, 0.8, 0.8, 1.0],
            'metallicFactor': 0.0,
            'roughnessFactor': 1.0,
        },
        'doubleSided': True,
    }],
    'buffers': [{
        'byteLength': len(buffer),
        'uri': 'data:application/octet-stream;base64,' + base64.b64encode(bytes(buffer)).decode('ascii'),
    }],
    'bufferViews': views,
    'accessors': [
        {'bufferView': positions, 'componentType': 5126, 'count': QUADS, 'type': 'VEC3',
         'min': [-0.5, -0.5, 0.0], 'max': [0.5, 0.5, 0.0]},
        {'bufferView': normals, 'componentType': 5126, 'count': QUADS, 'type': 'VEC3'},
        {'bufferView': indices, 'componentType': 5123, 'count': len(INDICES), 'type': 'SCALAR'},
        {'bufferView': translations, 'componentType': 5126, 'count': QUADS, 'type': 'VEC3'},
        {'bufferView': rotations, 'componentType': 5122, 'count': QUADS, 'type': 'VEC4',
         'normalized': True},
        {'bufferView': scales, 'componentType': 5126, 'count': QUADS, 'type': 'VEC3'},
    ],
}

root = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..')
path = os.path.join(root, 'testdata', 'InstancedQuadsQuantized.gltf')
with io.open(path, 'w', encoding='utf-8') as handle:
    json.dump(document, handle, indent=2)
    handle.write('\n')

# What a reader must get back, so the fixture states its own tolerance rather
# than leaving the gate to guess one.
worst = max(abs(component - quantize(component) / 32767)
            for quaternion in ROTATIONS for component in quaternion)
print(f'{os.path.getsize(path)} bytes -> {path}')
print(f'worst rotation component error after quantization: {worst:.2e}')
