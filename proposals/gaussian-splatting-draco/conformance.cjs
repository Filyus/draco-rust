const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { execFileSync, spawnSync } = require('node:child_process');
const draco = require('../../web/node_modules/draco3d');
const base = 'KHR_gaussian_splatting';
const child = 'TEST_gaussian_splatting_compression_draco';
const dir = path.join(__dirname, 'target');
const trackedVectors = path.join(__dirname, 'vectors');
const updateVectors = process.argv.includes('--update-vectors');
const verifiedVectors = [];
const count = 64;
const quarterAngle = (i) => (2 * Math.PI * i) / count;
const floatRotation = (i) =>
    [0, 0, Math.sin(quarterAngle(i)), Math.cos(quarterAngle(i))].map(Math.fround);
const fields = [
    ['POSITION', Float32Array, 5126, 'VEC3', (i) => [i, 63 - i, (i * 17) % 64]],
    [base + ':ROTATION', Int8Array, 5120, 'VEC4', (i) => [0, 0, i % 2 ? -90 : 90, 90]],
    [base + ':SCALE', Uint8Array, 5121, 'VEC3', (i) => [1, 2, 1 + i]],
    [base + ':OPACITY', Uint8Array, 5121, 'SCALAR', (i) => [i * 4]],
    [base + ':SH_DEGREE_0_COEF_0', Float32Array, 5126, 'VEC3', (i) => [i, 0, 63 - i]],
    ['_POINT_ID', Uint16Array, 5123, 'SCALAR', (i) => [i]],
];
function writeInput(opts) {
    const { duplicate = false, rotation = 'unit', sh = 'finite', opacity, scale } = opts;
    fs.mkdirSync(dir, { recursive: true });
    const attributes = {},
        accessors = [],
        bufferViews = [],
        chunks = [];
    let offset = 0;
    const layout = fields.map((field) => {
        if (field[0].endsWith(':ROTATION') && rotation === 'float') {
            return [field[0], Float32Array, 5126, 'VEC4', floatRotation];
        }
        if (field[0].endsWith(':OPACITY') && opacity !== undefined) {
            return [field[0], Float32Array, 5126, 'SCALAR', () => [opacity]];
        }
        if (field[0].endsWith(':SCALE') && scale !== undefined) {
            return [field[0], Float32Array, 5126, 'VEC3', () => [1, 1, scale]];
        }
        if (field[0].endsWith(':ROTATION') && rotation === 'zero') {
            return [field[0], Int8Array, 5120, 'VEC4', () => [0, 0, 0, 0]];
        }
        if (duplicate && field[0].endsWith(':ROTATION')) {
            const mid = Math.round(32767 * Math.SQRT1_2);
            return [field[0], Int16Array, 5122, 'VEC4', (i) => [0, 0, i % 2 ? -mid : mid, mid]];
        }
        if (field[0].endsWith(':SH_DEGREE_0_COEF_0') && sh === 'nan') {
            return [field[0], Float32Array, 5126, 'VEC3', (i) => [NaN, 0, 63 - i]];
        }
        return field;
    });
    for (const [name, Type, componentType, type, row] of layout) {
        const bytes = Buffer.from(
            new Type(
                Array.from({ length: count }, (_, i) => row(duplicate && i === 1 ? 0 : i)).flat(),
            ).buffer,
        );
        const index = accessors.length;
        attributes[name] = index;
        const accessor = { bufferView: index, componentType, type, count };
        if (componentType !== 5126 && (name.endsWith(':ROTATION') || name.endsWith(':OPACITY')))
            accessor.normalized = true;
        if (name === 'POSITION') Object.assign(accessor, { min: [0, 0, 0], max: [63, 63, 63] });
        accessors.push(accessor);
        bufferViews.push({ buffer: 0, byteOffset: offset, byteLength: bytes.length });
        chunks.push(bytes);
        offset += bytes.length;
    }
    const prim = {
        mode: 0,
        attributes,
        extensions: { [base]: { kernel: 'ellipse', colorSpace: 'srgb_rec709_display' } },
    };
    const doc = {
        asset: { version: '2.0' },
        scene: 0,
        scenes: [{ nodes: [0] }],
        nodes: [{ mesh: 0 }],
        meshes: [{ primitives: [prim] }],
        extensionsUsed: [base],
        extensionsRequired: [base],
        accessors,
        bufferViews,
        buffers: [{ byteLength: offset, uri: 'kd.bin' }],
    };
    fs.writeFileSync(path.join(dir, 'kd.bin'), Buffer.concat(chunks));
    fs.writeFileSync(path.join(dir, 'kd.gltf'), JSON.stringify(doc));
}
function readGLB(file) {
    const bytes = fs.readFileSync(file);
    const end = 20 + bytes.readUInt32LE(12);
    return [JSON.parse(bytes.subarray(20, end)), bytes.subarray(end + 8)];
}
function writeVector(name, scene, bin) {
    const json = Buffer.from(JSON.stringify(scene));
    const padded = Math.ceil(json.length / 4) * 4;
    const file = Buffer.alloc(28 + padded + bin.length);
    file.write('glTF');
    file.writeUInt32LE(2, 4);
    file.writeUInt32LE(file.length, 8);
    file.writeUInt32LE(padded, 12);
    file.write('JSON', 16);
    file.fill(32, 20, 20 + padded);
    json.copy(file, 20);
    file.writeUInt32LE(bin.length, 20 + padded);
    file.writeUInt32LE(0x004e4942, 24 + padded);
    bin.copy(file, 28 + padded);
    const output = path.join(dir, 'vectors', name + '.glb');
    fs.mkdirSync(path.dirname(output), { recursive: true });
    fs.writeFileSync(output, file);
    return output;
}
function checkReader(name, scene, bytes, { reject, mode = 'read' } = {}) {
    const file = writeVector(name, scene, bytes);
    const probe = path.join(
        dir,
        'debug',
        process.platform === 'win32' ? 'splat-contract-probe.exe' : 'splat-contract-probe',
    );
    const result = spawnSync(probe, [mode, file], { encoding: 'utf8' });
    if (result.error) throw result.error;
    if (reject) {
        assert.equal(result.status, 1, name + ': expected draco-gltf rejection');
        assert.match(result.stderr, /^Error: /, name);
        assert.match(result.stderr, reject, name);
    } else {
        assert.equal(result.status, 0, name + ': ' + result.stderr);
    }
    verifiedVectors.push(file);
    return result.stdout;
}
function decode(mod, scene, bytes, supported, method = 1) {
    for (const name of scene.extensionsRequired)
        assert.ok(supported.has(name), 'unsupported required extension');
    const prim = scene.meshes.at(-1).primitives[0];
    const ext = prim.extensions[base].extensions[child];
    assert.deepEqual(
        Object.keys(ext.attributes).sort(),
        Object.keys(prim.attributes).sort(),
        'semantic map',
    );
    const view = scene.bufferViews[ext.bufferView];
    assert.ok(view, 'bufferView');
    assert.ok(!('byteStride' in view), 'strided payload');
    const start = view.byteOffset ?? 0;
    const payload = bytes.subarray(start, start + view.byteLength);
    assert.deepEqual([...payload.subarray(5, 7)], [2, 3], 'stream version');
    assert.equal(payload[7], 0, 'point cloud type');
    assert.equal(payload[8], method, 'point cloud method');
    const buf = new mod.DecoderBuffer(),
        dec = new mod.Decoder(),
        cloud = new mod.PointCloud();
    try {
        buf.Init(payload, payload.length);
        assert.ok(dec.DecodeBufferToPointCloud(buf, cloud).ok());
        const unique = new Set(),
            values = {};
        for (const [name, id] of Object.entries(ext.attributes)) {
            assert.ok(!unique.has(id), 'duplicate ID');
            unique.add(id);
            const a = scene.accessors[prim.attributes[name]];
            assert.ok(
                a && !('bufferView' in a) && !('sparse' in a) && !('byteOffset' in a),
                'accessor source',
            );
            const attr = dec.GetAttributeByUniqueId(cloud, id);
            assert.ok(attr && attr.ptr, 'missing attribute ID');
            assert.equal(a.count, cloud.num_points(), 'count');
            if (name.startsWith(base + ':SH_'))
                assert.ok(a.componentType === 5126 && !a.normalized, 'SH representation');
            if (name.endsWith(':ROTATION'))
                assert.ok(
                    (a.componentType === 5126 && !a.normalized) ||
                        ([5120, 5122].includes(a.componentType) && a.normalized),
                    'rotation representation',
                );
            const types = {
                5120: mod.DT_INT8,
                5122: mod.DT_INT16,
                5121: mod.DT_UINT8,
                5123: mod.DT_UINT16,
                5126: mod.DT_FLOAT32,
            };
            assert.equal(attr.data_type(), types[a.componentType], 'componentType');
            const n = attr.num_components();
            assert.equal(n, { SCALAR: 1, VEC3: 3, VEC4: 4 }[a.type], 'shape');
            const extraction = {
                5120: [mod.DracoInt8Array, 'GetAttributeInt8ForAllPoints'],
                5121: [mod.DracoUInt8Array, 'GetAttributeUInt8ForAllPoints'],
                5123: [mod.DracoUInt16Array, 'GetAttributeUInt16ForAllPoints'],
                5122: [mod.DracoInt16Array, 'GetAttributeInt16ForAllPoints'],
                5126: [mod.DracoFloat32Array, 'GetAttributeFloatForAllPoints'],
            };
            const [ArrayType, method] = extraction[a.componentType];
            const out = new ArrayType();
            try {
                assert.ok(dec[method](cloud, attr, out));
                const decoded = Array.from({ length: a.count }, (_, i) =>
                    Array.from({ length: n }, (_, c) => {
                        const v = out.GetValue(i * n + c);
                        if (!a.normalized) return v;
                        if (a.componentType === 5120) return Math.max(-1, v / 127);
                        if (a.componentType === 5122) return Math.max(-1, v / 32767);
                        return v / 255;
                    }),
                );
                values[name] = decoded;
                // The draft's decoded-value rules: every base value finite,
                // every decoded rotation finite and nonzero.
                if (name.endsWith(':ROTATION')) {
                    for (const q of decoded) {
                        assert.ok(
                            q.every((v) => Number.isFinite(v)),
                            'rotation finite',
                        );
                        assert.ok(Math.hypot(...q) > 0, 'rotation nonzero');
                    }
                } else if (name.startsWith(base + ':')) {
                    for (const tuple of decoded) {
                        assert.ok(
                            tuple.every((v) => Number.isFinite(v)),
                            'attribute finite',
                        );
                    }
                }
            } finally {
                mod.destroy(out);
            }
        }
        return values;
    } finally {
        mod.destroy(cloud);
        mod.destroy(dec);
        mod.destroy(buf);
    }
}
function build(kind, opts) {
    writeInput(
        opts ??
            (kind === 'duplicate'
                ? { duplicate: true }
                : kind === 'zero'
                  ? { rotation: 'zero' }
                  : kind === 'nan'
                    ? { sh: 'nan' }
                    : {}),
    );
    const variant =
        kind === 'zero' ? 'zero' : ['nan', 'lossy', 'range', 'invalid-range'].includes(kind) ? kind : 'kd';
    execFileSync(
        'cargo',
        ['run', '--offline', '--manifest-path', path.join(__dirname, 'Cargo.toml'), '--', variant],
        { stdio: 'inherit' },
    );
    execFileSync(process.execPath, [path.join(__dirname, 'probe.cjs'), 'kd'], { stdio: 'inherit' });
    return readGLB(path.join(dir, 'mixed-kd.glb'));
}
async function run() {
    const mod = await draco.createDecoderModule();
    // Decoded values must be finite, a decoded rotation nonzero: craft files
    // that break each rule and expect rejection.
    const [zeroed, zeroBytes] = build('zero');
    checkReader('zero-rotation', zeroed, zeroBytes, { reject: /rotation must be nonzero/ });
    assert.throws(
        () => decode(mod, zeroed, zeroBytes, new Set(zeroed.extensionsRequired)),
        /rotation nonzero/,
    );
    const [broken, brokenBytes] = build('nan');
    checkReader('nan-sh', broken, brokenBytes, { reject: /must be finite/ });
    assert.throws(
        () => decode(mod, broken, brokenBytes, new Set(broken.extensionsRequired), 0),
        /finite/,
    );
    const [duplicates, duplicateBytes] = build('duplicate');
    checkReader('duplicates', duplicates, duplicateBytes);
    const repeated = decode(
        mod,
        duplicates,
        duplicateBytes,
        new Set(duplicates.extensionsRequired),
    );
    assert.deepEqual(
        repeated._POINT_ID.flat().sort((a, b) => a - b),
        Array.from({ length: count }, (_, i) => (i === 1 ? 0 : i)),
    );
    const [scene, bytes] = build('plain');
    checkReader('kd-tree', scene, bytes);
    checkReader('sequential', ...readGLB(path.join(__dirname, 'mixed.glb')));
    repeated._POINT_ID.forEach(([id], i) => {
        const mid = Math.round(32767 * Math.SQRT1_2) / 32767;
        const q = repeated[base + ':ROTATION'][i];
        assert.deepEqual(q, [0, 0, id % 2 ? -mid : mid, mid]);
        assert.ok(Math.abs(Math.hypot(...q) - 1) <= 1 / 32767);
    });
    const supported = new Set(scene.extensionsRequired);
    const values = decode(mod, scene, bytes, supported);
    const q = values[base + ':ROTATION'][0];
    const norm = Math.hypot(...q);
    assert.ok(norm !== 1 && Math.abs(norm - 1) <= 1 / 127);
    assert.ok(Math.abs(Math.hypot(...q.map((v) => v / norm)) - 1) <= Number.EPSILON);
    const ids = values._POINT_ID.flat();
    assert.deepEqual(
        [...ids].sort((a, b) => a - b),
        Array.from({ length: count }, (_, i) => i),
    );
    assert.ok(
        ids.some((id, i) => id !== i),
        'vector must exercise reordering',
    );
    for (const [name, , , , row] of fields) {
        ids.forEach((id, i) => {
            const expected = row(id).map((v) =>
                name.endsWith(':ROTATION') ? v / 127 : name.endsWith(':OPACITY') ? v / 255 : v,
            );
            assert.deepEqual(values[name][i], expected, name);
        });
    }
    const missingChild = new Set(supported);
    missingChild.delete(child);
    assert.throws(() => decode(mod, scene, bytes, missingChild), /unsupported required/);
    const undeclared = structuredClone(scene);
    undeclared.extensionsRequired = undeclared.extensionsRequired.filter((name) => name !== child);
    checkReader('missing-required-declaration', undeclared, bytes, { reject: /must be declared/ });
    const cases = [
        [
            'semantic-map',
            /semantic map/,
            /invalid semantic map/,
            (e) => delete e.attributes.POSITION,
        ],
        [
            'unknown-attribute-id',
            /missing attribute ID/,
            /decoded attribute 900 is missing/,
            (e) => (e.attributes.POSITION = 900),
        ],
        [
            'duplicate-attribute-id',
            /duplicate ID/,
            /duplicate ID/,
            (e) => (e.attributes._POINT_ID = e.attributes.POSITION),
        ],
        // The committed box stream is a 2.2 mesh stream; point the extension
        // at it and raise its version to 2.3 so the rejection is about the
        // geometry type alone.
        [
            'mesh-stream',
            /point cloud type/,
            /expected sequential or KD-tree point cloud type/,
            (e) => {
                e.bufferView = 0;
                const patched = Buffer.from(bytes);
                const off = scene.bufferViews[0].byteOffset ?? 0;
                patched[off + 5] = 2;
                patched[off + 6] = 3;
                return patched;
            },
        ],
        ['unknown-buffer-view', /bufferView/, /invalid bufferView/, (e) => (e.bufferView = 900)],
        [
            'accessor-count',
            /count/,
            /DracoAccessorCount.*POSITION.*decoded: 64.*declared: 65/,
            (_, a) => a.count++,
        ],
        [
            'accessor-component-type',
            /componentType/,
            /POSITION has invalid accessor format.*VEC3.*Some\(U8\)/,
            (_, a) => (a.componentType = 5121),
        ],
        [
            'accessor-shape',
            /shape/,
            /POSITION has invalid accessor format.*VEC4.*Some\(F32\)/,
            (_, a) => (a.type = 'VEC4'),
        ],
        [
            'accessor-buffer-view',
            /accessor source/,
            /conflicting accessor source/,
            (_, a) => (a.bufferView = 0),
        ],
        [
            'unnormalized-rotation',
            /rotation representation/,
            /invalid representation for KHR_gaussian_splatting:ROTATION/,
            (e, a, p, d) => (d.accessors[p.attributes[base + ':ROTATION']].normalized = false),
        ],
        [
            'integer-sh',
            /SH representation/,
            /invalid representation for KHR_gaussian_splatting:SH_DEGREE_0_COEF_0/,
            (e, a, p, d) =>
                (d.accessors[p.attributes[base + ':SH_DEGREE_0_COEF_0']].componentType = 5122),
        ],
        [
            'strided-payload',
            /strided payload/,
            /strided payload/,
            (e, a, p, d) => (d.bufferViews[e.bufferView].byteStride = 4),
        ],
        [
            'point-cloud-version-2-2',
            /stream version/,
            /expected Draco stream version 2\.3/,
            (e) => {
                const patched = Buffer.from(bytes);
                const off = scene.bufferViews[e.bufferView].byteOffset ?? 0;
                patched[off + 5] = 2;
                patched[off + 6] = 2;
                return patched;
            },
        ],
    ];
    for (const [name, oracleReason, readerReason, change] of cases) {
        const bad = structuredClone(scene),
            prim = bad.meshes.at(-1).primitives[0];
        // Cases that patch the payload return the patched bytes; assignment
        // arrows return their value, so only a real byte view counts.
        const patch = change(
            prim.extensions[base].extensions[child],
            bad.accessors[prim.attributes.POSITION],
            prim,
            bad,
        );
        const payload = patch instanceof Uint8Array ? patch : bytes;
        assert.throws(() => decode(mod, bad, payload, supported), oracleReason);
        checkReader(name, bad, payload, { reject: readerReason });
    }
    const conflict = structuredClone(scene);
    const ext = conflict.meshes.at(-1).primitives[0].extensions;
    ext.KHR_draco_mesh_compression = ext[base].extensions[child];
    checkReader('conflicting-source', conflict, bytes, { reject: /conflicting attribute source/ });
    const badBounds = structuredClone(scene);
    const position = badBounds.meshes.at(-1).primitives[0].attributes.POSITION;
    badBounds.accessors[position].min[0] = -1;
    checkReader('incorrect-position-bounds', badBounds, bytes, {
        reject: /POSITION bounds disagree/,
    });
    for (const [name, extra, reject] of [
        ['negative-scale', { scale: -0.25 }, /negative scale/],
        ['negative-opacity', { opacity: -0.25 }, /negative scale or opacity/],
        ['opacity-out-of-range', { opacity: 1 + 2 * 2 ** -23 }, /opacity exceeds 1/],
        ['opacity-one-ulp', { opacity: 1 + 2 ** -23 }, undefined],
    ]) {
        const [doc, bin] = build(reject ? 'invalid-range' : 'range', extra);
        checkReader(name, doc, bin, { reject, mode: reject ? 'read' : 'clamp' });
    }
    const [signedZero, signedZeroBytes] = build('range', { scale: -0, opacity: -0 });
    checkReader('signed-zero-scale-opacity', signedZero, signedZeroBytes, { mode: 'signed-zero' });
    const [lossy, lossyBytes] = build('lossy', { rotation: 'float' });
    const rustRotation = checkReader('lossy-float32-rotation', lossy, lossyBytes, {
        mode: 'rotation',
    })
        .trim()
        .split(/\s+/)
        .map((value) => Math.fround(Number(value)));
    const quantized = decode(mod, lossy, lossyBytes, supported, 0);
    assert.deepEqual(rustRotation, quantized[base + ':ROTATION'].flat());
    let worstComponent = 0;
    let worstNorm = 0;
    quantized._POINT_ID.forEach(([id], i) => {
        const target = floatRotation(id);
        const actual = quantized[base + ':ROTATION'][i];
        worstNorm = Math.max(worstNorm, Math.abs(Math.hypot(...actual) - 1));
        actual.forEach((v, j) => {
            worstComponent = Math.max(worstComponent, Math.abs(v - target[j]));
        });
    });
    // With u = 2^-24, subtracting the minimum incurs at most u on [0, 2].
    // The inverse step 127.5 is exact. Multiplication and adding 0.5 each
    // round by at most 128u below 256. In component units this totals
    // (127.5 + 2*128)u / 127.5 < 4u, plus the ideal half-step 1/255.
    // On decode, rounding 2/255 contributes less than 255*2^-31 < 2u
    // after multiplication by the code. The product adds at most u
    // (code 255 rounds to 2 exactly), and adding the minimum at most u.
    const f32UnitRoundoff = 2 ** -24;
    const componentBound = 1 / 255 + 8 * f32UnitRoundoff;
    assert.ok(worstComponent > 0 && worstComponent <= componentBound);
    assert.ok(worstNorm <= 2 * componentBound + f32UnitRoundoff);
    if (updateVectors) fs.mkdirSync(trackedVectors, { recursive: true });
    for (const file of verifiedVectors) {
        const tracked = path.join(trackedVectors, path.basename(file));
        if (updateVectors) fs.copyFileSync(file, tracked);
        else assert.deepEqual(fs.readFileSync(file), fs.readFileSync(tracked), path.basename(file));
    }
    assert.deepEqual(
        fs
            .readdirSync(trackedVectors)
            .filter((name) => name.endsWith('.glb'))
            .sort(),
        verifiedVectors.map((file) => path.basename(file)).sort(),
        'tracked GLB inventory',
    );
    console.log({ worstComponent, worstNorm });
    console.log(
        'Multiplicity, KD-tree reorder, SNORM/UNORM, quaternion normalization and',
        cases.length + 9,
        'rejection cases passed.',
    );
}
run().catch((e) => {
    console.error(e);
    process.exitCode = 1;
});
