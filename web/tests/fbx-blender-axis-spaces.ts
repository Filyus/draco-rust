/**
 * Both FBX axis conventions, differentially against Blender.
 *
 * `fbx-export-space.ts` covers the same ground with our own reader on both ends,
 * which proves the writer and the reader agree and nothing about whether either
 * agrees with the world. Every Z-up file in that test is one this code wrote. So
 * the Z-up branch -- the whole reason the space became a choice -- rested on a
 * circle, and a shared misreading of `UpAxis` would have passed it.
 *
 * Blender breaks the circle in both directions:
 *
 *   in   -- it exports one mesh three ways, and our importer has to place all
 *           three where Blender put the vertices. Blender does not turn the
 *           coordinates for Y-up; it leaves them and turns the object, so the
 *           files differ in more than a declaration. The third states a
 *           `UnitScaleFactor` of 100 where the others state 1 -- the first
 *           fixture here with two units to tell apart.
 *   out  -- we write one document in both spaces, and Blender has to read the
 *           same world back from each.
 *
 * What this pins that a round trip through our own reader cannot: that our
 * reading of what the seven fields *mean* is another tool's reading. Ignoring
 * the axes, ignoring `UnitScaleFactor`, and converting the wrong way each fail
 * here. A self-consistent but unconventional declaration would pass, and should
 * -- FBX permits it, and Blender honours whatever the file says.
 *
 * Skipped, not failed, on a machine without Blender: the assertions are about
 * our code, but the oracle is not ours to ship.
 */
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { buildSceneDocumentFromFbx } from '../src/fbx-scene-document.ts';
import { buildFbxSceneFromDocument } from '../src/fbx-scene-document-writer.ts';
import { buildSceneDocumentFromMeshes } from '../src/mesh-scene-document.ts';
import { loadWasm, readBytes } from './fbx-test-utils.ts';
import type { FbxExportSpaceName } from '../src/fbx-space.ts';

const blender = process.env.BLENDER || 'C:/Program Files/Blender Foundation/Blender 4.5/blender.exe';
if (!existsSync(blender)) {
    console.log(`SKIP FBX Blender axis spaces: missing ${blender}`);
    process.exit(0);
}

/** Distinct on every axis and asymmetric, so no swap or sign can hide. */
const VERTICES: Vector3[] = [[1, 2, 3], [-4, 5, -6], [7, -8, 9]];
const TOLERANCE = 1e-3;

type Vector3 = [number, number, number];

const fbx = await loadWasm('fbx');
const scratch = await mkdtemp(resolve(tmpdir(), 'draco-fbx-axis-'));

/** Blender is Z-up and right-handed; glTF is Y-up. */
function blenderToGltf([x, y, z]: Vector3): Vector3 {
    return [x, z, -y];
}

function gltfToBlender([x, y, z]: Vector3): Vector3 {
    return [x, -z, y];
}

function runBlender(script: string, label: string): any {
    const result = spawnSync(blender, ['--background', '--python-expr', script], {
        encoding: 'utf8',
        maxBuffer: 8 * 1024 * 1024,
    });
    assert.equal(result.status, 0, `Blender ${label}:\n${result.stderr || result.stdout}`);
    const line = result.stdout.split(/\r?\n/).find((value) => value.startsWith('DRACO_JSON='));
    assert.ok(line, `Blender ${label} produced no result:\n${result.stderr || result.stdout}`);
    return JSON.parse(line!.slice('DRACO_JSON='.length));
}

/**
 * Sorted world positions, so the comparison survives a reordering.
 *
 * Neither end of this promises to keep vertex order -- Blender's importer welds
 * and reindexes -- and order is not what is under test.
 */
function sorted(vertices: Vector3[]): Vector3[] {
    return [...vertices].sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]);
}

function close(actual: Vector3[], expected: Vector3[], label: string): void {
    assert.equal(actual.length, expected.length, `${label}: vertex count`);
    const left = sorted(actual);
    const right = sorted(expected);
    for (let vertex = 0; vertex < left.length; vertex += 1) {
        for (let component = 0; component < 3; component += 1) {
            assert.ok(
                Math.abs(left[vertex][component] - right[vertex][component]) < TOLERANCE,
                `${label}: vertex ${vertex} is ${left[vertex].join(', ')}, expected ${right[vertex].join(', ')}`,
            );
        }
    }
}

