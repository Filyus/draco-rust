/**
 * What an FBX mesh's materials have to become before the preview draws it.
 *
 * Three things were wrong at once and each hid the next. The scene carried one
 * material per mesh while the document's own list was indexed per material, and
 * the texture bindings were walked against the scene list by position, so a
 * mesh was handed whichever material shared its ordinal. A mesh assigning a
 * material per polygon was drawn with only the first of them. And a colour
 * layer, which FBX writes as floats in 0..1, was read as bytes in 0..255, which
 * multiplied every base colour by 1/255 and rendered a fully textured character
 * black.
 */
import assert from 'node:assert/strict';
import { pathToFileURL } from 'node:url';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

// The import path is browser code. These are the globals it reaches for while
// building materials, and neither participates in what is asserted below.
(globalThis as any).createImageBitmap = async () => ({ width: 4, height: 4, close() {} });
(globalThis as any).WebGL2RenderingContext = {
  REPEAT: 10497, LINEAR: 9729, LINEAR_MIPMAP_LINEAR: 9987,
};

const { buildSceneFromFbx } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'mesh-loader.ts')).href
);

/** A quad of two triangles, the second one assigned to a different material. */
function twoMaterialMesh() {
  return {
    name: 'body',
    positions: [0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0],
    normals: [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1],
    uvs: [0, 0, 1, 0, 1, 1, 0, 1],
    indices: [0, 1, 2, 0, 2, 3],
    // The document material of the first triangle the decoder saw, which is
    // the only one the scene used to be told about.
    material: 2,
    materialIndices: [2, 1],
    colors: Float32Array.from([
      1, 1, 0, 1,
      1, 1, 0, 1,
      1, 1, 0, 1,
      1, 1, 0, 1,
    ]),
  };
}

function parsedFbx(materials: unknown[], textures: unknown[]) {
  return {
    scene: {
      materials,
      textures,
      rootNodes: [{ id: 1, name: 'root', meshes: [twoMaterialMesh()], children: [] }],
    },
  };
}

const texture = { name: 'diffuse', filename: 'wood.png', content: new Uint8Array([1, 2, 3, 4]) };

// ---- the scene's materials are the document's, in the document's order -----
{
  const parsed = parsedFbx(
    [
      { name: 'first', diffuse: [1, 1, 1], textures: [{ slot: 'diffuse', textureIndex: 0 }] },
      { name: 'second', diffuse: [1, 1, 1], textures: [] },
      { name: 'third', diffuse: [1, 1, 1], textures: [{ slot: 'diffuse', textureIndex: 1 }] },
    ],
    [texture, { ...texture, filename: 'stone.png' }],
  );
  const scene = await buildSceneFromFbx(parsed, Object.create(null), {});
  assert.deepEqual(
    scene.materials.map((material: any) => material.name),
    ['first', 'second', 'third'],
    'every document material reaches the scene, in its own order',
  );
  assert.equal(scene.materials[0].baseColorTexture, 0, 'the texture the document binds to it');
  assert.equal(scene.materials[1].baseColorTexture, null, 'a material binding none keeps none');
  assert.equal(scene.materials[2].baseColorTexture, 1);

  // ---- a material per polygon becomes a primitive per material ------------
  const primitives = scene.meshes[0].primitives;
  assert.equal(primitives.length, 2, 'the two materials of one mesh are two primitives');
  assert.deepEqual(
    primitives.map((p: any) => p.materialIndex).sort(),
    [1, 2],
    'each names the document material its own triangles carry',
  );
  const triangles = primitives.reduce((sum: number, p: any) => sum + p.indices.count / 3, 0);
  assert.equal(triangles, 2, 'the split moves triangles, it does not lose or copy them');
  for (const primitive of primitives) {
    assert.ok(primitive.attributes.POSITION, 'every primitive keeps the mesh vertex data');
  }

  // ---- a colour layer does not tint a surface the material textures -------
  const textured = primitives.find((p: any) => p.materialIndex === 2);
  assert.equal(textured.attributes.COLOR_0, undefined, 'no COLOR_0 where a texture states the colour');
  const untextured = primitives.find((p: any) => p.materialIndex === 1);
  assert.ok(untextured.attributes.COLOR_0, 'kept where the material states no texture');
}

// ---- FBX colours are floats in 0..1, not bytes in 0..255 ------------------
{
  // No textures anywhere, so the colour layer is the only surface colour and
  // survives to be inspected.
  const parsed = parsedFbx(
    [{ name: 'plain', diffuse: [1, 1, 1], textures: [] },
      { name: 'plain2', diffuse: [1, 1, 1], textures: [] },
      { name: 'plain3', diffuse: [1, 1, 1], textures: [] }],
    [],
  );
  const scene = await buildSceneFromFbx(parsed, Object.create(null), {});
  const colour = scene.meshes[0].primitives[0].attributes.COLOR_0;
  assert.ok(colour, 'the colour layer reaches the primitive');
  assert.equal(colour.componentType, 5126, 'read as float, which is the domain FBX writes');
  assert.equal(colour.normalized, false, 'and therefore not renormalized by 255');
  assert.ok(colour.bytes instanceof Float32Array);
  assert.deepEqual(
    Array.from(colour.bytes.slice(0, 4)),
    [1, 1, 0, 1],
    'the authored value, not a truncation of it',
  );
}

console.log('fbx-material-split: OK');
