"""Builds the geometric-normal overflow fixtures and their C++ goldens.

Run from anywhere with the three reference CLIs given as arguments:

    python -I generate.py <draco-1.0.0 encoder> <draco-1.5.7 encoder> \
        <draco-1.5.7 decoder> <work dir>

The meshes are generated from a fixed seed, encoded by the named release, and
decoded by the 1.5.7 CLI to binary PLY; the golden is that decoder's normals,
one little-endian f32 triplet per point, sorted. The script also models the
predictor on approximately quantized positions and prints how many corners
take each overflow path -- a guide for choosing scales, not proof; README.md
says what is.
"""

import math
import os
import random
import struct
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
I64_MAX = (1 << 63) - 1
UPPER = 1 << 29


def wrap64(x):
    x &= (1 << 64) - 1
    return x - (1 << 64) if x >> 63 else x


def wrap32(x):
    x &= (1 << 32) - 1
    return x - (1 << 32) if x >> 31 else x


def trunc_div(a, b):
    q = abs(a) // abs(b)
    return q if (a >= 0) == (b >= 0) else -q


def sub(a, b):
    return [wrap64(a[i] - b[i]) for i in range(3)]


def cross(a, b):
    return [
        wrap64(a[1] * b[2] - a[2] * b[1]),
        wrap64(a[2] * b[0] - a[0] * b[2]),
        wrap64(a[0] * b[1] - a[1] * b[0]),
    ]


def abs_sum_wrapping(n):
    return wrap64(sum(wrap64(abs(c)) for c in n))


def abs_sum_saturating(n):
    return min(sum(abs(c) for c in n), I64_MAX)


def scale(n, abs_sum):
    if abs_sum > UPPER:
        q = trunc_div(abs_sum, UPPER)
        n = [trunc_div(c, q) for c in n]
    return [wrap32(c) for c in n]


def canonical(v, center):
    s = sum(abs(c) for c in v)
    if s == 0:
        return (center, 0, 0)
    a = trunc_div(v[0] * center, s)
    b = trunc_div(v[1] * center, s)
    c = center - abs(a) - abs(b)
    return (a, b, c if v[2] >= 0 else -c)


def faces_around(faces):
    around = {}
    for f in faces:
        for k in range(3):
            around.setdefault(f[k], []).append((f[(k + 1) % 3], f[(k + 2) % 3]))
    return around


def model_one_triangle(pos, faces, center):
    """Counts corners where upstream's ONE_TRIANGLE prediction leaves ours.

    `int32`: upstream's abs sum truncated to int32 picks another quotient.
    `count`: only the face-count multiple differs (upstream adds the corner's
    triangle once per face around the vertex; a single add divides otherwise).
    """
    around = faces_around(faces)
    tally = {"int32": 0, "count": 0, "same": 0}
    for v, ring in around.items():
        k = len(ring)
        for nxt, prv in ring:
            c = cross(sub(pos[nxt], pos[v]), sub(pos[prv], pos[v]))
            ours = scale(c, abs_sum_wrapping(c))
            n = [wrap64(k * x) for x in c]
            s64 = abs_sum_saturating(n)
            upstream = scale(n, wrap32(s64))
            no_trunc = scale(n, s64)
            if canonical(upstream, center) == canonical(ours, center):
                tally["same"] += 1
            elif canonical(upstream, center) != canonical(no_trunc, center):
                tally["int32"] += 1
            else:
                tally["count"] += 1
    return tally


def model_triangle_area(pos, faces, center):
    """Counts vertices whose wrapped abs sum predicts other than saturation."""
    around = faces_around(faces)
    tally = {"saturated": 0, "differs": 0}
    for v, ring in around.items():
        n = [0, 0, 0]
        for nxt, prv in ring:
            c = cross(sub(pos[nxt], pos[v]), sub(pos[prv], pos[v]))
            n = [wrap64(n[i] + c[i]) for i in range(3)]
        if sum(abs(c) for c in n) > I64_MAX:
            tally["saturated"] += 1
            if canonical(scale(n, abs_sum_wrapping(n)), center) != canonical(
                scale(n, abs_sum_saturating(n)), center
            ):
                tally["differs"] += 1
    return tally