/** World positions from a SceneDocument: the node's matrix applied to POSITION. */
function documentWorldPositions(document: ReturnType<typeof buildSceneDocumentFromFbx>): Vector3[] {
    const node = document.nodes.find((entry) => entry.mesh !== undefined);
    assert.ok(node, 'the imported document must carry a node with a mesh');
    const primitive = document.meshes[node!.mesh!].primitives[0];
    const accessor = document.accessors[primitive.attributes.POSITION];
    const values = new Float32Array(
        accessor.bytes.buffer,
        accessor.bytes.byteOffset,
        accessor.bytes.byteLength / 4,
    );
    // glTF matrices are column-major, and the importer writes one per node
    // rather than a decomposition.
    const m = node!.matrix || [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
    const world: Vector3[] = [];
    for (let offset = 0; offset + 2 < values.length; offset += 3) {
        const [x, y, z] = [values[offset], values[offset + 1], values[offset + 2]];
        world.push([
            m[0] * x + m[4] * y + m[8] * z + m[12],
            m[1] * x + m[5] * y + m[9] * z + m[13],
            m[2] * x + m[6] * y + m[10] * z + m[14],
        ]);
    }
    return world;
}

// A triangle authored in Blender, exported once per convention. `axis_forward`
// pairs with `axis_up` the way Blender's own presets do.
const exportScript = `
import bpy, json
verts = ${JSON.stringify(VERTICES)}
written = {}
for name, up, forward, scaling in (('yup', 'Y', '-Z', 'FBX_SCALE_NONE'), ('zup', 'Z', 'Y', 'FBX_SCALE_NONE'),
                                   ('zup-metres', 'Z', 'Y', 'FBX_SCALE_UNITS')):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    mesh = bpy.data.meshes.new('Probe')
    mesh.from_pydata([tuple(v) for v in verts], [], [(0, 1, 2)])
    mesh.update()
    bpy.context.scene.collection.objects.link(bpy.data.objects.new('Probe', mesh))
    path = ${JSON.stringify(scratch.replace(/\\/g, '/'))} + '/blender-' + name + '.fbx'
    bpy.ops.export_scene.fbx(filepath=path, use_selection=False, object_types={'MESH'},
                             axis_up=up, axis_forward=forward, mesh_smooth_type='OFF',
                             apply_scale_options=scaling)
    written[name] = path
print('DRACO_JSON=' + json.dumps(written, separators=(',', ':')))
`;

/** World vertices of every mesh Blender finds in a file, in Blender's space. */
function importScript(paths: Record<string, string>): string {
    return `
import bpy, json
result = {}
for name, path in ${JSON.stringify(paths)}.items():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.fbx(filepath=path)
    world = []
    for obj in bpy.context.scene.objects:
        if obj.type != 'MESH':
            continue
        matrix = obj.matrix_world
        world += [list(matrix @ vertex.co) for vertex in obj.data.vertices]
    result[name] = world
print('DRACO_JSON=' + json.dumps(result, separators=(',', ':')))
`;
}

try {
    // --- In: Blender authors both conventions, we have to read both alike.
    const authored = runBlender(exportScript, 'export');
    const expectedIn = VERTICES.map(blenderToGltf);
    for (const [name, path] of Object.entries(authored) as [string, string][]) {
        const parsed = fbx.parse_fbx(await readBytes(path));
        assert.equal(parsed.success, true, `${name}: ${parsed.error || ''}`);
        const settings = parsed.scene.globalSettings;
        assert.equal(settings.upAxis, name === 'yup' ? 1 : 2, `${name}: Blender declared its up axis`);
        assert.equal(settings.unitScaleFactor, name === 'zup-metres' ? 100 : 1, `${name}: unit scale`);
        const world = documentWorldPositions(buildSceneDocumentFromFbx(parsed));
        close(world, expectedIn, `Blender ${name} -> SceneDocument world`);
        console.log(`PASS Blender ${name} FBX -> SceneDocument: up=${settings.upAxis}, unit=${settings.unitScaleFactor}`);
    }

    // --- Out: we author both conventions, Blender has to read both alike.
    const source = buildSceneDocumentFromMeshes([{
        name: 'Probe',
        positions: expectedIn.flat(),
        indices: [0, 1, 2],
    }]);
    const written: Record<string, string> = {};
    for (const space of ['meters-y-up', 'meters-z-up'] as FbxExportSpaceName[]) {
        const scene = buildFbxSceneFromDocument(source, { space });
        const output = fbx.create_fbx_scene(scene, { version: 7500 });
        assert.equal(output.success, true, `${space} writer: ${output.error || ''}`);
        const path = resolve(scratch, `draco-${space}.fbx`);
        await writeFile(path, Buffer.from(output.binary_data));
        written[space] = path.replace(/\\/g, '/');
    }
    const read = runBlender(importScript(written), 'import');
    // Back in Blender's own space, which is where it started.
    const expectedOut = expectedIn.map(gltfToBlender);
    for (const [space, world] of Object.entries(read) as [string, number[][]][]) {
        close(world.map((vertex) => vertex.slice(0, 3) as Vector3), expectedOut, `SceneDocument -> ${space} FBX -> Blender`);
        console.log(`PASS SceneDocument -> ${space} FBX -> Blender: ${world.length} vertices`);
    }
} finally {
    await rm(scratch, { recursive: true, force: true });
}
