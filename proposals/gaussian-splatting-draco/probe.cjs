const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const repo = process.cwd();
const draco = require(path.join(repo, 'web/node_modules/draco3d'));
const BASE = 'KHR_gaussian_splatting';
const CHILD = 'TEST_gaussian_splatting_compression_draco';
const MESH = 'KHR_draco_mesh_compression';
function read(file) {
    const b = fs.readFileSync(file);
    const end = 20 + b.readUInt32LE(12);
    return [JSON.parse(b.subarray(20, end)), b.subarray(end + 8)];
}
function chunk(bytes, type, pad) {
    const out = Buffer.alloc(8 + Math.ceil(bytes.length / 4) * 4, pad);
    out.writeUInt32LE(out.length - 8);
    out.write(type, 4);
    bytes.copy(out, 8);
    return out;
}
const [doc, box] = read('testdata/Box/glTF_Binary/Box_Draco.glb');
const kd = process.argv[2] === 'kd';
const [cloud, bytes] = read(path.join(__dirname, 'target', kd ? 'cloud-kd.glb' : 'cloud.glb'));
const p = cloud.meshes[0].primitives[0];
if (kd && p.extensions[MESH]) {
    // Forge negative vectors that the experimental writer rejects.
    p.extensions[BASE].extensions = { [CHILD]: p.extensions[MESH] };
    delete p.extensions[MESH];
}
const compressed = p.extensions[BASE].extensions[CHILD];
compressed.bufferView = doc.bufferViews.length;
doc.bufferViews.push({
    buffer: 0,
    byteOffset: box.length,
    byteLength: cloud.bufferViews[0].byteLength,
});
for (const key of Object.keys(p.attributes)) p.attributes[key] += doc.accessors.length;
doc.accessors.push(...cloud.accessors);
doc.meshes.push(cloud.meshes[0]);
doc.nodes.push({ mesh: 1 }, { mesh: 1, rotation: [0, 0, 1, 0], translation: [10, 0, 0] });
doc.scenes[0].nodes.push(2, 3);
doc.extensionsRequired = [MESH, BASE, CHILD];
doc.extensionsUsed = [...doc.extensionsRequired];
const bin = Buffer.concat([box, bytes]);
doc.buffers = [{ byteLength: bin.length }];
const json = chunk(Buffer.from(JSON.stringify(doc)), 'JSON', 32);
const data = chunk(bin, 'BIN\0', 0);
const header = Buffer.alloc(12);
header.write('glTF');
header.writeUInt32LE(2, 4);
header.writeUInt32LE(12 + json.length + data.length, 8);
const file = path.join(__dirname, kd ? 'target/mixed-kd.glb' : 'mixed.glb');
fs.writeFileSync(file, Buffer.concat([header, json, data]));
async function verify() {
    const [scene, binary] = read(file);
    const mod = await draco.createDecoderModule({});
    const source = JSON.parse(fs.readFileSync('testdata/GaussianSplats/tiny-splat.gltf'));
    const raw = fs.readFileSync('testdata/GaussianSplats/tiny-splat.bin');
    for (const mesh of scene.meshes) {
        const prim = mesh.primitives[0];
        const splat = prim.mode === 0;
        const ext = splat ? prim.extensions[BASE].extensions[CHILD] : prim.extensions[MESH];
        const view = scene.bufferViews[ext.bufferView];
        const buf = new mod.DecoderBuffer();
        buf.Init(
            binary.subarray(view.byteOffset, view.byteOffset + view.byteLength),
            view.byteLength,
        );
        const dec = new mod.Decoder();
        const geo = splat ? new mod.PointCloud() : new mod.Mesh();
        assert.equal(
            dec.GetEncodedGeometryType(buf),
            splat ? mod.POINT_CLOUD : mod.TRIANGULAR_MESH,
        );
        assert.ok(
            (splat
                ? dec.DecodeBufferToPointCloud(buf, geo)
                : dec.DecodeBufferToMesh(buf, geo)
            ).ok(),
        );
        if (!splat) assert.equal(geo.num_faces() * 3, scene.accessors[prim.indices].count);
        for (const [semantic, id] of Object.entries(ext.attributes)) {
            const a = scene.accessors[prim.attributes[semantic]];
            const attr = dec.GetAttributeByUniqueId(geo, id);
            assert.ok(attr.ptr);
            assert.equal(a.count, geo.num_points());
            if (!splat) continue;
            const components = { SCALAR: 1, VEC3: 3, VEC4: 4 }[a.type];
            assert.equal(attr.num_components(), components);
            const values = new mod.DracoFloat32Array();
            assert.ok(dec.GetAttributeFloatForAllPoints(geo, attr, values));
            const original = source.accessors[source.meshes[0].primitives[0].attributes[semantic]];
            const stride = source.bufferViews[original.bufferView].byteStride;
            for (let i = 0; i < a.count; i++) {
                for (let c = 0; c < components; c++) {
                    assert.equal(
                        values.GetValue(i * components + c),
                        raw.readFloatLE(i * stride + original.byteOffset + c * 4),
                    );
                }
            }
            mod.destroy(values);
        }
        mod.destroy(geo);
        mod.destroy(dec);
        mod.destroy(buf);
    }
    console.log(
        'Mixed GLB independently decoded. All synthetic splat attribute values match source.',
    );
}
if (!kd)
    verify().catch((error) => {
        console.error(error);
        process.exitCode = 1;
    });