def icosphere(rng, jitter):
    t = (1 + 5**0.5) / 2
    verts = [
        (-1, t, 0), (1, t, 0), (-1, -t, 0), (1, -t, 0),
        (0, -1, t), (0, 1, t), (0, -1, -t), (0, 1, -t),
        (t, 0, -1), (t, 0, 1), (-t, 0, -1), (-t, 0, 1),
    ]
    faces = [
        (0, 11, 5), (0, 5, 1), (0, 1, 7), (0, 7, 10), (0, 10, 11),
        (1, 5, 9), (5, 11, 4), (11, 10, 2), (10, 7, 6), (7, 1, 8),
        (3, 9, 4), (3, 4, 2), (3, 2, 6), (3, 6, 8), (3, 8, 9),
        (4, 9, 5), (2, 4, 11), (6, 2, 10), (8, 6, 7), (9, 8, 1),
    ]
    verts = [list(v) for v in verts]
    mid = {}

    def midpoint(a, b):
        key = (min(a, b), max(a, b))
        if key not in mid:
            verts.append([(verts[a][i] + verts[b][i]) / 2 for i in range(3)])
            mid[key] = len(verts) - 1
        return mid[key]

    sub_faces = []
    for a, b, c in faces:
        ab, bc, ca = midpoint(a, b), midpoint(b, c), midpoint(c, a)
        sub_faces += [(a, ab, ca), (b, bc, ab), (c, ca, bc), (ab, bc, ca)]
    out = []
    for v in verts:
        r = math.sqrt(sum(x * x for x in v))
        j = 1 + rng.uniform(-0.15, 0.15) * jitter
        out.append([x / r * j + rng.uniform(-0.05, 0.05) * jitter for x in v])
    return out, sub_faces


def write_obj(path, verts, faces, rng):
    with open(path, "w", newline="\n") as f:
        for v in verts:
            f.write("v %.6f %.6f %.6f\n" % tuple(v))
        for v in verts:
            n = [x + rng.uniform(-0.2, 0.2) for x in v]
            r = math.sqrt(sum(x * x for x in n))
            f.write("vn %.6f %.6f %.6f\n" % tuple(x / r for x in n))
        for a, b, c in faces:
            f.write("f %d//%d %d//%d %d//%d\n" % (a + 1, a + 1, b + 1, b + 1, c + 1, c + 1))
        f.flush()
        os.fsync(f.fileno())


def quantize(verts, bits):
    lo = [min(v[i] for v in verts) for i in range(3)]
    rng = max(max(v[i] for v in verts) - lo[i] for i in range(3))
    inv = ((1 << bits) - 1) / rng
    return [[math.floor((v[i] - lo[i]) * inv + 0.5) for i in range(3)] for v in verts]


