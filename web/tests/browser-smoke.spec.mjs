import { expect, test } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  animatedTranslation,
  embeddedTriangle,
  externalTriangle,
  triangleBytes,
} from './smoke-fixtures.mjs';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

async function waitForConverterReady(page) {
  await expect(page.locator('#console')).toContainText('Ready to convert 3D files!');
}

test('glTF asset API reads document, geometry, accessors, GLB, and resources', async ({ page }) => {
  await page.goto('/index.html');
  const result = await page.evaluate(async ({ animated, embedded, external, resource }) => {
    const api = await import('/pkg/gltf.js');
    await api.default();

    const data = new TextEncoder().encode(embedded);
    const asset = new api.GltfAsset(data, '2.0');
    const packed = asset.readPrimitive(0, 0);
    const accessor = asset.readAccessor(0);
    const bufferView = asset.bufferViewBytes(0);
    const previewManifest = JSON.parse(new TextDecoder().decode(asset.previewManifest()));
    const glb = asset.glb(2);
    const roundtrip = new api.GltfAsset(glb, '2.0');
    let missing = false;
    try {
      new api.GltfAsset(new TextEncoder().encode(external), '2.0');
    } catch (error) {
      missing = String(error).includes('missing.bin');
    }
    const resolved = api.GltfAsset.withResources(
      new TextEncoder().encode(external),
      { 'missing.bin': new Uint8Array(resource) },
      '2.0',
    );
    const animatedAsset = new api.GltfAsset(new TextEncoder().encode(animated), '2.0');
    const times = animatedAsset.readAccessor(0);
    const translations = animatedAsset.readAccessor(1);
    return {
      document: asset.summary(),
      geometry: { meshes: asset.meshCount(), bytes: packed.attributeBytes(0).length },
      accessor: {
        count: accessor.count(),
        type: accessor.accessorType(),
        components: accessor.components(),
        componentType: accessor.componentType(),
        normalized: accessor.normalized(),
        bytes: accessor.bytes().length,
      },
      bufferViewBytes: bufferView.length,
      previewManifest: {
        roots: previewManifest.rootIndices,
        meshes: previewManifest.meshes.length,
        materials: previewManifest.materials.length,
      },
      animation: {
        times: Array.from(new Float32Array(times.bytes().buffer)),
        translations: Array.from(new Float32Array(translations.bytes().buffer)),
        outputType: translations.accessorType(),
        outputBufferViewBytes: animatedAsset.bufferViewBytes(1).length,
      },
      roundtrip: roundtrip.summary(),
      missing,
      resolved: resolved.summary(),
    };
  }, {
    animated: animatedTranslation(),
    embedded: embeddedTriangle(),
    external: externalTriangle(),
    resource: Array.from(triangleBytes()),
  });

  expect(result.document.success).toBe(true);
  expect(result.document.meshCount).toBe(1);
  expect(result.geometry).toEqual({ meshes: 1, bytes: 36 });
  expect(result.accessor).toEqual({
    count: 3,
    type: 'VEC3',
    components: 3,
    componentType: 5126,
    normalized: false,
    bytes: 36,
  });
  expect(result.bufferViewBytes).toBe(36);
  expect(result.previewManifest).toEqual({ roots: [0], meshes: 1, materials: 0 });
  expect(result.animation).toEqual({
    times: [0, 1],
    translations: [0, 0, 0, 1, 2, 3],
    outputType: 'VEC3',
    outputBufferViewBytes: 24,
  });
  expect(result.roundtrip.success).toBe(true);
  expect(result.missing).toBe(true);
  expect(result.resolved.success).toBe(true);
});

test('converter resolves glTF companions and reports decoded geometry', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles([
    path.join(repoRoot, 'testdata', 'Fox', 'glTF', 'Fox.gltf'),
    path.join(repoRoot, 'testdata', 'Fox', 'glTF', 'Fox.bin'),
    path.join(repoRoot, 'testdata', 'Fox', 'glTF', 'Texture.png'),
  ]);

  await expect(page.locator('#file-name')).toHaveText('Fox.gltf (+2 resources)');
  await expect(page.locator('#mesh-count')).toHaveText('1');
  await expect(page.locator('#vertex-count')).toHaveText(/1(?:,|\s)?728/);
  await expect(page.locator('#triangle-count')).toHaveText('576');
  await expect(page.locator('#has-normals')).toHaveText('No');
  await expect(page.locator('#has-uvs')).toHaveText('Yes');
  await expect(page.locator('#viewer-section')).toBeVisible();
  await expect(page.locator('#viewer-animation')).toBeVisible();
  await expect(page.locator('#anim-clip')).toHaveValue('0');

  const dracoToggle = page.locator('#use-draco');
  if (await dracoToggle.isEnabled()) {
    await expect(dracoToggle).toBeChecked();
  } else {
    await expect(dracoToggle).not.toBeChecked();
  }
  const downloadPromise = page.waitForEvent('download');
  await page.getByRole('button', { name: '⬇ Convert & Download' }).click();
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toBe('export.glb');
  await expect(page.locator('#console')).toContainText(
    (await dracoToggle.isEnabled())
      ? 'Document compressed with Draco and exported as GLB'
      : 'Document packaged and exported as GLB',
  );
});

test('preview applies KHR_texture_transform', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  const fixture = path.join(repoRoot, 'testdata', 'glTF', 'TextureTransformTestWithRequired');
  await page.locator('#file-input').setInputFiles([
    path.join(fixture, 'TextureTransformTestWithRequired.gltf'),
    path.join(fixture, 'TextureTransformTest.bin'),
    path.join(fixture, 'UV.png'),
    path.join(fixture, 'Arrow.png'),
    path.join(fixture, 'Correct.png'),
    path.join(fixture, 'NotSupported.png'),
    path.join(fixture, 'Error.png'),
  ]);

  await expect(page.locator('#viewer-section')).toBeVisible();
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('Unsupported glTF extensions ignored: KHR_texture_transform');
  await expect(page.locator('#console')).not.toContainText('Model requires extensions that this viewer ignores');
  await expect(page.locator('#console')).not.toContainText('Preview failed');
});

test('converter explains a missing external glTF buffer', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles(
    path.join(repoRoot, 'testdata', 'Fox', 'glTF', 'Fox.gltf'),
  );

  await expect(page.locator('#console')).toContainText('External resource denied: Fox.bin');
  await expect(page.locator('#console')).toContainText(
    'Select the .gltf together with all referenced .bin and image files.',
  );
  await expect(page.locator('#console')).not.toContainText('undefined');
});

test('3D preview renders a GLB into the WebGL2 canvas', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles(
    path.join(repoRoot, 'testdata', 'Box', 'glTF_Binary', 'Box.glb'),
  );

  await expect(page.locator('#viewer-section')).toBeVisible();
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('Skipped primitive');
  await expect(page.locator('#console')).not.toContainText('Preview failed');

  const hasContext = await page.evaluate(() => {
    const canvas = document.getElementById('viewer-canvas');
    const gl = canvas && canvas.getContext('webgl2');
    return Boolean(gl);
  });
  expect(hasContext).toBe(true);

  await expect(page.locator('#console')).not.toContainText('undefined');
});

test('3D preview opens a transformed skinned glTF scene', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles([
    path.join(repoRoot, 'testdata', 'CesiumMan', 'glTF', 'CesiumMan.gltf'),
    path.join(repoRoot, 'testdata', 'CesiumMan', 'glTF', 'CesiumMan0.bin'),
    path.join(repoRoot, 'testdata', 'CesiumMan', 'glTF', 'CesiumMan.jpg'),
  ]);

  await expect(page.locator('#viewer-section')).toBeVisible();
  await expect(page.locator('#viewer-animation')).toBeVisible();
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('Preview failed');
});

test('converter reads a CR-delimited binary PLY header', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles(
    path.join(repoRoot, 'testdata', 'delim_test.ply'),
  );

  await expect(page.locator('#console')).toContainText('Successfully parsed delim_test.ply');
  await expect(page.locator('#console')).not.toContainText('PLY header must be valid UTF-8/ASCII');
  await expect(page.locator('#viewer-section')).toBeVisible();
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('Skipped primitive');
});

test('converter previews OBJ meshes with reverse winding', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles(
    path.join(repoRoot, 'testdata', 'test_nm_seq_100.obj'),
  );

  await expect(page.locator('#console')).toContainText('Successfully parsed test_nm_seq_100.obj');
  await expect(page.locator('#viewer-section')).toBeVisible();
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('Preview failed');
  await expect(page.locator('#console')).not.toContainText('Skipped primitive');
});

test('converter preserves OBJ material groups for preview', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles([
    path.join(repoRoot, 'testdata', 'mat_test.obj'),
    path.join(repoRoot, 'testdata', 'mat_test.mtl'),
  ]);

  await expect(page.locator('#mesh-count')).toHaveText('7');
  await expect(page.locator('#viewer-section')).toBeVisible();
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).toContainText(
    'OBJ texture black.png ignored for mat4: mesh has no texture coordinates',
  );
  await expect(page.locator('#console')).not.toContainText('Skipped primitive');
  await expect(page.locator('#console')).not.toContainText('Preview failed');
});

test('converter applies a selected OBJ map_Kd texture', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  const texture = await readFile(path.join(repoRoot, 'testdata', 'Fox', 'glTF', 'Texture.png'));
  const obj = Buffer.from([
    'mtllib textured.mtl',
    'v 0 0 0',
    'v 1 0 0',
    'v 0 1 0',
    'vt 0 0',
    'vt 1 0',
    'vt 0 1',
    'usemtl textured',
    'f 1/1 2/2 3/3',
  ].join('\n'));
  const mtl = Buffer.from([
    'newmtl textured',
    'Kd 1 1 1',
    'map_Kd black.png',
  ].join('\n'));
  await page.locator('#file-input').setInputFiles([
    { name: 'textured.obj', mimeType: 'text/plain', buffer: obj },
    { name: 'textured.mtl', mimeType: 'text/plain', buffer: mtl },
    { name: 'black.png', mimeType: 'image/png', buffer: texture },
  ]);

  await expect(page.locator('#viewer-section')).toBeVisible();
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('OBJ texture not selected: black.png');
  await expect(page.locator('#console')).not.toContainText('OBJ texture black.png ignored');
  await expect(page.locator('#console')).not.toContainText('Failed to decode OBJ texture black.png');
});