def wound_fan():
    """A closed double cone whose ring winds both apexes four times.

    The summed cross products around an apex are twice the vector area its
    ring encloses, once per winding. The 1.5.7 encoder fails on integer
    positions spanning 2^31, so they stay within +-2^29, where one winding of
    a triangle on the box's corners has an abs sum near 12 h^2 = 1.5 * 2^61;
    four windings put it past i64::MAX. The triangle is skewed off the
    diagonal so the three components differ: on the diagonal the wrapped and
    the saturated sums both lead to the same canonical direction, and the
    fixture would pin nothing.
    """
    h = (1 << 29) - 4096
    corners = [(h, h, -h), (-h, h, h // 3), (h, -h, h)]
    windings = 4
    ring = []
    for k in range(windings):
        for x, y, z in corners:
            ring.append([x - 997 * k, y - 1013 * k, z - 1009 * k])
    verts = [[0, 0, 1000], [0, 0, -1000]] + ring
    n = len(ring)
    faces = []
    for i in range(n):
        a, b = 2 + i, 2 + (i + 1) % n
        faces += [(0, a, b), (1, b, a)]
    return verts, faces


def write_ply_int(path, verts, faces):
    with open(path, "w", newline="\n") as f:
        f.write("ply\nformat ascii 1.0\n")
        f.write("element vertex %d\n" % len(verts))
        f.write("property int x\nproperty int y\nproperty int z\n")
        f.write("property float nx\nproperty float ny\nproperty float nz\n")
        f.write("element face %d\nproperty list uchar int vertex_indices\nend_header\n" % len(faces))
        for i, v in enumerate(verts):
            a = 0.7 * i
            n = (math.cos(a) * 0.6, math.sin(a) * 0.6, 0.53)
            r = math.sqrt(sum(x * x for x in n))
            f.write("%d %d %d %.6f %.6f %.6f\n" % (*v, *(x / r for x in n)))
        for a, b, c in faces:
            f.write("3 %d %d %d\n" % (a, b, c))
        f.flush()
        os.fsync(f.fileno())


def ply_normals(path):
    with open(path, "rb") as f:
        data = f.read()
    end = data.index(b"end_header\n") + len(b"end_header\n")
    header = data[:end].decode("ascii").splitlines()
    assert "format binary_little_endian 1.0" in header, header
    props, count, in_vertex = [], 0, False
    sizes = {"float": 4, "float32": 4, "int": 4, "int32": 4, "uchar": 1, "uint8": 1}
    for line in header:
        p = line.split()
        if p[0] == "element":
            in_vertex = p[1] == "vertex"
            if in_vertex:
                count = int(p[2])
        elif p[0] == "property" and in_vertex:
            props.append((p[-1], sizes[p[1]]))
    stride = sum(s for _, s in props)
    offs, o = {}, 0
    for name, s in props:
        offs[name] = o
        o += s
    out = []
    for i in range(count):
        base = end + i * stride
        out.append(tuple(
            struct.unpack_from("<I", data, base + offs[k])[0] for k in ("nx", "ny", "nz")
        ))
    return sorted(out)


def run(*args):
    subprocess.run(args, check=True, stdout=subprocess.DEVNULL)


def ellipsoid(a, b, squash):
    """A regular icosphere turned off the axes, squashed along y and z.

    Its corners all have nearly the same cross product, so one scale puts many
    of them in the narrow band the face-count multiple decides; squashing two
    axes moves that scale without moving the quantization range, which the
    unsquashed x axis keeps.
    """

    class Still:
        def uniform(self, lo, hi):
            return 0.0

    verts, faces = icosphere(Still(), 0)
    out = []
    for x, y, z in verts:
        x, y = x * math.cos(a) - y * math.sin(a), x * math.sin(a) + y * math.cos(a)
        y, z = y * math.cos(b) - z * math.sin(b), y * math.sin(b) + z * math.cos(b)
        out.append([x, y * squash, z * squash])
    return out, faces


def center(normal_bits):
    return (1 << (normal_bits - 1)) - 1


def main():
    enc100, enc157, dec157, work = sys.argv[1:5]
    os.makedirs(work, exist_ok=True)
    rng = random.Random(20261009)
    jobs = []

    # The face-count multiple moves a prediction by less than a unit of a
    # 2^29-scale vector, which only a 30-bit octahedral grid resolves.
    verts, faces = ellipsoid(0.37, 0.61, 0.5375)
    obj = os.path.join(work, "ellipsoid.obj")
    write_obj(obj, verts, faces, rng)
    print("count model:", model_one_triangle(quantize(verts, 16), faces, center(30)))
    jobs.append((enc100, ["-i", obj, "-cl", "10", "-qp", "16", "-qn", "30"],
                 "ellipsoid.one_triangle_count.1.0.0.drc"))

    verts, faces = icosphere(rng, 1)
    obj = os.path.join(work, "icosphere.obj")
    write_obj(obj, verts, faces, rng)
    print("int32 model:", model_one_triangle(quantize(verts, 20), faces, center(10)))
    jobs.append((enc100, ["-i", obj, "-cl", "10", "-qp", "20", "-qn", "10"],
                 "icosphere.one_triangle_int32.1.0.0.drc"))

    fan, fan_faces = wound_fan()
    print("saturation model:", model_triangle_area(fan, fan_faces, center(10)))
    ply = os.path.join(work, "wound_fan.ply")
    write_ply_int(ply, fan, fan_faces)
    jobs.append((enc157, ["-i", ply, "-cl", "10", "-qn", "10"], "wound_fan.saturated_abs_sum.2.2.drc"))

    for encoder, args, name in jobs:
        drc = os.path.join(HERE, name)
        run(encoder, *args, "-o", drc)
        out = os.path.join(work, name + ".ply")
        run(dec157, "-i", drc, "-o", out)
        normals = ply_normals(out)
        golden = os.path.join(HERE, name[: -len(".drc")] + ".normals_golden.bin")
        with open(golden, "wb") as f:
            for n in normals:
                f.write(struct.pack("<3I", *n))
            f.flush()
            os.fsync(f.fileno())
        print(name, len(normals), "normals")


if __name__ == "__main__":
    main()
