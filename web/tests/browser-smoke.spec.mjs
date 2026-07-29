import { expect, test } from '@playwright/test';
import { existsSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  animatedTranslation,
  basisTexturedGlb,
  embeddedTriangle,
  emissiveTransformQuad,
  externalTriangle,
  normalMappedQuad,
  solidColorPng,
  triangleBytes,
} from './smoke-fixtures.mjs';
import { decodeFirstDracoPrimitive } from './draco-interop.mjs';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const mixamoFbx = process.env.MIXAMO_FBX || 'D:/Projects/Three.ts/examples/models/fbx/mixamo.fbx';
const sambaFbx = process.env.SAMBA_FBX || 'D:/Projects/Three.ts/examples/models/fbx/Samba Dancing.fbx';

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
    const sceneDocument = JSON.parse(new TextDecoder().decode(asset.json()));
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
      sceneDocument: {
        roots: sceneDocument.scenes[sceneDocument.scene ?? 0]?.nodes ?? [],
        meshes: sceneDocument.meshes?.length ?? 0,
        materials: sceneDocument.materials?.length ?? 0,
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
  expect(result.sceneDocument).toEqual({ roots: [0], meshes: 1, materials: 0 });
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

test('glTF can build a portable SceneDocument without browser image handles', async ({ page }) => {
  await page.goto('/index.html');
  const result = await page.evaluate(async ({ animated }) => {
    const [api, sceneDocument, viewerAdapter] = await Promise.all([
      import('/pkg/gltf.js'),
      import('/gltf-scene-document.js'),
      import('/scene-document-viewer.js'),
    ]);
    await api.default();
    const document = sceneDocument.buildSceneDocumentFromGltf(
      new TextEncoder().encode(animated), {}, api,
    );
    const scene = viewerAdapter.buildViewerSceneFromDocument(document);
    return {
      nodes: document.nodes.length,
      accessors: document.accessors.length,
      clips: scene.animations.length,
      firstPath: scene.animations[0]?.channels[0]?.path,
      textureHasImage: Object.hasOwn(scene.textures[0] || {}, 'image'),
    };
  }, { animated: animatedTranslation() });

  expect(result.nodes).toBeGreaterThan(0);
  expect(result.accessors).toBeGreaterThan(0);
  expect(result.clips).toBe(1);
  expect(result.firstPath).toBe('translation');
  expect(result.textureHasImage).toBe(false);
});

test('portable SceneDocument serializes through typed glTF WASM to GLB', async ({ page }) => {
  await page.goto('/index.html');
  const result = await page.evaluate(async ({ animated }) => {
    const [api, importer, exporter] = await Promise.all([
      import('/pkg/gltf.js'),
      import('/gltf-scene-document.js'),
      import('/scene-document-gltf.js'),
    ]);
    await api.default();
    const document = importer.buildSceneDocumentFromGltf(
      new TextEncoder().encode(animated), {}, api,
    );
    const output = exporter.serializeSceneDocumentToGlb(document, api);
    const roundtrip = new api.GltfAsset(output.binary, '2.0');
    const summary = roundtrip.summary();
    roundtrip.free();
    return {
      magic: Array.from(output.binary.slice(0, 4)),
      summary,
      warnings: output.warnings.length,
      capabilities: output.capabilities,
    };
  }, { animated: animatedTranslation() });

  expect(result.magic).toEqual([0x67, 0x6c, 0x54, 0x46]);
  expect(result.summary.success).toBe(true);
  expect(result.summary.sceneCount).toBe(1);
  expect(result.capabilities.gltf20).toBe(true);
  expect(result.capabilities.glb).toBe(true);
  expect(result.warnings).toBeGreaterThanOrEqual(0);
});

test('FBX SceneDocument exports to GLB and reloads without flattening', async ({ page }) => {
  test.skip(!existsSync(mixamoFbx) || !existsSync(sambaFbx), 'local Mixamo/Samba FBX fixtures are unavailable');
  await page.goto('/index.html');
  await waitForConverterReady(page);

  for (const fixture of [mixamoFbx, sambaFbx]) {
    await page.locator('#file-input').setInputFiles(fixture);
    await expect(page.locator('#console')).toContainText('Preview ready');
    await expect(page.locator('#scene-summary-compact')).toHaveCount(0);
    await expect(page.locator('#scene-panel')).toBeVisible();
    expect(await page.locator('#scene-panel').evaluate((element) => Boolean(element.closest('#scene-section')))).toBe(true);
    expect(await page.locator('#scene-panel').evaluate((element) => element.tagName)).toBe('SECTION');
    await expect(page.locator('#scene-tree .scene-tree-row')).not.toHaveCount(0);
    await expect(page.locator('#scene-tree .scene-tree-node')).not.toHaveCount(0);
    await expect(page.locator('#scene-node-stat')).not.toHaveText('0');
    await expect(page.locator('#scene-capability-summary')).toContainText('shared scene model');
    await expect(page.locator('#viewer-animation')).toBeVisible();
    await page.locator('[data-choice-for="export-format"] [data-value="glb"]').click();
    const downloadPromise = page.waitForEvent('download');
    await page.locator('#export-btn').click();
    const download = await downloadPromise;
    await expect(page.locator('#console')).toContainText('SceneDocument capabilities:');
    const downloadedPath = await download.path();
    expect(downloadedPath).not.toBeNull();
    const bytes = await readFile(downloadedPath);
    expect(bytes.subarray(0, 4).toString('binary')).toBe('glTF');
    expect(bytes.length).toBeGreaterThan(20);

    await page.locator('#file-input').setInputFiles({
      name: 'fbx-scene-roundtrip.glb',
      mimeType: 'model/gltf-binary',
      buffer: bytes,
    });
    await expect(page.locator('#console')).toContainText('Successfully parsed fbx-scene-roundtrip.glb');
    await expect(page.locator('#console')).toContainText('Preview ready');
    await expect(page.locator('#console')).not.toContainText('Preview failed');
    await page.locator('#clear-file').click();
    await expect(page.locator('#drop-zone')).toBeVisible();
  }
});

test('FBX SceneDocument exports through the typed FBX writer', async ({ page }) => {
  test.skip(!existsSync(mixamoFbx), 'local Mixamo FBX fixture is unavailable');
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles(mixamoFbx);
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#scene-summary-compact')).toHaveCount(0);
  await expect(page.locator('#scene-panel')).toBeVisible();
  expect(await page.locator('#scene-panel').evaluate((element) => Boolean(element.closest('#scene-section')))).toBe(true);
  expect(await page.locator('#export-section').evaluate((element) => Boolean(element.closest('#export-sidebar')))).toBe(true);
  // Warnings have a single home, directly under the Export panel.
  await expect(page.locator('#warnings-container')).toHaveCount(0);
  expect(await page.locator('#scene-warnings-section').evaluate((element) => Boolean(element.closest('#export-sidebar')))).toBe(true);
  await page.locator('[data-choice-for="export-format"] [data-value="fbx"]').click();
  const downloadPromise = page.waitForEvent('download');
  await page.locator('#export-btn').click();
  const download = await downloadPromise;
  const downloadedPath = await download.path();
  expect(downloadedPath).not.toBeNull();
  const bytes = await readFile(downloadedPath);
  expect(bytes.subarray(0, 21).toString('binary')).toBe('Kaydara FBX Binary  \u0000');
  await page.locator('#file-input').setInputFiles({
    name: 'scene-document-roundtrip.fbx',
    mimeType: 'application/octet-stream',
    buffer: bytes,
  });
  await expect(page.locator('#console')).toContainText('Successfully parsed scene-document-roundtrip.fbx');
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('Preview failed');
});

test('shared scene details expose all animation clips', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  const fox = path.join(repoRoot, 'testdata', 'Fox', 'glTF');
  await page.locator('#file-input').setInputFiles([
    path.join(fox, 'Fox.gltf'),
    path.join(fox, 'Fox.bin'),
  ]);
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#scene-panel')).toBeVisible();
  expect(await page.locator('#scene-panel').evaluate((element) => Boolean(element.closest('#scene-section')))).toBe(true);
  await expect(page.locator('#scene-clip-stat')).toHaveText('3');
  await expect(page.locator('#anim-clip option')).toHaveCount(3);
});

test('clicking the animated preview keeps focus on the viewport, not the clip picker', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  const fox = path.join(repoRoot, 'testdata', 'Fox', 'glTF');
  await page.locator('#file-input').setInputFiles([
    path.join(fox, 'Fox.gltf'),
    path.join(fox, 'Fox.bin'),
  ]);
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#viewer-animation')).toBeVisible();

  const box = await page.locator('#viewer-canvas').boundingBox();
  await page.mouse.click(box.x + box.width * 0.5, box.y + box.height * 0.5);
  expect(await page.evaluate(() => document.activeElement?.id)).toBe('viewer-canvas');

  // Opening the picker and closing it with Escape hands focus back to the
  // trigger rather than dropping it on the body.
  await page.locator('#anim-clip-trigger').click();
  await expect(page.locator('#anim-clip-menu')).toBeVisible();

  // The same control as the sidebar's variant picker, on a different surface,
  // and it has to hold the same two things there: the chosen row renders as
  // the field, and the list is the lower half of that control rather than a
  // popup that floated in over the viewport. Both are stated here as well as
  // on the variant picker, because each surface names its own colours and a
  // gate on one of them lets the other drift.
  await page.mouse.move(0, 0);
  await page.waitForTimeout(300);
  const clipPaint = await page.evaluate(() => {
    const rendered = (element) => {
      const layers = [];
      for (let node = element; node; node = node.parentElement) {
        const match = getComputedStyle(node).backgroundColor.match(/rgba?\(([^)]+)\)/);
        if (!match) continue;
        const [red, green, blue, alpha = 1] = match[1].split(',').map(Number.parseFloat);
        if (alpha === 0) continue;
        layers.push([red, green, blue, alpha]);
        if (alpha === 1) break;
      }
      let [red, green, blue] = layers.pop() ?? [0, 0, 0];
      while (layers.length > 0) {
        const [r, g, b, a] = layers.pop();
        red = r * a + red * (1 - a);
        green = g * a + green * (1 - a);
        blue = b * a + blue * (1 - a);
      }
      return [red, green, blue].map(Math.round).join(',');
    };
    const menu = document.querySelector('#anim-clip-menu');
    return {
      field: rendered(document.querySelector('#anim-clip-trigger')),
      selected: rendered(menu.querySelector('.menu-picker-option.selected')),
      shadow: getComputedStyle(menu).boxShadow,
    };
  });
  expect(clipPaint.selected).toBe(clipPaint.field);
  expect(clipPaint.shadow).toBe('none');

  await page.keyboard.press('Escape');
  await expect(page.locator('#anim-clip-menu')).toBeHidden();
  expect(await page.evaluate(() => document.activeElement?.id)).toBe('anim-clip-trigger');
});

test('Space toggles playback from the viewport but not from a focused control', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  const fox = path.join(repoRoot, 'testdata', 'Fox', 'glTF');
  await page.locator('#file-input').setInputFiles([
    path.join(fox, 'Fox.gltf'),
    path.join(fox, 'Fox.bin'),
  ]);
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#viewer-animation')).toBeVisible();

  // The play button label tracks `viewer.animation.playing`.
  const play = page.locator('#anim-play');
  await expect(play).toHaveAttribute('aria-label', 'Pause animation');

  await page.locator('#viewer-canvas').click();
  await page.keyboard.press('Space');
  await expect(play).toHaveAttribute('aria-label', 'Play animation');
  await page.keyboard.press('Space');
  await expect(play).toHaveAttribute('aria-label', 'Pause animation');

  // On the Loop checkbox, Space belongs to the checkbox.
  const loop = page.locator('#anim-loop');
  await loop.focus();
  const checked = await loop.isChecked();
  await page.keyboard.press('Space');
  expect(await loop.isChecked()).toBe(!checked);
  await expect(play).toHaveAttribute('aria-label', 'Pause animation');
});

test('the animation timeline follows playback and holds still when paused', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  const fox = path.join(repoRoot, 'testdata', 'Fox', 'glTF');
  await page.locator('#file-input').setInputFiles([
    path.join(fox, 'Fox.gltf'),
    path.join(fox, 'Fox.bin'),
  ]);
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#viewer-animation')).toBeVisible();

  const timeLabel = page.locator('#anim-time');
  const scrub = page.locator('#anim-scrub');
  const readTime = () => timeLabel.textContent();

  // Playing: the label has to move on its own.
  const started = await readTime();
  await expect.poll(readTime).not.toBe(started);

  // Paused: it has to stop, including the scrub position.
  await page.locator('#viewer-canvas').click();
  await page.keyboard.press('Space');
  await expect(page.locator('#anim-play')).toHaveAttribute('aria-label', 'Play animation');
  const paused = await readTime();
  const pausedScrub = await scrub.inputValue();
  await page.waitForTimeout(300);
  expect(await readTime()).toBe(paused);
  expect(await scrub.inputValue()).toBe(pausedScrub);
});

test('scene details stays hidden before load and exposes a hierarchy tree after load', async ({ page }) => {
  await page.goto('/index.html');
  await expect(page.locator('#scene-section')).toBeHidden();
  await expect(page.locator('#scene-warnings-section')).toBeHidden();
  await waitForConverterReady(page);
  const fox = path.join(repoRoot, 'testdata', 'Fox', 'glTF');
  await page.locator('#file-input').setInputFiles([
    path.join(fox, 'Fox.gltf'),
    path.join(fox, 'Fox.bin'),
  ]);
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#scene-section')).toBeVisible();
  await expect(page.locator('#scene-tree')).toBeVisible();
  await expect(page.locator('#scene-tree .scene-tree-row')).not.toHaveCount(0);
  // The root scene block never collapses, so it is a plain section without an open state.
  await expect(page.locator('#scene-panel')).toBeVisible();
  expect(await page.locator('#scene-panel').evaluate((element) => element.tagName)).toBe('SECTION');
  await expect(page.locator('#scene-tree .scene-tree-node[open]')).not.toHaveCount(0);
  await expect(page.locator('#scene-tree .scene-tree-badge-animation')).not.toHaveCount(0);
  await expect(page.locator('#scene-tree .scene-tree-children')).not.toHaveCount(0);
  await expect(page.locator('#scene-tree .scene-tree-twisty')).not.toHaveCount(0);
  // Branch rows stay obviously expandable: collapse-all then expand-all round-trips.
  await page.locator('#scene-tree-collapse').click();
  await expect(page.locator('#scene-tree .scene-tree-node[open]')).toHaveCount(0);
  await page.locator('#scene-tree-expand').click();
  await expect(page.locator('#scene-tree .scene-tree-node:not([open])')).toHaveCount(0);
  // Warnings live in their own collapsible panel that only appears when there is something to show.
  const warningsVisible = await page.locator('#scene-warnings-section').isVisible();
  if (warningsVisible) {
    expect(await page.locator('#scene-warnings').evaluate((element) => element.open)).toBe(false);
    expect(Number(await page.locator('#scene-warning-count').textContent())).toBeGreaterThan(0);
  }
  const layout = await page.locator('.sidebar').first().evaluate((element) => ({
    overflowY: getComputedStyle(element).overflowY,
    maxHeight: getComputedStyle(element).maxHeight,
  }));
  expect(layout.overflowY).toBe('auto');
  expect(layout.maxHeight).not.toBe('none');
});

test('glTF CUBICSPLINE scales tangents by keyframe duration', async ({ page }) => {
  await page.goto('/index.html');
  const values = await page.evaluate(async () => {
    const { cubicSplineInterpolate } = await import('/viewer.js');
    return {
      twoSeconds: cubicSplineInterpolate(0, 1, 0, 0, 0.5, 2),
      halfSecond: cubicSplineInterpolate(0, 1, 0, 0, 0.5, 0.5),
    };
  });

  expect(values.twoSeconds).toBeCloseTo(0.25);
  expect(values.halfSecond).toBeCloseTo(0.0625);
});

test('preview Reset restores the default orbit camera direction', async ({ page }) => {
  await page.goto('/index.html');
  const camera = await page.evaluate(async () => {
    const { Viewer } = await import('/viewer.js');
    const viewer = Object.create(Viewer.prototype);
    viewer.camera = {
      target: new Float32Array([8, 9, 10]),
      distance: 42,
      azimuth: -1.3,
      elevation: 1.1,
    };
    viewer.scene = {};
    viewer.autoRotate = true;
    viewer._updateWorldMatrices = () => {};
    viewer._updateSceneBounds = () => {};
    viewer._disposeGrid = () => {};
    viewer._fitCameraToScene = () => {
      viewer.camera.target.set([1, 2, 3]);
      viewer.camera.distance = 7;
    };

    viewer.resetView();
    return {
      azimuth: viewer.camera.azimuth,
      elevation: viewer.camera.elevation,
      target: Array.from(viewer.camera.target),
      distance: viewer.camera.distance,
      autoRotate: viewer.autoRotate,
    };
  });

  expect(camera.azimuth).toBeCloseTo(Math.PI * 0.25);
  expect(camera.elevation).toBeCloseTo(Math.PI * 0.09);
  expect(camera.target).toEqual([1, 2, 3]);
  expect(camera.distance).toBe(7);
  expect(camera.autoRotate).toBe(true);
});

test('viewport pan slides inside the camera plane and vertical orbit follows the drag', async ({ page }) => {
  await page.goto('/index.html');
  const state = await page.evaluate(async () => {
    const { Viewer } = await import('/viewer.js');
    const viewer = Object.create(Viewer.prototype);
    viewer.camera = {
      target: new Float32Array([0, 0, 0]),
      distance: 10,
      // Looking down -X: a horizontal drag must move the target along Z.
      azimuth: Math.PI * 0.5,
      elevation: 0,
      fov: Math.PI * 0.5,
    };
    viewer.canvas = { clientHeight: 100, height: 100 };
    viewer._basisRight = new Float32Array(3);
    viewer._basisUp = new Float32Array(3);
    viewer._basisForward = new Float32Array(3);
    viewer._pivotScratch = new Float32Array(3);

    // 2 * distance * tan(fov / 2) / height = 0.2 world units per pixel.
    viewer._panBy(10, 5);
    const panned = Array.from(viewer.camera.target);

    viewer._orbitBy(0.3, 0.1);
    return {
      panned,
      azimuth: viewer.camera.azimuth,
      elevation: viewer.camera.elevation,
    };
  });

  expect(state.panned[0]).toBeCloseTo(0);
  expect(state.panned[1]).toBeCloseTo(1);
  expect(state.panned[2]).toBeCloseTo(2);
  expect(state.azimuth).toBeCloseTo(Math.PI * 0.5 - 0.3);
  expect(state.elevation).toBeCloseTo(0.1);
});

test('viewport orbit turns around the scene centre after the target has moved', async ({ page }) => {
  await page.goto('/index.html');
  const state = await page.evaluate(async () => {
    const { Viewer } = await import('/viewer.js');
    const viewer = Object.create(Viewer.prototype);
    viewer.camera = {
      // Flown 5 units forward, so the look-at point is no longer the model.
      target: new Float32Array([0, 0, -5]),
      distance: 10,
      azimuth: 0,
      elevation: 0,
      fov: Math.PI * 0.5,
    };
    viewer.scene = { aabb: { min: [-1, -1, -1], max: [1, 1, 1] } };
    viewer._basisRight = new Float32Array(3);
    viewer._basisUp = new Float32Array(3);
    viewer._basisForward = new Float32Array(3);
    viewer._pivotScratch = new Float32Array(3);

    viewer._orbitBy(Math.PI * 0.5, 0);
    const eye = Array.from(viewer._cameraPosition(new Float32Array(3)));
    return { target: Array.from(viewer.camera.target), eye };
  });

  // A quarter turn about the origin takes the eye from (0, 0, 5) to (-5, 0, 0)
  // and carries the look-at point with it, so the model stays framed the same.
  expect(state.eye[0]).toBeCloseTo(-5);
  expect(state.eye[1]).toBeCloseTo(0);
  expect(state.eye[2]).toBeCloseTo(0);
  expect(state.target[0]).toBeCloseTo(5);
  expect(state.target[2]).toBeCloseTo(0);
});

test('viewport wheel zoom keeps the point under the cursor and scales its limits to the scene', async ({ page }) => {
  await page.goto('/index.html');
  const state = await page.evaluate(async () => {
    const { Viewer } = await import('/viewer.js');
    const viewer = Object.create(Viewer.prototype);
    viewer.camera = {
      target: new Float32Array([0, 0, 0]),
      distance: 10,
      azimuth: 0,
      elevation: 0,
      fov: Math.PI * 0.5,
      minDistance: 0.05,
      maxDistance: 1000,
    };
    viewer.canvas = {
      clientHeight: 100,
      height: 100,
      getBoundingClientRect: () => ({ left: 0, top: 0, width: 200, height: 100 }),
    };
    viewer._basisRight = new Float32Array(3);
    viewer._basisUp = new Float32Array(3);

    // The cursor sits 50 px right of centre, which is 10 world units out on
    // the target plane; halving the distance must halve that offset.
    viewer._zoomBy(0.5, 150, 50);
    const zoomed = { target: Array.from(viewer.camera.target), distance: viewer.camera.distance };

    // A scene far larger than the old fixed 1000 clamp.
    viewer.scene = { aabb: { min: [-3000, -3000, -3000], max: [3000, 3000, 3000] } };
    viewer.canvas.width = 200;
    viewer._fitCameraToScene();
    const fitted = viewer.camera.distance;
    viewer._zoomBy(1.2);
    return { zoomed, fitted, zoomedOut: viewer.camera.distance };
  });

  expect(state.zoomed.distance).toBeCloseTo(5);
  expect(state.zoomed.target[0]).toBeCloseTo(5);
  expect(state.zoomed.target[1]).toBeCloseTo(0);
  expect(state.zoomedOut).toBeCloseTo(state.fitted * 1.2);
});

test('viewport movement keys fly the orbit target along the camera axes', async ({ page }) => {
  await page.goto('/index.html');
  const state = await page.evaluate(async () => {
    const { Viewer } = await import('/viewer.js');
    const viewer = Object.create(Viewer.prototype);
    viewer.camera = {
      target: new Float32Array([0, 0, 0]),
      distance: 10,
      azimuth: 0,
      elevation: Math.PI * 0.4,
      fov: Math.PI * 0.5,
    };
    viewer._basisRight = new Float32Array(3);
    viewer._basisUp = new Float32Array(3);
    viewer._basisForward = new Float32Array(3);
    viewer._pivotScratch = new Float32Array(3);
    viewer._navFast = false;
    viewer._navSlow = false;

    // 0.4 * distance * dt = 0.4 world units per step, along the pitched view
    // direction rather than the ground plane.
    viewer._navKeys = new Set(['KeyW']);
    viewer._applyKeyboardNavigation(0.1);
    const forward = Array.from(viewer.camera.target);

    viewer.camera.target.set([0, 0, 0]);
    viewer._navKeys = new Set(['KeyE']);
    viewer._applyKeyboardNavigation(0.1);
    const lifted = Array.from(viewer.camera.target);

    viewer._navKeys = new Set(['ArrowUp']);
    viewer._applyKeyboardNavigation(0.1);
    const elevation = viewer.camera.elevation;
    viewer._navKeys = new Set(['ArrowRight']);
    viewer._applyKeyboardNavigation(0.1);
    return { forward, lifted, elevation, azimuth: viewer.camera.azimuth };
  });

  const pitch = Math.PI * 0.4;
  // Camera forward at azimuth 0 is (0, -sin(pitch), -cos(pitch)); its up is
  // perpendicular to that, in the same vertical plane.
  expect(state.forward[0]).toBeCloseTo(0);
  expect(state.forward[1]).toBeCloseTo(-0.4 * Math.sin(pitch));
  expect(state.forward[2]).toBeCloseTo(-0.4 * Math.cos(pitch));
  expect(state.lifted[1]).toBeCloseTo(0.4 * Math.cos(pitch));
  expect(state.lifted[2]).toBeCloseTo(-0.4 * Math.sin(pitch));
  // Arrow Up raises the camera, matching the sign the mouse now uses.
  expect(state.elevation).toBeCloseTo(pitch + 0.12);
  expect(state.azimuth).toBeCloseTo(-0.12);
});

test('preview seek applies the selected animation frame immediately', async ({ page }) => {
  await page.goto('/index.html');
  const state = await page.evaluate(async () => {
    const { Viewer } = await import('/viewer.js');
    const viewer = Object.create(Viewer.prototype);
    const node = {
      localMatrix: new Float32Array(16),
      trs: {
        translation: new Float32Array([0, 0, 0]),
        rotation: new Float32Array([0, 0, 0, 1]),
        scale: new Float32Array([1, 1, 1]),
      },
    };
    viewer.scene = {
      animations: [{
        duration: 2,
        channels: [{
          node,
          path: 'translation',
          targetCount: 3,
          sampler: {
            input: new Float32Array([0, 2]),
            output: new Float32Array([0, 0, 0, 4, 2, -2]),
            interpolation: 'LINEAR',
          },
        }],
      }],
    };
    viewer.animation = { clipIndex: 0, time: 0 };

    const applied = viewer.seekAnimation(1);
    return {
      applied,
      time: viewer.animation.time,
      translation: Array.from(node.trs.translation),
      localMatrix: node.localMatrix,
    };
  });

  expect(state.applied).toBe(true);
  expect(state.time).toBe(1);
  expect(state.translation).toEqual([2, 1, -1]);
  expect(state.localMatrix).toBeNull();
});

test('manual camera interaction synchronizes the Auto-rotate button', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles(
    path.join(repoRoot, 'testdata', 'Box', 'glTF_Binary', 'Box.glb'),
  );
  await expect(page.locator('#console')).toContainText('Preview ready');

  const autoRotate = page.locator('#viewer-autorotate');
  await autoRotate.click();
  await expect(autoRotate).toHaveAttribute('aria-pressed', 'true');
  await expect(autoRotate).toHaveClass(/active/);

  const canvas = page.locator('#viewer-canvas');
  const box = await canvas.boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.5);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.55, box.y + box.height * 0.5);
  await page.mouse.up();

  await expect(autoRotate).toHaveAttribute('aria-pressed', 'false');
  await expect(autoRotate).not.toHaveClass(/active/);
});

test('the viewer redraws on its own when the scene or a display flag changes', async ({ page }) => {
  await page.goto('/index.html');
  const result = await page.evaluate(async () => {
    const [{ Viewer }, { buildSceneFromMeshes }] = await Promise.all([
      import('/viewer.js'),
      import('/mesh-loader.js'),
    ]);
    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    const scene = await buildSceneFromMeshes({
      meshes: [{
        positions: [-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0],
        normals: [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1],
        indices: [0, 1, 2, 0, 2, 3],
      }],
    });

    // Deliberately never calls viewer._render(): the loop has to notice each
    // change by itself, which is what makes idle frames skippable.
    const nextFrames = (count) => new Promise((resolve) => {
      let remaining = count;
      const step = () => (remaining-- > 0 ? requestAnimationFrame(step) : resolve());
      step();
    });
    const sample = async () => {
      await nextFrames(3);
      const rgba = new Uint8Array(4);
      viewer.gl.readPixels(32, 32, 1, 1, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, rgba);
      return Array.from(rgba);
    };

    viewer.setScene(scene);
    viewer.showGrid = false;
    viewer.camera.target.set([0, 0, 0]);
    viewer.camera.distance = 3;
    viewer.camera.azimuth = 0;
    viewer.camera.elevation = 0;
    viewer.invalidate();
    const afterScene = await sample();

    viewer.baseColorOnly = true;
    const afterFlag = await sample();

    // An idle viewer must keep the last frame rather than clearing it.
    const whileIdle = await sample();

    viewer.dispose();
    canvas.remove();
    return { afterScene, afterFlag, whileIdle };
  });

  // The quad is lit and facing the camera, so the sampled texel is opaque.
  expect(result.afterScene[3]).toBe(255);
  expect(Math.max(...result.afterScene.slice(0, 3))).toBeGreaterThan(0);
  // Base color bypasses the preview lighting, so the same texel changes.
  expect(result.afterFlag).not.toEqual(result.afterScene);
  expect(result.whileIdle).toEqual(result.afterFlag);
});

test('preview Base color bypasses environment lighting and studio IBL has usable exposure', async ({ page }) => {
  await page.goto('/index.html');
  const pixels = await page.evaluate(async () => {
    const [{ Viewer }, { buildSceneFromMeshes }] = await Promise.all([
      import('/viewer.js'),
      import('/mesh-loader.js'),
    ]);
    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    const scene = await buildSceneFromMeshes({
      meshes: [{
        positions: [-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0],
        normals: [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1],
        indices: [0, 1, 2, 0, 2, 3],
      }],
    });
    viewer.setScene(scene);
    viewer.baseColorOnly = true;
    viewer.showGrid = false;
    viewer.camera.target.set([0, 0, 0]);
    viewer.camera.distance = 3;
    viewer.camera.azimuth = 0;
    viewer.camera.elevation = 0;
    const sample = () => {
      viewer._render();
      const rgba = new Uint8Array(4);
      viewer.gl.readPixels(32, 32, 1, 1, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, rgba);
      return Array.from(rgba);
    };
    const baseColor = sample();
    viewer.baseColorOnly = false;
    Object.assign(scene.materials[0], {
      baseColorFactor: [1, 1, 1, 1], metallic: 0, roughness: 1,
    });
    const whiteRough = sample();
    scene.materials[0].baseColorFactor = [0.5, 0.5, 0.5, 1];
    const grayRough = sample();
    Object.assign(scene.materials[0], {
      baseColorFactor: [1, 1, 1, 1], roughness: 0.5,
    });
    const whiteMedium = sample();
    Object.assign(scene.materials[0], {
      baseColorFactor: [0.8, 0.8, 0.8, 1], metallic: 1,
    });
    const metalMedium = sample();
    viewer.dispose();
    canvas.remove();
    return { baseColor, whiteRough, grayRough, whiteMedium, metalMedium };
  });

  expect(pixels.baseColor[0]).toBe(255);
  expect(pixels.baseColor[1]).toBe(pixels.baseColor[0]);
  expect(pixels.baseColor[2]).toBe(pixels.baseColor[0]);
  expect(pixels.baseColor[3]).toBe(255);
  const displayLuminance = ([red, green, blue]) => 0.2126 * red + 0.7152 * green + 0.0722 * blue;
  expect(displayLuminance(pixels.whiteRough)).toBeGreaterThan(155);
  expect(displayLuminance(pixels.whiteRough)).toBeLessThan(220);
  expect(displayLuminance(pixels.grayRough)).toBeGreaterThan(105);
  expect(displayLuminance(pixels.metalMedium)).toBeGreaterThan(105);
  expect(Math.max(...pixels.whiteMedium.slice(0, 3))).toBeLessThan(240);
});

test('preview smoothing preserves authored 90-degree creases', async ({ page }) => {
  await page.goto('/index.html');
  const normals = await page.evaluate(async () => {
    const { buildSmoothNormalAttribute } = await import('/viewer.js');
    const positions = new Float32Array([
      0, 0, 0, 1, 0, 0, 0, 1, 0,
      0, 0, 0, 0, 0, 1, 1, 0, 0,
    ]);
    const sourceNormals = new Float32Array([
      0, 0, 1, 0, 0, 1, 0, 0, 1,
      0, 1, 0, 0, 1, 0, 0, 1, 0,
    ]);
    const attribute = buildSmoothNormalAttribute({
      mode: 4,
      attributes: {
        POSITION: { bytes: positions, componentType: 5126, components: 3, count: 6 },
        NORMAL: { bytes: sourceNormals, componentType: 5126, components: 3, count: 6 },
      },
      indices: { bytes: new Uint8Array([0, 1, 2, 3, 4, 5]), componentType: 5121, count: 6 },
    });
    return Array.from(attribute.bytes);
  });

  expect(normals.slice(0, 3)).toEqual([0, 0, 1]);
  expect(normals.slice(9, 12)).toEqual([0, 1, 0]);
});

test('scaled cube fixture uses valid node instances of one mesh', async ({ page }) => {
  await page.goto('/index.html');
  const fixture = path.join(repoRoot, 'testdata', 'CubeScaledInstances', 'glTF');
  const source = Array.from(await readFile(path.join(fixture, 'cube_att.gltf')));
  const buffer = Array.from(await readFile(path.join(fixture, 'buffer0.bin')));
  const result = await page.evaluate(async ({ source, buffer }) => {
    const api = await import('/pkg/gltf.js');
    await api.default();
    const asset = api.GltfAsset.withResources(
      new Uint8Array(source),
      { 'buffer0.bin': new Uint8Array(buffer) },
      '2.0',
    );
    const document = JSON.parse(new TextDecoder().decode(asset.json()));
    const roots = document.scenes?.[document.scene ?? 0]?.nodes ?? [];
    return {
      roots,
      meshNodes: document.nodes.filter((node) => node.mesh === 0).length,
      meshes: document.meshes.length,
    };
  }, { source, buffer });

  expect(result).toEqual({ roots: [0], meshNodes: 4, meshes: 1 });
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
  const animationTrigger = page.locator('#anim-clip-trigger');
  await expect(animationTrigger).toContainText('Survey');
  await animationTrigger.click();
  await expect(page.locator('#anim-clip-menu')).toBeVisible();
  await page.locator('.menu-picker-option[data-value="1"]').click();
  await expect(page.locator('#anim-clip')).toHaveValue('1');
  await expect(animationTrigger).toContainText('Walk');
  await expect(animationTrigger).toHaveAttribute('aria-expanded', 'false');
  await animationTrigger.press('ArrowDown');
  await expect(page.locator('#anim-clip')).toHaveValue('2');
  await expect(animationTrigger).toContainText('Run');
  await animationTrigger.press('ArrowUp');
  await expect(page.locator('#anim-clip')).toHaveValue('1');
  await animationTrigger.click();
  const selectedClip = page.locator('.menu-picker-option.selected');
  await selectedClip.press('ArrowDown');
  const runClip = page.locator('.menu-picker-option[data-value="2"]');
  await expect(runClip).toBeFocused();
  await runClip.press('Enter');
  await expect(page.locator('#anim-clip')).toHaveValue('2');
  await expect(animationTrigger).toHaveAttribute('aria-expanded', 'false');
  const smoothNormals = page.locator('#viewer-smooth-normals');
  await expect(smoothNormals).toHaveAttribute('aria-pressed', 'false');
  await smoothNormals.click();
  await expect(smoothNormals).toHaveAttribute('aria-pressed', 'true');
  await smoothNormals.click();
  await expect(smoothNormals).toHaveAttribute('aria-pressed', 'false');
  const webglError = await page.evaluate(
    () => document.getElementById('viewer-canvas')?.getContext('webgl2')?.getError(),
  );
  expect(webglError).toBe(0);

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
  if (await dracoToggle.isEnabled()) {
    const downloadedPath = await download.path();
    expect(downloadedPath).not.toBeNull();
    const decoded = await decodeFirstDracoPrimitive(await readFile(downloadedPath));
    expect(decoded.points).toBe(1728);
    expect(decoded.faces).toBe(576);
    expect(decoded.declaredPoints).toBe(decoded.points);
    expect(decoded.declaredIndices).toBe(decoded.faces * 3);
  }
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

test('preview loads metallic-roughness and emissive PBR textures', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  const fixture = path.join(repoRoot, 'testdata', 'Lantern', 'glTF');
  await page.locator('#file-input').setInputFiles([
    path.join(fixture, 'Lantern.gltf'),
    path.join(fixture, 'Lantern.bin'),
    path.join(fixture, 'Lantern_baseColor.png'),
    path.join(fixture, 'Lantern_roughnessMetallic.png'),
    path.join(fixture, 'Lantern_normal.png'),
    path.join(fixture, 'Lantern_emissive.png'),
  ]);

  await expect(page.locator('#viewer-section')).toBeVisible();
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('Preview failed');
  await expect(page.locator('#console')).not.toContainText('Skipped primitive');
});

// Both alternate image sources take the same path — the viewer names a MIME
// type and the browser decodes it — so they are checked the same way rather
// than twice over in prose. The fixtures are the same image, which is what
// makes one set of expected pixels legitimate for both.
const IMAGE_SOURCES = [
  { extension: 'EXT_texture_webp', file: 'quadrants.webp', mimeType: 'image/webp' },
  { extension: 'EXT_texture_avif', file: 'quadrants.avif', mimeType: 'image/avif' },
];

for (const source of IMAGE_SOURCES) {
  test(`preview decodes an ${source.extension} source into the right pixels`, async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  const fixture = path.join(repoRoot, 'testdata', 'textures');
  await page.locator('#file-input').setInputFiles([
    path.join(fixture, `quadrants-${source.mimeType.slice('image/'.length)}.gltf`),
    path.join(fixture, source.file),
  ]);
  await expect(page.locator('#console')).toContainText('Preview ready');

  // Everything else about these codecs is checked without a browser: that the
  // extension is read as an image source, that the type survives the document
  // model, that content sniffing agrees with the declared type. None of it
  // touches a decoder, because Node has none. This does — the codec belongs to
  // the host, and the only way to find out whether the host used it is to ask
  // for the pixels back.
  const decoded = await page.evaluate(async () => {
    const { state } = await import('/app/state.js');
    const texture = state.viewer.scene.textures.find((entry) => entry && entry.image);
    if (!texture) return { found: false };

    const canvas = document.createElement('canvas');
    canvas.width = texture.image.width;
    canvas.height = texture.image.height;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    context.drawImage(texture.image, 0, 0);
    const at = (x, y) => Array.from(context.getImageData(x, y, 1, 1).data).slice(0, 3);

    const quarter = Math.floor(canvas.width / 4);
    const threeQuarters = Math.floor((canvas.width * 3) / 4);
    return {
      found: true,
      mimeType: texture.mimeType,
      size: [canvas.width, canvas.height],
      topLeft: at(quarter, quarter),
      topRight: at(threeQuarters, quarter),
      bottomLeft: at(quarter, threeQuarters),
      bottomRight: at(threeQuarters, threeQuarters),
      uploaded: state.viewer.glResources.textures.filter(Boolean).length,
    };
  });

  expect(decoded.found).toBe(true);
  expect(decoded.mimeType).toBe(source.mimeType);
  expect(decoded.size).toEqual([64, 64]);
  // The fixture's own quadrants, stated here rather than read from the encoded
  // file, which would be circular. testdata/textures/quadrants.png is the same
  // image if they ever need re-reading by eye. Both fixtures are exact, so
  // there is nothing to tolerate: an off-by-one here means a colour conversion
  // nobody asked for.
  expect(decoded.topLeft).toEqual([220, 40, 40]);
  expect(decoded.topRight).toEqual([40, 200, 60]);
  expect(decoded.bottomLeft).toEqual([50, 90, 230]);
  expect(decoded.bottomRight).toEqual([240, 220, 40]);
  expect(decoded.uploaded).toBeGreaterThan(0);

  // And the extension is not merely tolerated: a source the viewer could not
  // read would be reported, and this asset has no fallback image to fall back
  // to.
  await expect(page.locator('#console')).not.toContainText('requires a transcoder');
  await expect(page.locator('#console')).not.toContainText(source.extension);
  });
}

test('preview uploads its textures to the GPU', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  const fixture = path.join(repoRoot, 'testdata', 'Lantern', 'glTF');
  await page.locator('#file-input').setInputFiles([
    path.join(fixture, 'Lantern.gltf'),
    path.join(fixture, 'Lantern.bin'),
    path.join(fixture, 'Lantern_baseColor.png'),
    path.join(fixture, 'Lantern_roughnessMetallic.png'),
    path.join(fixture, 'Lantern_normal.png'),
    path.join(fixture, 'Lantern_emissive.png'),
  ]);
  await expect(page.locator('#console')).toContainText('Preview ready');

  // Decoding a bitmap is not the same as handing it to WebGL: a scene can
  // carry every image and still draw untextured, which is what an untextured
  // model looks like — flat white.
  const textures = await page.evaluate(async () => {
    const { state } = await import('/app/state.js');
    const scene = state.viewer.scene;
    const gl = state.viewer.glResources.textures;
    return {
      withBitmap: scene.textures.filter((t) => t && t.image).length,
      uploaded: gl.filter(Boolean).length,
      distinct: new Set(gl.filter(Boolean)).size,
    };
  });
  expect(textures.withBitmap).toBeGreaterThan(0);
  expect(textures.uploaded).toBe(textures.withBitmap);
  // One GL texture per distinct image and sampler, not one per slot.
  expect(textures.distinct).toBeLessThanOrEqual(textures.uploaded);
});

test('preview loads normal and occlusion PBR textures', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  const fixture = path.join(repoRoot, 'testdata', 'SphereAllSame');
  await page.locator('#file-input').setInputFiles([
    path.join(fixture, 'sphere_texture_all.gltf'),
    path.join(fixture, 'buffer0.bin'),
    path.join(fixture, '256x256_all_orange.png'),
    path.join(fixture, '256x256_all_blue.png'),
    path.join(fixture, '256x256_all_red.png'),
    path.join(fixture, '256x256_all_green.png'),
  ]);

  await expect(page.locator('#viewer-section')).toBeVisible();
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('Preview failed');
  await expect(page.locator('#console')).not.toContainText('Skipped primitive');
});

test('converter explains a missing external glTF buffer', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles(
    path.join(repoRoot, 'testdata', 'Fox', 'glTF', 'Fox.gltf'),
  );

  // Said twice over, and the first is the useful one: the reader names what it
  // could not resolve, but the intake knows before anything is opened, because
  // it looked the URI up in the selection and did not find it.
  await expect(page.locator('#console')).toContainText(
    'Referenced file not in the selection: Fox.bin',
  );
  await expect(page.locator('#console')).toContainText('External resource denied: Fox.bin');
  await expect(page.locator('#console')).toContainText(
    'Drop the whole folder instead, or select the .gltf together with every referenced .bin and image.',
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

  const baseColor = page.locator('#viewer-base-color');
  await expect(baseColor).toHaveAttribute('aria-pressed', 'false');
  await baseColor.click();
  await expect(baseColor).toHaveAttribute('aria-pressed', 'true');
  await expect(baseColor).toHaveClass(/active/);

  const hasContext = await page.evaluate(() => {
    const canvas = document.getElementById('viewer-canvas');
    const gl = canvas && canvas.getContext('webgl2');
    return Boolean(gl);
  });
  expect(hasContext).toBe(true);

  const webglError = await page.evaluate(() => {
    const gl = document.getElementById('viewer-canvas')?.getContext('webgl2');
    return gl?.getError();
  });
  expect(webglError).toBe(0);

  await expect(page.locator('#console')).not.toContainText('undefined');
});

test('mobile preview height remains stable after clearing a desktop-loaded file', async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 800 });
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles(
    path.join(repoRoot, 'testdata', 'Box', 'glTF_Binary', 'Box.glb'),
  );
  await expect(page.locator('#console')).toContainText('Preview ready');

  await page.setViewportSize({ width: 800, height: 900 });
  const loadedHeight = await page.locator('#viewer-section').evaluate((element) => element.getBoundingClientRect().height);
  await page.locator('#clear-file').click();
  await expect(page.locator('#drop-zone')).toBeVisible();
  const clearedHeight = await page.locator('#viewer-section').evaluate((element) => element.getBoundingClientRect().height);

  expect(Math.abs(loadedHeight - clearedHeight)).toBeLessThan(1);
  await expect(page.locator('#drop-zone')).toHaveCSS('display', 'grid');
});

test('converter exports a glTF document through the FBX writer', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles(
    path.join(repoRoot, 'testdata', 'Box', 'glTF_Binary', 'Box.glb'),
  );

  await expect(page.locator('#console')).toContainText('Preview ready');
  await page.locator('[data-choice-for="export-format"] [data-value="fbx"]').click();
  const downloadPromise = page.waitForEvent('download');
  await page.locator('#export-btn').click();
  const download = await downloadPromise;
  expect(download.suggestedFilename()).toBe('export.fbx');

  const downloadedPath = await download.path();
  expect(downloadedPath).not.toBeNull();
  const bytes = await readFile(downloadedPath);
  expect(bytes.subarray(0, 21).toString('binary')).toBe('Kaydara FBX Binary  \u0000');
  await expect(page.locator('#console')).toContainText('Export complete!');
  await expect(page.locator('#console')).not.toContainText('Document export to FBX is not supported');

  await page.locator('#file-input').setInputFiles({
    name: 'roundtrip.fbx',
    mimeType: 'application/octet-stream',
    buffer: bytes,
  });
  await expect(page.locator('#console')).toContainText('Successfully parsed roundtrip.fbx');
  await expect(page.locator('#console')).not.toContainText('Failed to parse file');
});

test('preview renders a glTF morph target animation', async ({ page }) => {
  const pageErrors = [];
  page.on('pageerror', (error) => pageErrors.push(error.message));

  await page.goto('/index.html');
  await waitForConverterReady(page);
  const fixture = path.join(repoRoot, 'testdata', 'KhronosSampleModels', 'AnimatedMorphCube', 'glTF');
  await page.locator('#file-input').setInputFiles([
    path.join(fixture, 'AnimatedMorphCube.gltf'),
    path.join(fixture, 'AnimatedMorphCube.bin'),
  ]);

  await expect(page.locator('#viewer-section')).toBeVisible();
  await expect(page.locator('#viewer-animation')).toBeVisible();
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('weights channels are not supported');
  await expect(page.locator('#console')).not.toContainText('Preview failed');
  await page.waitForTimeout(100);
  expect(pageErrors).toEqual([]);

  const webglError = await page.evaluate(() => {
    const gl = document.getElementById('viewer-canvas')?.getContext('webgl2');
    return gl?.getError();
  });
  expect(webglError).toBe(0);
});

test('preview animates every target of a long morph cycle', async ({ page }) => {
  await page.goto('/index.html');
  const samples = await page.evaluate(async () => {
    const [{ Viewer }, { createSceneDocument }, { buildViewerSceneFromDocument }] = await Promise.all([
      import('/viewer.js'),
      import('/scene-document.js'),
      import('/scene-document-viewer.js'),
    ]);

    const TARGETS = 6;
    const bytes = (values) => new Uint8Array(
      values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength),
    );
    const accessor = (values, components) => ({
      bytes: bytes(values), componentType: 5126, components, count: values.length / components,
    });
    // Every target sweeps the quad far off screen, so a frame that drops its
    // target is visible as the quad still covering the sampled pixel.
    const offScreen = accessor(new Float32Array(Array.from(
      { length: 12 }, (_, i) => (i % 3 === 1 ? 100 : 0),
    )), 3);
    // One target per keyframe, exactly how a shape-key flap cycle is exported.
    const times = Array.from({ length: TARGETS + 1 }, (_, key) => key * 0.1);
    const weights = new Float32Array((TARGETS + 1) * TARGETS);
    for (let key = 1; key <= TARGETS; key++) weights[key * TARGETS + (key - 1)] = 1;

    const scene = buildViewerSceneFromDocument(createSceneDocument({
      materials: [{
        baseColorFactor: [1, 0, 0, 1],
        metallicFactor: 0,
        roughnessFactor: 1,
        emissiveFactor: [0, 0, 0],
      }],
      accessors: [
        accessor(new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]), 3),
        {
          bytes: bytes(new Uint16Array([0, 1, 2, 0, 2, 3])),
          componentType: 5123,
          components: 1,
          count: 6,
        },
        ...Array.from({ length: TARGETS }, () => offScreen),
        accessor(new Float32Array(times), 1),
        accessor(weights, TARGETS),
      ],
      meshes: [{
        primitives: [{
          attributes: { POSITION: 0 },
          indices: 1,
          material: 0,
          targets: Array.from({ length: TARGETS }, (_, i) => ({ POSITION: 2 + i })),
        }],
        weights: new Array(TARGETS).fill(0),
      }],
      nodes: [{
        name: 'Flap',
        translation: [0, 0, 0],
        rotation: [0, 0, 0, 1],
        scale: [1, 1, 1],
        mesh: 0,
      }],
      rootNodes: [0],
      animations: [{
        name: 'Cycle',
        duration: times[times.length - 1],
        samplers: [{ input: 2 + TARGETS, output: 3 + TARGETS, interpolation: 'LINEAR' }],
        channels: [{ sampler: 0, node: 0, path: 'weights' }],
      }],
    }));

    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    viewer.setScene(scene);
    viewer.showGrid = false;
    viewer.animation.playing = false;
    viewer.camera.target.set([0, 0, 0]);
    viewer.camera.distance = 3;
    viewer.camera.azimuth = 0;
    viewer.camera.elevation = 0;

    const sample = (time) => {
      viewer.seekAnimation(time);
      viewer._render();
      const rgba = new Uint8Array(4);
      viewer.gl.readPixels(32, 32, 1, 1, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, rgba);
      return Array.from(rgba);
    };
    const rest = sample(0);
    const firstTarget = sample(0.1);
    const lastTarget = sample(times[TARGETS]);
    const backToRest = sample(0);
    const glError = viewer.gl.getError();
    viewer.dispose();
    canvas.remove();
    return { rest, firstTarget, lastTarget, backToRest, glError };
  });

  expect(samples.glError).toBe(0);
  // The rest pose covers the sampled pixel; any active target clears it.
  expect(samples.firstTarget).not.toEqual(samples.rest);
  expect(samples.lastTarget).toEqual(samples.firstTarget);
  expect(samples.backToRest).toEqual(samples.rest);
});

test('preview blends more than four morph targets at once', async ({ page }) => {
  await page.goto('/index.html');
  const samples = await page.evaluate(async () => {
    const [{ Viewer }, { createSceneDocument }, { buildViewerSceneFromDocument }] = await Promise.all([
      import('/viewer.js'),
      import('/scene-document.js'),
      import('/scene-document-viewer.js'),
    ]);

    const bytes = (values) => new Uint8Array(
      values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength),
    );
    const accessor = (values, components) => ({
      bytes: bytes(values), componentType: 5126, components, count: values.length / components,
    });
    // Six targets that cancel out: the first four push the quad up by 1, the
    // last two pull it down by 2. Blending all six leaves the quad centred,
    // while blending only the four strongest sweeps it off screen.
    const shift = (dy) => accessor(new Float32Array(Array.from(
      { length: 12 }, (_, i) => (i % 3 === 1 ? dy : 0),
    )), 3);
    const deltas = [shift(1), shift(1), shift(1), shift(1), shift(-2), shift(-2)];

    const scene = buildViewerSceneFromDocument(createSceneDocument({
      materials: [{
        baseColorFactor: [1, 0, 0, 1],
        metallicFactor: 0,
        roughnessFactor: 1,
        emissiveFactor: [0, 0, 0],
      }],
      accessors: [
        accessor(new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]), 3),
        {
          bytes: bytes(new Uint16Array([0, 1, 2, 0, 2, 3])),
          componentType: 5123,
          components: 1,
          count: 6,
        },
        ...deltas,
      ],
      meshes: [{
        primitives: [{
          attributes: { POSITION: 0 },
          indices: 1,
          material: 0,
          targets: deltas.map((_, i) => ({ POSITION: 2 + i })),
        }],
        weights: new Array(deltas.length).fill(0),
      }],
      nodes: [{
        name: 'Blend',
        translation: [0, 0, 0],
        rotation: [0, 0, 0, 1],
        scale: [1, 1, 1],
        mesh: 0,
      }],
      rootNodes: [0],
    }));

    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    viewer.setScene(scene);
    viewer.showGrid = false;
    viewer.camera.target.set([0, 0, 0]);
    viewer.camera.distance = 3;
    viewer.camera.azimuth = 0;
    viewer.camera.elevation = 0;

    const sample = (weights) => {
      scene.nodes[0].weights.set(weights);
      viewer._render();
      const rgba = new Uint8Array(4);
      viewer.gl.readPixels(32, 32, 1, 1, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, rgba);
      return Array.from(rgba);
    };
    const rest = sample([0, 0, 0, 0, 0, 0]);
    const allSix = sample([1, 1, 1, 1, 1, 1]);
    const firstFour = sample([1, 1, 1, 1, 0, 0]);
    const glError = viewer.gl.getError();
    viewer.dispose();
    canvas.remove();
    return { rest, allSix, firstFour, glError };
  });

  expect(samples.glError).toBe(0);
  // All six cancel back to the rest pose; the up-shifts alone clear the pixel.
  expect(samples.allSix).toEqual(samples.rest);
  expect(samples.firstFour).not.toEqual(samples.rest);
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

/**
 * A normal map must reach the surface on a centimetre-sized model.
 *
 * The preview has no TANGENT attribute, so the tangent frame comes from
 * screen-space derivatives, whose magnitude is world units per pixel. Judging
 * that frame by an absolute threshold makes normal mapping vanish on small
 * models while leaving large ones intact — silently, and only in the shading.
 * Rendering the same quad with and without the map is what exposes it.
 */
test('normal maps shade a model far smaller than one unit', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);

  const shot = async (name, gltf) => {
    await page.locator('#file-input').setInputFiles({
      name,
      mimeType: 'model/gltf+json',
      buffer: Buffer.from(gltf),
    });
    await expect(page.locator('#console')).toContainText(`Successfully parsed ${name}`);
    await expect(page.locator('#console')).toContainText('Preview ready');
    const image = await page.locator('#viewer-canvas').screenshot();
    await page.locator('#clear-file').click();
    return image.toString('base64');
  };

  const mapped = await shot('normal-mapped.gltf', normalMappedQuad({ normalMap: true }));
  const flat = await shot('flat.gltf', normalMappedQuad({ normalMap: false }));

  // Both frames are decoded in the page: the screenshots are PNGs, and the
  // browser is the only PNG decoder this suite has.
  const difference = await page.evaluate(async ([a, b]) => {
    const decode = async (base64) => {
      const bytes = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
      const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = canvas.getContext('2d');
      context.drawImage(bitmap, 0, 0);
      return context.getImageData(0, 0, bitmap.width, bitmap.height).data;
    };
    const [left, right] = await Promise.all([decode(a), decode(b)]);
    if (left.length !== right.length) return { sameSize: false, meanAbsolute: 0 };
    let total = 0;
    for (let index = 0; index < left.length; index += 4) {
      total += Math.abs(left[index] - right[index])
        + Math.abs(left[index + 1] - right[index + 1])
        + Math.abs(left[index + 2] - right[index + 2]);
    }
    let changed = 0;
    let peak = 0;
    for (let index = 0; index < left.length; index += 4) {
      const delta = Math.max(
        Math.abs(left[index] - right[index]),
        Math.abs(left[index + 1] - right[index + 1]),
        Math.abs(left[index + 2] - right[index + 2]),
      );
      if (delta > 8) changed += 1;
      peak = Math.max(peak, delta);
    }
    return { sameSize: true, meanAbsolute: total / (left.length / 4) / 3, changedFraction: changed / (left.length / 4), peak };
  }, [mapped, flat]);

  expect(difference.sameSize).toBe(true);
  // Losing the tangent frame makes the two frames identical, so the gate only
  // has to separate "the map shades the surface" from "nothing happened"; the
  // measured difference on a working build is several times these bounds.
  expect(difference.meanAbsolute).toBeGreaterThan(2);
  expect(difference.changedFraction).toBeGreaterThan(0.05);
});

/**
 * The Khronos material-extension comparisons open and shade.
 *
 * The synthetic fixtures elsewhere in this file pin one behaviour each; these
 * are the reference assets for the four extensions the preview grew, authored
 * to make the extension's effect the visible difference between rows. Loading
 * them is the check that the shading path holds up on real content rather than
 * on quads built to suit it.
 */
const MATERIAL_COMPARISONS = {
  CompareClearcoat: 'KHR_materials_clearcoat',
  CompareIor: 'KHR_materials_ior',
  CompareSpecular: 'KHR_materials_specular',
  CompareEmissiveStrength: 'KHR_materials_emissive_strength',
};

for (const [model, extension] of Object.entries(MATERIAL_COMPARISONS)) {
  test(`preview opens ${model}`, async ({ page }) => {
    await page.goto('/index.html');
    await waitForConverterReady(page);
    await page.locator('#file-input').setInputFiles(
      path.join(repoRoot, 'testdata', 'KhronosSampleModels', model, 'glTF_Binary', `${model}.glb`),
    );

    await expect(page.locator('#console')).toContainText('Preview ready');
    await expect(page.locator('#console')).not.toContainText('Preview failed');
    await expect(page.locator('#console')).not.toContainText('Skipped primitive');
    // Named rather than blanket: CompareIor also carries transmission and
    // volume, which the preview really does ignore and says so honestly. The
    // claim here is only about the extension the model exists to compare.
    await expect(page.locator('#console')).not.toContainText(`ignored: ${extension}`);
    await expect(page.locator('#console')).not.toContainText(`, ${extension}`);
    // The frame must not be a flat backdrop: something was shaded into it.
    // Read back through a screenshot, because the viewer's context does not
    // preserve its drawing buffer and reads back blank between frames.
    const shot = (await page.locator('#viewer-canvas').screenshot()).toString('base64');
    const distinct = await page.evaluate(async (base64) => {
      const bytes = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
      const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = canvas.getContext('2d');
      context.drawImage(bitmap, 0, 0);
      const { data } = context.getImageData(0, 0, canvas.width, canvas.height);
      const seen = new Set();
      for (let index = 0; index < data.length; index += 4) {
        seen.add((data[index] >> 3 << 10) | (data[index + 1] >> 3 << 5) | (data[index + 2] >> 3));
      }
      return seen.size;
    }, shot);
    expect(distinct).toBeGreaterThan(8);
  });
}

/**
 * KHR_texture_transform reaches slots other than base color.
 *
 * The reader has always handed the transform over for every textureInfo, and
 * the scene model has always carried it, but the shader read it on base color
 * alone: nine slots silently sampled untransformed UVs. Emissive is the slot
 * under test because it is additive, so the frame's colour is the sampled
 * texel rather than a lighting result.
 */
test('the texture transform reaches slots other than base color', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);

  const shot = async (name, gltf) => {
    await page.locator('#file-input').setInputFiles({
      name,
      mimeType: 'model/gltf+json',
      buffer: Buffer.from(gltf),
    });
    await expect(page.locator('#console')).toContainText(`Successfully parsed ${name}`);
    await expect(page.locator('#console')).toContainText('Preview ready');
    const image = await page.locator('#viewer-canvas').screenshot();
    await page.locator('#clear-file').click();
    return image.toString('base64');
  };

  const red = await shot('emissive-left.gltf', emissiveTransformQuad({ offset: 0 }));
  const green = await shot('emissive-right.gltf', emissiveTransformQuad({ offset: 0.5 }));

  const measure = await page.evaluate(async ([a, b]) => {
    const decode = async (base64) => {
      const bytes = Uint8Array.from(atob(base64), (character) => character.charCodeAt(0));
      const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = canvas.getContext('2d');
      context.drawImage(bitmap, 0, 0);
      return context.getImageData(0, 0, bitmap.width, bitmap.height).data;
    };
    // The backdrop fills most of the frame, so the emitting quad is measured
    // over the pixels where one channel actually dominates.
    const dominance = (pixels) => {
      let redPixels = 0;
      let greenPixels = 0;
      for (let index = 0; index < pixels.length; index += 4) {
        const [r, g, blue] = [pixels[index], pixels[index + 1], pixels[index + 2]];
        if (r > g + 40 && r > blue + 40) redPixels += 1;
        if (g > r + 40 && g > blue + 40) greenPixels += 1;
      }
      const total = pixels.length / 4;
      return { red: redPixels / total, green: greenPixels / total };
    };
    const [left, right] = await Promise.all([decode(a), decode(b)]);
    let identical = left.length === right.length;
    for (let index = 0; identical && index < left.length; index += 1) {
      if (left[index] !== right[index]) identical = false;
    }
    return { left: dominance(left), right: dominance(right), identical };
  }, [red, green]);

  // Ignoring the transform renders both fixtures as the same half-and-half
  // frame, which fails all three of these at once.
  expect(measure.identical).toBe(false);
  expect(measure.left.red).toBeGreaterThan(measure.left.green * 4 + 0.02);
  expect(measure.right.green).toBeGreaterThan(measure.right.red * 4 + 0.02);
});

/**
 * An export that costs something has to say so.
 *
 * Five of the six export routes used to compute warnings and drop them; the
 * flattening route did not even produce any. Exporting a scene-bearing glTF to
 * OBJ is the cheapest deterministic case: OBJ has nowhere to put materials,
 * textures or a hierarchy, and the user should not have to discover that by
 * opening the result.
 */
test('exporting a scene to a flat format says what it costs', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles(
    path.join(repoRoot, 'testdata', 'Box', 'glTF_Binary', 'Box.glb'),
  );
  await expect(page.locator('#console')).toContainText('Preview ready');

  await page.locator('[data-choice-for="export-format"] [data-value="obj"]').click();
  const downloadPromise = page.waitForEvent('download');
  await page.locator('#export-btn').click();
  await downloadPromise;

  await expect(page.locator('#console')).toContainText('Export complete!');
  await expect(page.locator('#scene-warning-list')).toContainText('flattens the scene');
});

test('one surface program per set of texture slots, not per material', async ({ page }) => {
  await page.goto('/index.html');
  // A single program declaring every slot cannot survive the layered material
  // extensions: the slot list plus the frame's own samplers already needs more
  // than the sixteen texture units WebGL2 guarantees. So a program is built for
  // the slots one material actually binds, and materials that bind the same set
  // share it — including across scenes, which is what keeps the count bounded
  // by the vocabulary rather than by the asset.
  const observed = await page.evaluate(async (pngBytes) => {
    const [{ Viewer }, { createSceneDocument }, { buildViewerSceneFromDocument }, { hydrateSceneTextures }] =
      await Promise.all([
        import('/viewer.js'),
        import('/scene-document.js'),
        import('/scene-document-viewer.js'),
        import('/scene-document-textures.js'),
      ]);

    const bytes = (values) => new Uint8Array(
      values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength),
    );
    const accessor = (values, components, componentType = 5126) => ({
      bytes: bytes(values), componentType, components, count: values.length / components,
    });
    const quad = (material) => ({
      attributes: { POSITION: 0, NORMAL: 1, TEXCOORD_0: 2 },
      indices: 3,
      material,
    });

    // Four materials over three distinct slot sets: two bind base colour only,
    // one adds emissive, one binds nothing.
    const document_ = createSceneDocument({
      resources: [{ mimeType: 'image/png', bytes: new Uint8Array(pngBytes), name: 'map.png' }],
      textures: [{ resource: 0, sampler: {} }],
      materials: [
        { baseColorFactor: [1, 1, 1, 1], baseColorTexture: { texture: 0 } },
        { baseColorFactor: [0.5, 0.5, 0.5, 1], baseColorTexture: { texture: 0 } },
        {
          baseColorFactor: [1, 1, 1, 1],
          baseColorTexture: { texture: 0 },
          emissiveFactor: [1, 1, 1],
          emissiveTexture: { texture: 0 },
        },
        { baseColorFactor: [1, 0, 0, 1] },
      ],
      accessors: [
        accessor(new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]), 3),
        accessor(new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]), 3),
        accessor(new Float32Array([0, 1, 1, 1, 1, 0, 0, 0]), 2),
        accessor(new Uint16Array([0, 1, 2, 0, 2, 3]), 1, 5123),
      ],
      meshes: [{ primitives: [quad(0), quad(1), quad(2), quad(3)] }],
      nodes: [{ name: 'Quads', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0 }],
      rootNodes: [0],
    });

    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    viewer.showGrid = false;
    viewer.setScene(await hydrateSceneTextures(buildViewerSceneFromDocument(document_)));
    viewer._render();
    const afterFirst = viewer.surfacePrograms.size;
    // A second scene over the same slot sets must not link anything new.
    viewer.setScene(await hydrateSceneTextures(buildViewerSceneFromDocument(document_)));
    viewer._render();
    const afterSecond = viewer.surfacePrograms.size;
    const glError = viewer.gl.getError();

    // The program built for no slots declares no samplers at all; the one for
    // base colour declares exactly one.
    const sources = [[], ['BASE_COLOR']].map((slots) => {
      const source = viewer.surfacePrograms.get(slots);
      return { slots: source.slots.length, samplers: Object.keys(source.uniforms).filter((name) => (
        name === 'uBaseColor' || name === 'uEmissive'
      ) && source.uniforms[name] !== null).length };
    });

    viewer.dispose();
    canvas.remove();
    return { afterFirst, afterSecond, glError, sources };
  }, Array.from(solidColorPng()));

  expect(observed.glError).toBe(0);
  // Three slot sets over four materials, drawn twice.
  expect(observed.afterFirst).toBe(3);
  expect(observed.afterSecond).toBe(3);
  expect(observed.sources[0]).toEqual({ slots: 0, samplers: 0 });
  expect(observed.sources[1]).toEqual({ slots: 1, samplers: 1 });
});

test('the summary says what became of every extension the file declared', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  // Three fates in one file: a material layer glTF can carry back out, a scene
  // extension the same, and one nothing acts on. Each was reported somewhere
  // already - in a preview warning, a document warning, an export warning -
  // and the point of the section is that they are one answer about one file.
  await page.locator('#file-input').setInputFiles({
    name: 'mixed-extensions.gltf',
    mimeType: 'model/gltf+json',
    buffer: Buffer.from(JSON.stringify({
      asset: { version: '2.0' },
      extensionsUsed: ['KHR_materials_clearcoat', 'KHR_lights_punctual', 'KHR_materials_pbrSpecularGlossiness'],
      extensionsRequired: ['KHR_materials_pbrSpecularGlossiness'],
      extensions: {
        KHR_lights_punctual: { lights: [{ type: 'point', color: [1, 1, 1], intensity: 2 }] },
      },
      buffers: [{
        byteLength: 36,
        uri: 'data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA',
      }],
      bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }],
      accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] }],
      materials: [{ extensions: { KHR_materials_clearcoat: { clearcoatFactor: 1 } } }],
      meshes: [{ primitives: [{ attributes: { POSITION: 0 }, material: 0 }] }],
      nodes: [{ mesh: 0 }, { extensions: { KHR_lights_punctual: { light: 0 } } }],
      scenes: [{ nodes: [0, 1] }],
      scene: 0,
    })),
  });
  await expect(page.locator('#console')).toContainText('Preview ready');

  const lines = await page.locator('#scene-extension-reach li').allTextContents();
  expect(lines).toHaveLength(2);
  // Worst first, and the file's own "you may not skip this" carried through:
  // it is the difference between a poorer export and a wrong one.
  expect(lines[0]).toContain('KHR_materials_pbrSpecularGlossiness (required)');
  expect(lines[0]).toContain('not understood');
  // Un-understood is not the same as lost: the glTF route rewrites the asset
  // in place, so this JSON is copied out with everything around it.
  expect(lines[0]).toContain('copied unchanged into exported glTF');
  expect(lines[1]).toContain('KHR_materials_clearcoat');
  expect(lines[1]).toContain('KHR_lights_punctual');
  expect(lines[1]).toContain('OBJ, PLY and FBX cannot state it');
});

test('a file that declares no extensions gets no reach section', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles({
    name: 'plain.gltf',
    mimeType: 'model/gltf+json',
    buffer: Buffer.from(embeddedTriangle()),
  });
  await expect(page.locator('#console')).toContainText('Preview ready');
  // "Nothing to report" about a plain glTF is noise, so the section is absent
  // rather than empty.
  await expect(page.locator('#scene-extension-reach')).toBeHidden();
});

test('KHR_materials_variants offers every choice and shows the one picked', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  // A variant is a choice about how to look at the scene, not a property of
  // it, so the document carries every alternative and no selection. The picker
  // is where the selection is made, and it must reach the frame.
  await page.locator('#file-input').setInputFiles({
    name: 'variants.gltf',
    mimeType: 'model/gltf+json',
    buffer: Buffer.from(JSON.stringify({
      asset: { version: '2.0' },
      extensionsUsed: ['KHR_materials_variants'],
      extensions: { KHR_materials_variants: { variants: [{ name: 'Ruby' }, { name: 'Emerald' }] } },
      buffers: [{
        byteLength: 36,
        uri: 'data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA',
      }],
      bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }],
      accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] }],
      materials: [
        { name: 'Plain', emissiveFactor: [0, 0, 0] },
        { name: 'Ruby', emissiveFactor: [1, 0, 0] },
        { name: 'Emerald', emissiveFactor: [0, 1, 0] },
      ],
      meshes: [{
        primitives: [{
          attributes: { POSITION: 0 },
          material: 0,
          extensions: {
            KHR_materials_variants: {
              mappings: [{ material: 1, variants: [0] }, { material: 2, variants: [1] }],
            },
          },
        }],
      }],
      nodes: [{ mesh: 0 }],
      scenes: [{ nodes: [0] }],
      scene: 0,
    })),
  });
  await expect(page.locator('#console')).toContainText('Preview ready');

  // Filled by the summary that renders the tree, in the same paint. Read off
  // the finished preview instead, the control appeared a paint after the tree
  // it sits above and pushed it down the moment a file with variants loaded.
  // Asserted by re-running the summary alone rather than by watching for
  // reflow: a small fixture parses and previews inside one frame, so the jump
  // is real but not always observable.
  const filledBySummary = await page.evaluate(async () => {
    const { renderSceneDocumentSummary } = await import('/app/scene-report.js');
    const { state } = await import('/app/state.js');
    document.querySelector('#viewer-variant').replaceChildren();
    renderSceneDocumentSummary(state.currentSceneDocument);
    return [...document.querySelectorAll('#viewer-variant option')].map((option) => option.textContent);
  });
  expect(filledBySummary).toEqual(['Ruby', 'Emerald']);

  // The picker names what the file offers, in the panel that lists what the
  // file contains, and draws its own list because a native select's popup is
  // drawn by the platform in the platform's palette.
  await expect(page.locator('#viewer-variant-picker')).toBeVisible();
  // Exactly what the file declares, and nothing else. There used to be a
  // leading entry of our own for "no variant" — the extension defines no such
  // thing, the primitive's `material` being the fallback for readers that
  // cannot follow it — and since a file usually writes that fallback as one of
  // the variants' own materials, the extra row drew the same image as the row
  // under it.
  expect(await page.locator('#viewer-variant option').allTextContents()).toEqual(['Ruby', 'Emerald']);
  // And one of them is in force from the start, rather than a state that is
  // none of them.
  await expect(page.locator('#viewer-variant-trigger')).toHaveText('Ruby');
  await expect(page.locator('#viewer-variant')).toHaveValue('0');
  await expect(page.locator('#viewer-variant-menu')).toBeHidden();

  const emissiveOf = async () => page.evaluate(async () => {
    const { state } = await import('/app/state.js');
    const scene = state.viewer.scene;
    const index = scene.meshes[0].primitives[0].materialIndex;
    return { index, emissive: [...(scene.materials[index]?.emissiveFactor ?? [])] };
  });

  const ruby = await emissiveOf();
  // Where the user had put the camera, and what was playing, before the switch.
  const viewBefore = await page.evaluate(async () => {
    const { state } = await import('/app/state.js');
    const viewer = state.viewer;
    viewer.camera.azimuth += 0.7;
    viewer.camera.elevation -= 0.3;
    viewer.camera.distance *= 0.6;
    viewer._render();
    return {
      azimuth: viewer.camera.azimuth,
      elevation: viewer.camera.elevation,
      distance: viewer.camera.distance,
      target: [...viewer.camera.target],
    };
  });

  // Driven the way a user drives it. The control is a listbox over a real
  // select — the platform draws a native popup in the platform's palette, which
  // is wrong over a dark viewport — so the click has to go through the trigger
  // and the option, and the select is checked afterwards for still holding the
  // value everything else reads.
  await page.locator('#viewer-variant-trigger').click();
  await expect(page.locator('#viewer-variant-menu')).toBeVisible();

  // The chevron is two edges of a square, so its ink sits off the centre of its
  // own box and the box has to ride against it — up while it points down, down
  // while it points up. Compensated in one state only, it reads as an arrow
  // that drifts when the list opens; the amounts differ by a tenth of a pixel
  // because the mark does not rasterize to the same shape upside down, so what
  // is checked is the direction and the rough size, not an exact figure.
  const chevron = () => page.evaluate(() => {
    const trigger = document.getElementById('viewer-variant-trigger');
    const box = trigger.getBoundingClientRect();
    const mark = trigger.querySelector('.menu-picker-chevron').getBoundingClientRect();
    return +((mark.top + mark.height / 2) - (box.top + box.height / 2)).toFixed(2);
  });
  // After the flip has finished: read at the click, it is still on its way and
  // the number is whatever the 140ms transition had reached.
  await page.waitForTimeout(300);
  const openOffset = await chevron();
  expect(openOffset).toBeGreaterThan(1.5);
  expect(openOffset).toBeLessThan(2.5);

  await page.locator('#viewer-variant-menu .menu-picker-option[data-value="1"]').click();
  await expect(page.locator('#viewer-variant')).toHaveValue('1');
  await expect(page.locator('#viewer-variant-menu')).toBeHidden();
  await page.waitForTimeout(300);
  const closedOffset = await chevron();
  expect(closedOffset).toBeLessThan(-1.5);
  expect(closedOffset).toBeGreaterThan(-2.5);
  // Waiting on the console would be satisfied by the "Preview ready" the first
  // load already printed, so wait for the scene the picker rebuilds. It has to
  // be expect.poll rather than waitForFunction: the latter takes the promise an
  // async callback returns as its truthy result and stops waiting at once.
  await expect.poll(async () => (await emissiveOf()).index, { timeout: 10000 }).toBe(2);
  const emerald = await emissiveOf();

  // The primitive took a different material, and the one the variant names.
  // Neither is material 0: that is the fixture's fallback, which a reader that
  // understands the extension never shows.
  expect(ruby.index).toBe(1);
  expect(ruby.emissive[0]).toBeGreaterThan(0.5);
  expect(emerald.index).toBe(2);
  expect(emerald.emissive[1]).toBeGreaterThan(0.5);

  // The field keeps looking like the focused one afterwards. Styling only
  // focus-visible meant a field chosen with the mouse went back to looking
  // untouched the moment its list closed, while still being what the keyboard
  // would act on -- focus that is real but invisible.
  // Only what is on screen. Whether the field holds focus is asserted on its
  // own; folding it into this string would make the comparison differ for a
  // reason that has nothing to do with what the user can see.
  const paint = () => page.evaluate(() => {
    const style = getComputedStyle(document.querySelector('#viewer-variant-trigger'));
    return `${style.borderColor}|${style.backgroundColor}`;
  });
  // Pointer away and focus dropped, then left to settle: these colours are
  // transitioned, so reading either end of the comparison too early samples a
  // value part way between and the comparison stops meaning anything.
  await page.mouse.move(0, 0);
  await page.evaluate(() => document.querySelector('#viewer-variant-trigger').blur());
  await page.waitForTimeout(300);
  const resting = await paint();
  await page.locator('#viewer-variant-trigger').click();
  // The variant already in force, so what is under test is the click itself.
  await page.locator('#viewer-variant-menu .menu-picker-option[data-value="1"]').click();
  // Pointer away, or this measures :hover -- the cursor stays over the field a
  // click landed on, and hover paints it the same way focus does.
  await page.mouse.move(0, 0);
  // Compared once it has settled, not polled: `not.toBe` is satisfied by the
  // first sample that differs, and every sample during the fade back toward
  // resting differs. The transition is 140ms.
  await page.waitForTimeout(300);
  expect(await paint()).not.toBe(resting);
  expect(await page.evaluate(() => document.activeElement?.id)).toBe('viewer-variant-trigger');

  // Keyboard use must survive the reload the choice kicks off. Arrowing in the
  // open list commits on every step, the commit rebuilds the preview, the
  // preview re-renders the panel and the panel syncs the picker -- which used
  // to replace the option buttons unconditionally, destroying the one holding
  // focus. A user pressing Down once landed on the body with a dead control.
  // Opened from the keyboard, which is the path that puts focus into the list;
  // a mouse-opened list leaves focus on the field, so arrows are the field's to
  // handle and there is no ring to jump around under the pointer.
  await page.locator('#viewer-variant-trigger').focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('#viewer-variant-menu')).toBeVisible();
  expect(await page.evaluate(() => document.activeElement?.id)).toBe('viewer-variant-option-1');
  await page.keyboard.press('ArrowUp');
  await expect.poll(async () => (await emissiveOf()).index, { timeout: 10000 }).toBe(1);
  expect(await page.evaluate(() => document.activeElement?.id)).toBe('viewer-variant-option-0');
  await page.keyboard.press('Escape');

  // And the view did not move. A variant swaps materials and nothing else, so
  // re-framing shows the user the same model from somewhere other than where
  // they had put it — which is what loading a scene normally does, and what
  // this path has to opt out of.
  const viewAfter = await page.evaluate(async () => {
    const { state } = await import('/app/state.js');
    const viewer = state.viewer;
    return {
      azimuth: viewer.camera.azimuth,
      elevation: viewer.camera.elevation,
      distance: viewer.camera.distance,
      target: [...viewer.camera.target],
    };
  });
  expect(viewAfter).toEqual(viewBefore);
});

/**
 * The list stays inside the control it belongs to, and inside the panel.
 *
 * Sized to its longest entry it grew wider than its trigger and out over the
 * panel edge; with enough entries it ran past the bottom of the sidebar, which
 * scrolls and therefore clips it. Both read as the list sliding away from the
 * control that opened it, and neither is visible with the two short variants
 * the other test uses -- so this one declares nine, one of them long.
 */
test('the variant list stays within its control and its panel', async ({ page }) => {
  await page.setViewportSize({ width: 1500, height: 700 });
  await page.goto('/index.html');
  await waitForConverterReady(page);
  const variants = [
    'Midnight Peacock Velvet with Contrast Stitching',
    'Beach', 'Street', 'Forest', 'Desert', 'Arctic', 'Volcano', 'Meadow', 'Harbour',
  ];
  await page.locator('#file-input').setInputFiles({
    name: 'many-variants.gltf',
    mimeType: 'model/gltf+json',
    buffer: Buffer.from(JSON.stringify({
      asset: { version: '2.0' },
      extensionsUsed: ['KHR_materials_variants'],
      extensions: { KHR_materials_variants: { variants: variants.map((name) => ({ name })) } },
      buffers: [{
        byteLength: 36,
        uri: 'data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA',
      }],
      bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }],
      accessors: [{ bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] }],
      materials: [{ name: 'Base' }, { name: 'Alt' }],
      meshes: [{
        primitives: [{
          attributes: { POSITION: 0 },
          material: 0,
          extensions: { KHR_materials_variants: { mappings: [{ material: 1, variants: [0] }] } },
        }],
      }],
      nodes: [{ mesh: 0 }],
      scenes: [{ nodes: [0] }],
      scene: 0,
    })),
  });
  await expect(page.locator('#console')).toContainText('Preview ready');

  await page.locator('#viewer-variant-trigger').click();
  await expect(page.locator('#viewer-variant-menu')).toBeVisible();
  const geometry = await page.evaluate(() => {
    const box = (selector) => {
      const rect = document.querySelector(selector).getBoundingClientRect();
      return { left: Math.round(rect.left), right: Math.round(rect.right), bottom: Math.round(rect.bottom) };
    };
    const menu = document.querySelector('#viewer-variant-menu');
    return {
      trigger: box('#viewer-variant-trigger'),
      menu: box('#viewer-variant-menu'),
      sidebar: box('.sidebar'),
      scrolls: menu.scrollHeight > menu.clientHeight,
    };
  });

  // The row holding the current value is the field seen twice, so with the list
  // open the two are painted the same and stand the same height. They drifted
  // apart in three ways at once: the rows were taller, their text sat further
  // in, and the chosen one wore an accent bar the field had no counterpart for.
  // Hover has to stay distinct from both -- it answers where the pointer is,
  // not what is chosen, and both are on screen together.
  await page.mouse.move(0, 0);
  await page.waitForTimeout(300);
  // Composited, not declared. Comparing the two declarations passed while the
  // field and its chosen row rendered a shade apart, because the same
  // translucent fill sat on different backdrops -- the field on the panel, the
  // row on a menu tinted to look like the field. Only the colour that reaches
  // the screen settles it.
  const paints = await page.evaluate(() => {
    const rendered = (element) => {
      const layers = [];
      for (let node = element; node; node = node.parentElement) {
        const match = getComputedStyle(node).backgroundColor.match(/rgba?\(([^)]+)\)/);
        if (!match) continue;
        const [red, green, blue, alpha = 1] = match[1].split(',').map(Number.parseFloat);
        if (alpha === 0) continue;
        layers.push([red, green, blue, alpha]);
        if (alpha === 1) break;
      }
      let [red, green, blue] = layers.pop() ?? [0, 0, 0];
      while (layers.length > 0) {
        const [r, g, b, a] = layers.pop();
        red = r * a + red * (1 - a);
        green = g * a + green * (1 - a);
        blue = b * a + blue * (1 - a);
      }
      return [red, green, blue].map(Math.round).join(',');
    };
    const of = (element) => ({
      paint: `${rendered(element)}|${getComputedStyle(element).color}`,
      height: Math.round(element.getBoundingClientRect().height),
    });
    return {
      field: of(document.querySelector('#viewer-variant-trigger')),
      selected: of(document.querySelector('.menu-picker-option.selected')),
    };
  });
  expect(paints.selected.paint).toBe(paints.field.paint);
  expect(paints.selected.height).toBe(paints.field.height);

  await page.locator('#viewer-variant-option-1').hover();
  await page.waitForTimeout(300);
  const hovered = await page.evaluate(() => {
    const style = getComputedStyle(document.querySelector('#viewer-variant-option-1'));
    return `${style.backgroundColor}|${style.color}`;
  });
  // Compared against the chosen row's own declaration, since both sit on the
  // same backdrop; what matters here is only that they are not the same fill.
  const selectedDeclared = await page.evaluate(() => {
    const style = getComputedStyle(document.querySelector('.menu-picker-option.selected'));
    return `${style.backgroundColor}|${style.color}`;
  });
  expect(hovered).not.toBe(selectedDeclared);
  // Not lifted off the surface either: a shadow is what makes a list read as a
  // popup that arrived over the page rather than as part of what opened it.
  expect(await page.evaluate(() => getComputedStyle(document.querySelector('#viewer-variant-menu')).boxShadow)).toBe('none');
  await page.mouse.move(0, 0);

  expect(geometry.menu.left).toBe(geometry.trigger.left);
  expect(geometry.menu.right).toBe(geometry.trigger.right);
  expect(geometry.menu.bottom).toBeLessThanOrEqual(geometry.sidebar.bottom);
  // Capped rather than truncated: every entry is still reachable.
  expect(geometry.scrolls).toBe(true);
  expect(await page.locator('#viewer-variant-menu .menu-picker-option').count()).toBe(9);
});

/**
 * Two pickers, one control.
 *
 * They sit on different surfaces, so each names its own colours and its own
 * border -- but everything that makes them the same control has to come out
 * the same, and it kept not doing. Each surface had been sizing its own field:
 * 28px tall with 14px of padding over the viewport, 27px with 9px in the
 * sidebar, at two different font sizes. The list takes its metrics from the
 * field it opens under, so the two lists came out different too, which is
 * exactly how it looked.
 *
 * One fixture carrying both a variant list and a clip list, because a gate that
 * measures one picker in one test and the other in another cannot see them
 * drift apart.
 */
test('the two pickers are the same control', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles({
    name: 'both.gltf',
    mimeType: 'model/gltf+json',
    buffer: Buffer.from(JSON.stringify({
      asset: { version: '2.0' },
      extensionsUsed: ['KHR_materials_variants'],
      extensions: { KHR_materials_variants: { variants: [{ name: 'Ruby' }, { name: 'Emerald' }] } },
      buffers: [{
        byteLength: 76,
        uri: 'data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAAAAAACAPw==',
      }],
      bufferViews: [
        { buffer: 0, byteOffset: 0, byteLength: 36 },
        { buffer: 0, byteOffset: 36, byteLength: 8 },
        { buffer: 0, byteOffset: 44, byteLength: 32 },
      ],
      accessors: [
        { bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [0, 0, 0], max: [1, 1, 0] },
        { bufferView: 1, componentType: 5126, count: 2, type: 'SCALAR', min: [0], max: [1] },
        { bufferView: 2, componentType: 5126, count: 2, type: 'VEC4' },
      ],
      materials: [{ name: 'Base' }, { name: 'Alt' }],
      meshes: [{
        primitives: [{
          attributes: { POSITION: 0 },
          material: 0,
          extensions: { KHR_materials_variants: { mappings: [{ material: 1, variants: [0] }] } },
        }],
      }],
      nodes: [{ mesh: 0 }],
      // Two clips, so the clip list has an unchosen row to compare as well.
      animations: [{
        name: 'Spin',
        samplers: [{ input: 1, output: 2, interpolation: 'LINEAR' }],
        channels: [{ sampler: 0, target: { node: 0, path: 'rotation' } }],
      }, {
        name: 'Rest',
        samplers: [{ input: 1, output: 2, interpolation: 'LINEAR' }],
        channels: [{ sampler: 0, target: { node: 0, path: 'rotation' } }],
      }],
      scenes: [{ nodes: [0] }],
      scene: 0,
    })),
  });
  await expect(page.locator('#console')).toContainText('Preview ready');

  const shapeOf = (field, menu) => page.evaluate(([fieldId, menuId]) => {
    // Everything that makes it look like itself, not only its size: the two
    // used to agree on metrics and still differ by a border, a corner radius
    // and a fill, because each surface dressed its own.
    const measure = (element) => {
      const style = getComputedStyle(element);
      return {
        // What a row and the field it stands for must share.
        text: [
          Math.round(element.getBoundingClientRect().height),
          style.padding,
          style.fontSize,
          style.fontWeight,
          style.color,
          style.backgroundColor,
        ].join('|'),
        // What only the two pickers must share: a row has no border of its own
        // and no corners, because the list is what carries those.
        frame: [style.border, style.borderRadius, style.boxShadow].join('|'),
      };
    };
    return {
      field: measure(document.querySelector(fieldId)),
      row: measure(document.querySelector(`${menuId} .menu-picker-option:not(.selected)`)),
      chosen: measure(document.querySelector(`${menuId} .menu-picker-option.selected`)),
    };
  }, [field, menu]);

  // Settled before each reading: the field's colours are transitioned, so a
  // measurement taken as the list opens catches it part way there.
  await page.locator('#viewer-variant-trigger').click();
  await page.waitForTimeout(300);
  const variant = await shapeOf('#viewer-variant-trigger', '#viewer-variant-menu');
  await page.keyboard.press('Escape');
  await page.locator('#anim-clip-trigger').click();
  await page.waitForTimeout(300);
  const clip = await shapeOf('#anim-clip-trigger', '#anim-clip-menu');
  await page.keyboard.press('Escape');

  expect(clip).toEqual(variant);
  // Including the list itself, which is where the difference showed.
  const menus = await page.evaluate(() => {
    const paint = (selector) => {
      const style = getComputedStyle(document.querySelector(selector));
      return [style.backgroundColor, style.border, style.borderRadius, style.boxShadow].join('|');
    };
    return { variant: paint('#viewer-variant-menu'), clip: paint('#anim-clip-menu') };
  });
  expect(menus.clip).toBe(menus.variant);
  // And within each, the chosen row is shaped like the field it stands for
  // while an unchosen one only differs in weight.
  expect(variant.chosen.text).toBe(variant.field.text);
  expect(variant.row.text).not.toBe(variant.chosen.text);
});

test('EXT_mesh_gpu_instancing draws the mesh once per instance transform', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);
  // The fixture is authored rather than borrowed - no asset in the corpus uses
  // this extension - and it places four copies of one quad in a row. What the
  // gate asks is whether the copies are where their transforms put them: a
  // renderer that ignored the instancing would draw one quad at the centre and
  // leave the outer positions empty.
  await page.locator('#file-input').setInputFiles(
    path.join(repoRoot, 'testdata', 'InstancedQuads.gltf'),
  );
  await expect(page.locator('#console')).toContainText('Preview ready');

  const samples = await page.evaluate(async () => {
    const { state } = await import('/app/state.js');
    const viewer = state.viewer;
    viewer.showGrid = false;
    viewer.camera.target.set([0, 0, 0]);
    viewer.camera.distance = 9;
    viewer.camera.azimuth = 0;
    viewer.camera.elevation = 0;
    viewer._render();
    const width = viewer.gl.drawingBufferWidth;
    const height = viewer.gl.drawingBufferHeight;
    const row = new Uint8Array(width * 4);
    viewer.gl.readPixels(0, Math.floor(height / 2), width, 1, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, row);
    // The quads emit green, so a covered column is one where green leads.
    const covered = [];
    let run = null;
    for (let x = 0; x < width; x += 1) {
      const green = row[x * 4 + 1] > row[x * 4] + 20;
      if (green && !run) run = { start: x };
      if (!green && run) {
        covered.push({ ...run, end: x });
        run = null;
      }
    }
    if (run) covered.push({ ...run, end: width });
    return {
      count: state.currentSceneDocument.nodes[0].instancing?.count,
      instancing: state.viewer.scene.nodes[0].instancing?.count,
      bands: covered.map((band) => band.end - band.start),
      centres: covered.map((band) => (band.start + band.end) / 2),
      width,
    };
  });

  expect(samples.count).toBe(4);
  expect(samples.instancing).toBe(4);
  // Four separated copies across the row, evenly spaced: the fixture puts them
  // 1.5 apart, so what the frame must show is four bands whose centres are as
  // far from each other as the transforms say. A renderer that ignored the
  // instancing would draw one band in the middle.
  expect(samples.bands).toHaveLength(4);
  const gaps = samples.centres.slice(1).map((centre, index) => centre - samples.centres[index]);
  for (const gap of gaps) expect(Math.abs(gap - gaps[0])).toBeLessThan(gaps[0] * 0.15);
  // And centred on the node, which is where the copies were placed around.
  expect(Math.abs((samples.centres[0] + samples.centres[3]) / 2 - samples.width / 2)).toBeLessThan(4);
});

test('KHR_lights_punctual lights the scene from the node that places it', async ({ page }) => {
  await page.goto('/index.html');
  // A light is the first thing the portable document carries that is neither
  // geometry nor material, and the node it hangs on is what gives it a place.
  // So the fixture moves the node rather than the light: if the renderer baked
  // the position once, the second frame would look like the first.
  const observed = await page.evaluate(async () => {
    const [{ Viewer }, { createSceneDocument }, { buildViewerSceneFromDocument }] = await Promise.all([
      import('/viewer.js'),
      import('/scene-document.js'),
      import('/scene-document-viewer.js'),
    ]);

    const bytes = (values) => new Uint8Array(
      values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength),
    );
    const accessor = (values, components, componentType = 5126) => ({
      bytes: bytes(values), componentType, components, count: values.length / components,
    });
    // A white rough quad facing the camera, lit by nothing but the light.
    const document_ = (lights, lightNode) => createSceneDocument({
      ...(lights ? { lights } : {}),
      materials: [{ baseColorFactor: [1, 1, 1, 1], metallicFactor: 0, roughnessFactor: 0.9 }],
      accessors: [
        accessor(new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]), 3),
        accessor(new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]), 3),
        accessor(new Uint16Array([0, 1, 2, 0, 2, 3]), 1, 5123),
      ],
      meshes: [{ primitives: [{ attributes: { POSITION: 0, NORMAL: 1 }, indices: 2, material: 0 }] }],
      nodes: [
        { name: 'Quad', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0 },
        ...(lightNode ? [lightNode] : []),
      ],
      rootNodes: lightNode ? [0, 1] : [0],
    });

    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    viewer.showGrid = false;
    const sample = (lights, lightNode) => {
      viewer.setScene(buildViewerSceneFromDocument(document_(lights, lightNode)));
      viewer.camera.target.set([0, 0, 0]);
      viewer.camera.distance = 3;
      viewer.camera.azimuth = 0;
      viewer.camera.elevation = 0;
      viewer._render();
      const rgba = new Uint8Array(4);
      viewer.gl.readPixels(
        Math.floor(viewer.gl.drawingBufferWidth / 2),
        Math.floor(viewer.gl.drawingBufferHeight / 2),
        1, 1, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, rgba,
      );
      return Array.from(rgba);
    };

    const placed = (translation) => ({
      name: 'Lamp', translation, rotation: [0, 0, 0, 1], scale: [1, 1, 1], light: 0,
    });
    const unlit = sample(null, null);
    const near = sample([{ type: 'point', color: [1, 0.2, 0.2], intensity: 12 }], placed([0, 0, 1]));
    const far = sample([{ type: 'point', color: [1, 0.2, 0.2], intensity: 12 }], placed([0, 0, 6]));
    // A spot aimed away from the quad reaches none of it: -Z is a light's
    // forward, so a lamp at +Z pointing at +Z looks the other way.
    const aimedAway = sample(
      [{ type: 'spot', color: [1, 0.2, 0.2], intensity: 12, innerConeAngle: 0, outerConeAngle: 0.3 }],
      { name: 'Lamp', translation: [0, 0, 1], rotation: [0, 1, 0, 0], scale: [1, 1, 1], light: 0 },
    );
    const glError = viewer.gl.getError();
    viewer.dispose();
    canvas.remove();
    return { unlit, near, far, aimedAway, glError };
  });

  expect(observed.glError).toBe(0);
  // The light reaches the surface.
  expect(observed.near[0]).toBeGreaterThan(observed.unlit[0] + 30);
  // And falls off with the square of the distance, so the node's place matters.
  expect(observed.far[0]).toBeLessThan(observed.near[0] - 30);
  // A cone pointed away delivers nothing, which is the node's rotation being read.
  expect(observed.aimedAway[0]).toBeLessThan(observed.unlit[0] + 10);
});

test('KHR_materials_anisotropy stretches the specular lobe along its rotation', async ({ page }) => {
  await page.goto('/index.html');
  // No asset in the corpus carries this one, so the fixture is built here: a
  // smooth metal quad whose lobe is combed in two different directions. If
  // the rotation were ignored, the two would be the same frame.
  const observed = await page.evaluate(async () => {
    const [{ Viewer }, { createSceneDocument }, { buildViewerSceneFromDocument }] = await Promise.all([
      import('/viewer.js'),
      import('/scene-document.js'),
      import('/scene-document-viewer.js'),
    ]);

    const bytes = (values) => new Uint8Array(
      values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength),
    );
    const accessor = (values, components, componentType = 5126) => ({
      bytes: bytes(values), componentType, components, count: values.length / components,
    });
    const document_ = (comb) => createSceneDocument({
      materials: [{ baseColorFactor: [1, 1, 1, 1], metallicFactor: 1, roughnessFactor: 0.15, ...comb }],
      accessors: [
        accessor(new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]), 3),
        accessor(new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]), 3),
        accessor(new Uint16Array([0, 1, 2, 0, 2, 3]), 1, 5123),
      ],
      meshes: [{ primitives: [{ attributes: { POSITION: 0, NORMAL: 1 }, indices: 2, material: 0 }] }],
      nodes: [{ name: 'Quad', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0 }],
      rootNodes: [0],
    });

    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    viewer.showGrid = false;
    const sample = (comb) => {
      viewer.setScene(buildViewerSceneFromDocument(document_(comb)));
      viewer.camera.target.set([0, 0, 0]);
      viewer.camera.distance = 3;
      viewer.camera.azimuth = 0.5;
      viewer.camera.elevation = 0.35;
      viewer._render();
      const rgba = new Uint8Array(4);
      viewer.gl.readPixels(
        Math.floor(viewer.gl.drawingBufferWidth / 2),
        Math.floor(viewer.gl.drawingBufferHeight / 2),
        1, 1, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, rgba,
      );
      return Array.from(rgba);
    };

    const isotropic = sample({});
    const combedAcross = sample({ anisotropyStrength: 0.9, anisotropyRotation: 0 });
    const combedAlong = sample({ anisotropyStrength: 0.9, anisotropyRotation: 1.5708 });
    const glError = viewer.gl.getError();
    viewer.dispose();
    canvas.remove();
    return { isotropic, combedAcross, combedAlong, glError };
  });

  const brightness = (pixel) => pixel[0] + pixel[1] + pixel[2];
  expect(observed.glError).toBe(0);
  // Combing the surface changes what the lobe gathers.
  expect(Math.abs(brightness(observed.combedAcross) - brightness(observed.isotropic))).toBeGreaterThan(6);
  // And which way it is combed matters, which is the rotation being read.
  expect(Math.abs(brightness(observed.combedAcross) - brightness(observed.combedAlong))).toBeGreaterThan(6);
});

test('KHR_materials_transmission shows what is behind the surface', async ({ page }) => {
  await page.goto('/index.html');
  // Transmission is the one extension that cannot be shaded from the material
  // alone: it reads the frame the opaque pass produced. So the fixture puts a
  // coloured opaque quad behind a transmissive one and asks whether its colour
  // arrives - which fails both if the surface does not refract and if it
  // refracts a frame that was captured at the wrong moment.
  const observed = await page.evaluate(async () => {
    const [{ Viewer }, { createSceneDocument }, { buildViewerSceneFromDocument }] = await Promise.all([
      import('/viewer.js'),
      import('/scene-document.js'),
      import('/scene-document-viewer.js'),
    ]);

    const bytes = (values) => new Uint8Array(
      values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength),
    );
    const accessor = (values, components, componentType = 5126) => ({
      bytes: bytes(values), componentType, components, count: values.length / components,
    });
    const document_ = (front, emissiveStrength = 1) => createSceneDocument({
      materials: [
        // Behind: a green emitter, so what shows through is unmistakably it.
        {
          baseColorFactor: [0, 0, 0, 1],
          metallicFactor: 0,
          roughnessFactor: 1,
          emissiveFactor: [0, 1, 0],
          emissiveStrength,
        },
        // In front: white, because the spec's BTDF tints what passes through
        // by the base colour. Black glass transmits nothing, which would make
        // this fixture measure the tint rather than the transmission.
        { baseColorFactor: [1, 1, 1, 1], metallicFactor: 0, roughnessFactor: 0.05, ...front },
      ],
      accessors: [
        // Behind: a green stripe rather than a wall, so a ray that leaves the
        // glass at a different angle can land off it.
        accessor(new Float32Array([-0.45, -2, -1, 0.45, -2, -1, 0.45, 2, -1, -0.45, 2, -1]), 3),
        accessor(new Float32Array([-1, -1, 0.5, 1, -1, 0.5, 1, 1, 0.5, -1, 1, 0.5]), 3),
        accessor(new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]), 3),
        accessor(new Uint16Array([0, 1, 2, 0, 2, 3]), 1, 5123),
      ],
      meshes: [{
        primitives: [
          { attributes: { POSITION: 0, NORMAL: 2 }, indices: 3, material: 0 },
          { attributes: { POSITION: 1, NORMAL: 2 }, indices: 3, material: 1 },
        ],
      }],
      nodes: [{ name: 'Quads', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0 }],
      rootNodes: [0],
    });

    // A viewer of its own per sample: a snapshot survives between frames, so a
    // reused one would let the previous frame stand in for this one's opaque
    // pass - and then drawing the transmissive quad too early would still look
    // right.
    let glError = 0;
    const sample = (front, emissiveStrength) => {
      const canvas = document.createElement('canvas');
      canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
      document.body.appendChild(canvas);
      const viewer = new Viewer(canvas);
      viewer.showGrid = false;
    // These are sixty-four texels square, so the frame can be drawn at twice
    // that and averaged down for nothing. What is measured here is a material,
    // and the finest sampling available is what keeps the margins about the
    // material rather than about how coarsely it was sampled.
    viewer.supersample = true;
      viewer.setScene(buildViewerSceneFromDocument(document_(front, emissiveStrength)));
      viewer.camera.target.set([0, 0, 0]);
      viewer.camera.distance = 4;
      viewer.camera.azimuth = 0.45;
      viewer.camera.elevation = 0;
      viewer._render();
      // A whole scanline rather than one pixel: dispersion moves a channel
      // sideways by a few texels, and which texel it lands on depends on the
      // drawing buffer's size. What the row says - this channel moved, that
      // one did not - does not.
      const width = viewer.gl.drawingBufferWidth;
      const row = new Uint8Array(width * 4);
      viewer.gl.readPixels(
        0, Math.floor(viewer.gl.drawingBufferHeight / 2),
        width, 1, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, row,
      );
      glError = glError || viewer.gl.getError();
      viewer.dispose();
      canvas.remove();
      const centre = (width >> 1) * 4;
      return { centre: Array.from(row.subarray(centre, centre + 4)), row: Array.from(row) };
    };

    const opaque = sample({});
    const clear = sample({ transmissionFactor: 1 });
    // KHR_materials_dispersion: the same glass, same thickness, with the
    // channels pulled apart - so the three rays leave at three angles and land
    // in three places. The control has to carry the thickness too, or what it
    // measures is the refraction offset rather than the dispersion.
    const thick = sample({ transmissionFactor: 1, thicknessFactor: 2 });
    const dispersed = sample({ transmissionFactor: 1, thicknessFactor: 2, dispersion: 4 });
    // The same pair at the index of air, where the surface does not refract.
    const straight = sample({ transmissionFactor: 1, thicknessFactor: 2, ior: 1 });
    const straightDispersed = sample({
      transmissionFactor: 1, thicknessFactor: 2, ior: 1, dispersion: 4,
    });
    // Mirror-smooth glass against glass at the floor the specular lobes need.
    // The lobes see the same roughness either way; the only thing that can
    // tell these two apart is which mip of the frame behind the transmission
    // reads, which is the whole question.
    const smooth_ = sample({ transmissionFactor: 1, roughnessFactor: 0 });
    const atFloor = sample({ transmissionFactor: 1, roughnessFactor: 0.045 });
    // The same glass filled with a green-absorbing medium: what comes through
    // is what the volume left of it.
    const tinted = sample({
      transmissionFactor: 1,
      thicknessFactor: 1,
      attenuationDistance: 0.35,
      attenuationColor: [1, 0.1, 0.1],
    });
    return {
      opaque, clear, tinted, thick, dispersed, straight, straightDispersed,
      smooth: smooth_, atFloor, glError,
    };
  });

  expect(observed.glError).toBe(0);
  // What separates the two is not brightness but colour: the surface is lit by
  // a neutral environment either way, so green standing out from red is the
  // emitter behind arriving and nothing else. Comparing absolute levels would
  // measure how bright the environment is.
  expect(Math.abs(observed.opaque.centre[1] - observed.opaque.centre[0])).toBeLessThan(20);
  expect(observed.clear.centre[1] - observed.clear.centre[0]).toBeGreaterThan(40);
  // With a volume that absorbs green, less of it does.
  expect(observed.tinted.centre[1]).toBeLessThan(observed.clear.centre[1] - 20);

  // What dispersion does is separate the ends of the spectrum: the red end
  // bends least and the blue end most, so where the glass shows an edge the
  // two land in different places and the channels come apart. Measured as the
  // widest gap between them along the row, against the same glass at the same
  // thickness - which is the extension rather than a brighter or dimmer
  // refraction. Not "green stays put": every channel is now built from a dozen
  // wavelengths of its own, and only the whole spectrum has a fixed point.
  const largestShift = (channel, a, b) => a.row.reduce((most, value, index) => (
    index % 4 === channel ? Math.max(most, Math.abs(value - b.row[index])) : most
  ), 0);
  const widestSplit = (sample) => sample.row.reduce((most, value, index) => (
    index % 4 === 0 ? Math.max(most, Math.abs(value - sample.row[index + 2])) : most
  ), 0);
  expect(largestShift(0, observed.dispersed, observed.thick)).toBeGreaterThan(5);
  expect(widestSplit(observed.dispersed)).toBeGreaterThan(widestSplit(observed.thick) * 3);

  // And at the index of air there is no dispersion to have: light that is not
  // bent at all is not bent by wavelength either. This is what a spread stated
  // as a bare multiple of the factor gets wrong - it splits the channels of a
  // surface that does not refract.
  for (const channel of [0, 1, 2]) {
    expect(largestShift(channel, observed.straightDispersed, observed.straight)).toBeLessThan(3);
  }

  // Roughness reaches transmission unclamped. The floor the specular lobes
  // need is a lie about the surface, and spending it on the mip level makes a
  // pane the asset called mirror-smooth read the level below - which is what
  // a filament seen through glass shows as steps. Clamped, these two samples
  // are the same surface and the rows come out identical.
  expect(largestShift(1, observed.smooth, observed.atFloor)).toBeGreaterThan(4);
});

test('KHR_materials_iridescence tints the specular lobe by film thickness', async ({ page }) => {
  await page.goto('/index.html');
  // A thin film reinforces the wavelengths its thickness suits, so the same
  // material at two thicknesses must reflect two different colours. That is
  // what separates the extension from a plain tint: nothing about the material
  // changes except a distance in nanometres.
  const observed = await page.evaluate(async () => {
    const [{ Viewer }, { createSceneDocument }, { buildViewerSceneFromDocument }] = await Promise.all([
      import('/viewer.js'),
      import('/scene-document.js'),
      import('/scene-document-viewer.js'),
    ]);

    const bytes = (values) => new Uint8Array(
      values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength),
    );
    const accessor = (values, components, componentType = 5126) => ({
      bytes: bytes(values), componentType, components, count: values.length / components,
    });
    // A smooth black dielectric: its specular reflectance is the 4% the core
    // model implies, which leaves the film room to change it. A white metal
    // reflects everything already, and interference has nothing to add.
    const document_ = (film) => createSceneDocument({
      materials: [{
        baseColorFactor: [0, 0, 0, 1],
        metallicFactor: 0,
        roughnessFactor: 0.05,
        ...film,
      }],
      accessors: [
        accessor(new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]), 3),
        accessor(new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]), 3),
        accessor(new Uint16Array([0, 1, 2, 0, 2, 3]), 1, 5123),
      ],
      meshes: [{ primitives: [{ attributes: { POSITION: 0, NORMAL: 1 }, indices: 2, material: 0 }] }],
      nodes: [{ name: 'Quad', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0 }],
      rootNodes: [0],
    });

    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    viewer.showGrid = false;
    // These are sixty-four texels square, so the frame can be drawn at twice
    // that and averaged down for nothing. What is measured here is a material,
    // and the finest sampling available is what keeps the margins about the
    // material rather than about how coarsely it was sampled.
    viewer.supersample = true;
    const sample = (film) => {
      viewer.setScene(buildViewerSceneFromDocument(document_(film)));
      viewer.camera.target.set([0, 0, 0]);
      viewer.camera.distance = 3;
      viewer.camera.azimuth = 0.6;
      viewer.camera.elevation = 0.2;
      viewer._render();
      const rgba = new Uint8Array(4);
      viewer.gl.readPixels(
        Math.floor(viewer.gl.drawingBufferWidth / 2),
        Math.floor(viewer.gl.drawingBufferHeight / 2),
        1, 1, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, rgba,
      );
      return Array.from(rgba);
    };

    const plain = sample({});
    const thin = sample({
      iridescenceFactor: 1, iridescenceIor: 1.8,
      iridescenceThicknessMinimum: 180, iridescenceThicknessMaximum: 180,
    });
    const thick = sample({
      iridescenceFactor: 1, iridescenceIor: 1.8,
      iridescenceThicknessMinimum: 520, iridescenceThicknessMaximum: 520,
    });
    const glError = viewer.gl.getError();
    viewer.dispose();
    canvas.remove();
    return { plain, thin, thick, glError };
  });

  const hue = (pixel) => [pixel[0] - pixel[1], pixel[1] - pixel[2]];
  expect(observed.glError).toBe(0);
  // The film colours a surface that was neutral without it.
  expect(Math.abs(hue(observed.thin)[0] - hue(observed.plain)[0])
    + Math.abs(hue(observed.thin)[1] - hue(observed.plain)[1])).toBeGreaterThan(6);
  // And the colour is the thickness talking, not a constant tint.
  expect(Math.abs(hue(observed.thin)[0] - hue(observed.thick)[0])
    + Math.abs(hue(observed.thin)[1] - hue(observed.thick)[1])).toBeGreaterThan(6);
});

test('KHR_materials_sheen shows on the surface it is set on', async ({ page }) => {
  await page.goto('/index.html');
  // Reading an extension and shading it are separate claims, and the gates so
  // far only made the first. This one renders the same quad twice, alike but
  // for the sheen colour, and requires the frame to differ: a table entry with
  // no GLSL behind it passes every other check in the suite.
  const observed = await page.evaluate(async () => {
    const [{ Viewer }, { createSceneDocument }, { buildViewerSceneFromDocument }] = await Promise.all([
      import('/viewer.js'),
      import('/scene-document.js'),
      import('/scene-document-viewer.js'),
    ]);

    const bytes = (values) => new Uint8Array(
      values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength),
    );
    const accessor = (values, components, componentType = 5126) => ({
      bytes: bytes(values), componentType, components, count: values.length / components,
    });
    // Rough and black: everything the frame shows comes from the sheen lobe,
    // and a lit dielectric would drown it.
    const document_ = (sheen) => createSceneDocument({
      materials: [{
        baseColorFactor: [0, 0, 0, 1],
        metallicFactor: 0,
        roughnessFactor: 1,
        ...sheen,
      }],
      accessors: [
        accessor(new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]), 3),
        accessor(new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]), 3),
        accessor(new Uint16Array([0, 1, 2, 0, 2, 3]), 1, 5123),
      ],
      meshes: [{ primitives: [{ attributes: { POSITION: 0, NORMAL: 1 }, indices: 2, material: 0 }] }],
      nodes: [{ name: 'Quad', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0 }],
      rootNodes: [0],
    });

    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    viewer.showGrid = false;
    // Off-axis, because a sheen lobe is retroreflective: head-on it returns
    // the least it ever will.
    const sample = (sheen) => {
      viewer.setScene(buildViewerSceneFromDocument(document_(sheen)));
      viewer.camera.target.set([0, 0, 0]);
      viewer.camera.distance = 3;
      viewer.camera.azimuth = 1.35;
      viewer.camera.elevation = 0.15;
      viewer._render();
      // Read the frame, not the canvas: what the material did is a quantity of
      // light, and the tone curve compresses differences near the top of its
      // range to almost nothing. A threshold on the picture would be a
      // threshold on the display transform.
      const gl = viewer.gl;
      const scene = viewer._sceneTarget;
      const radiance = new Float32Array(4);
      gl.bindFramebuffer(gl.FRAMEBUFFER, scene.resolveFramebuffer);
      gl.readPixels(
        scene.renderWidth >> 1, scene.renderHeight >> 1,
        1, 1, gl.RGBA, gl.FLOAT, radiance,
      );
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      return Array.from(radiance);
    };

    const plain = sample({});
    const sheened = sample({ sheenColorFactor: [1, 0.2, 0.2], sheenRoughnessFactor: 0.4 });
    const rougher = sample({ sheenColorFactor: [1, 0.2, 0.2], sheenRoughnessFactor: 1 });
    const glError = viewer.gl.getError();
    viewer.dispose();
    canvas.remove();
    return { plain, sheened, rougher, glError };
  });

  expect(observed.glError).toBe(0);
  // The sheen is red, and it is the only thing on a black rough surface that
  // could be. Measured as light, so the margins are fractions of the surface
  // rather than of whatever the output pass does with it.
  expect(observed.sheened[0]).toBeGreaterThan(observed.plain[0] * 1.3);
  expect(observed.sheened[0]).toBeGreaterThan(observed.sheened[1] * 1.3);
  // Roughness drives the lobe, so it is read rather than defaulted.
  expect(Math.abs(observed.sheened[0] - observed.rougher[0]))
    .toBeGreaterThan(observed.sheened[0] * 0.05);
});

test('the frame stays light until the output pass, which spreads glare and maps tones', async ({ page }) => {
  await page.goto('/index.html');
  // Everything is drawn into one linear frame and turned into a picture once,
  // at the end. That is what makes every average in between mean something:
  // the multisample resolve, the mip chain a rough refraction reads, the glare
  // pyramid. A surface that tone mapped its own output would poison all three,
  // and a transmissive one reading such a frame would refract it twice over.
  const observed = await page.evaluate(async () => {
    const [{ Viewer }, { createSceneDocument }, { buildViewerSceneFromDocument }] = await Promise.all([
      import('/viewer.js'),
      import('/scene-document.js'),
      import('/scene-document-viewer.js'),
    ]);

    const bytes = (values) => new Uint8Array(
      values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength),
    );
    const accessor = (values, components, componentType = 5126) => ({
      bytes: bytes(values), componentType, components, count: values.length / components,
    });
    // A blended quad in front of an opaque one, in the wrong order in the
    // scene: the opaque quad is second. Drawing them as they come would put
    // the blend underneath. The transmissive quad is what makes the copy
    // happen at all; it sits behind the opaque one, so it is depth-rejected
    // and contributes no pixel of its own.
    const materials = [
      { baseColorFactor: [0, 0, 1, 0.5], alphaMode: 'BLEND', emissiveFactor: [0, 0, 1] },
      // Far brighter than white, like a filament: the case the linear frame
      // and the glare pass both exist for.
      { baseColorFactor: [1, 0, 0, 1], emissiveFactor: [1, 0, 0], emissiveStrength: 25 },
      { baseColorFactor: [1, 1, 1, 1], metallicFactor: 0, roughnessFactor: 0, transmissionFactor: 1 },
    ];
    const sceneOf = (count) => createSceneDocument({
      materials: materials.slice(0, count),
      accessors: [
        accessor(new Float32Array([-1, -1, 1, 1, -1, 1, 1, 1, 1, -1, 1, 1]), 3),
        accessor(new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]), 3),
        accessor(new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]), 3),
        accessor(new Uint16Array([0, 1, 2, 0, 2, 3]), 1, 5123),
        accessor(new Float32Array([-1, -1, -1, 1, -1, -1, 1, 1, -1, -1, 1, -1]), 3),
      ],
      meshes: [{
        primitives: [
          { attributes: { POSITION: 0, NORMAL: 2 }, indices: 3, material: 0 },
          { attributes: { POSITION: 1, NORMAL: 2 }, indices: 3, material: 1 },
          { attributes: { POSITION: 4, NORMAL: 2 }, indices: 3, material: 2 },
        ].slice(0, count),
      }],
      // Turned about the view axis, so the edges cross texels at an angle
      // instead of landing on their boundaries. An axis-aligned edge says
      // nothing about whether the frame is antialiased.
      nodes: [{
        name: 'Quads',
        translation: [0, 0, 0],
        rotation: [0, 0, Math.sin(0.175), Math.cos(0.175)],
        scale: [1, 1, 1],
        mesh: 0,
      }],
      rootNodes: [0],
    });

    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    viewer.showGrid = false;
    // These are sixty-four texels square, so the frame can be drawn at twice
    // that and averaged down for nothing. What is measured here is a material,
    // and the finest sampling available is what keeps the margins about the
    // material rather than about how coarsely it was sampled.
    viewer.supersample = true;
    // Glare is measured on its own below; everything in between is compared
    // against the tone curve, which has to be the only thing acting.
    viewer.bloomStrength = 0;
    const frame = (count) => {
      viewer.setScene(buildViewerSceneFromDocument(sceneOf(count)));
      viewer.camera.target.set([0, 0, 0]);
      viewer.camera.distance = 5;
      viewer.camera.azimuth = 0;
      viewer.camera.elevation = 0;
      viewer._render();
    };

    const gl = viewer.gl;
    const readFloat = (framebuffer, x, y) => {
      const pixel = new Float32Array(4);
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
      gl.readPixels(x, y, 1, 1, gl.RGBA, gl.FLOAT, pixel);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      return [...pixel];
    };
    const readCanvas = (x, y) => {
      const pixel = new Uint8Array(4);
      gl.readPixels(x, y, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
      return [...pixel];
    };

    // Nothing transmits, so the copy is never taken and stays as cleared.
    frame(2);
    const scene = viewer._sceneTarget;
    const centre = [scene.renderWidth >> 1, scene.renderHeight >> 1];
    const withoutTransmission = readFloat(scene.captureFramebuffer, centre[0], centre[1]);

    frame(3);
    const capturePixel = readFloat(scene.captureFramebuffer, centre[0], centre[1]);
    // Where the canvas's corner texel sits in a frame drawn wider than it.
    const inFrame = (x, size, renderSize) => Math.round(
      renderSize * (0.5 + ((x + 0.5) / size * 2 - 1) * 0.5 / scene.guard));
    const cornerAt = [
      inFrame(1, gl.drawingBufferWidth, scene.renderWidth),
      inFrame(1, gl.drawingBufferHeight, scene.renderHeight),
    ];
    const captureCorner = readFloat(scene.captureFramebuffer, cornerAt[0], cornerAt[1]);
    const canvasCorner = readCanvas(1, 1);
    const canvasPixel = readCanvas(gl.drawingBufferWidth >> 1, gl.drawingBufferHeight >> 1);
    // A quarter down the canvas, mapped into the wider frame: a row measured a
    // quarter down the *frame* would sit in the guard, where the fixture put
    // no geometry.
    const rowY = inFrame(gl.drawingBufferHeight >> 2, gl.drawingBufferHeight, scene.renderHeight);
    const captureRow = [];
    for (let x = 0; x < scene.renderWidth; x += 1) {
      captureRow.push(readFloat(scene.captureFramebuffer, x, rowY)[0]);
    }
    const readError = gl.getError();

    // Glare, measured where the geometry put none: background beside the
    // emitter, with the pass off and then on.
    const probe = [Math.max(1, (gl.drawingBufferWidth >> 1) - 24), gl.drawingBufferHeight >> 1];
    const withoutGlare = readCanvas(probe[0], probe[1]);
    viewer.bloomStrength = 0.5;
    viewer._render();
    const withGlare = readCanvas(probe[0], probe[1]);
    viewer.bloomStrength = 0;

    // A resize has to reallocate: the attachments are allocated at a fixed
    // size, and a stale one would hold the frame at the wrong scale.
    const before = viewer._sceneTarget;
    canvas.width = gl.drawingBufferWidth + 32;
    canvas.height = gl.drawingBufferHeight + 32;
    viewer._render();
    const reallocated = viewer._sceneTarget !== before
      && viewer._sceneTarget.width === gl.drawingBufferWidth;

    const glError = gl.getError();
    viewer.dispose();
    canvas.remove();
    return {
      withoutTransmission, capturePixel, captureCorner, captureRow, canvasCorner, canvasPixel,
      withoutGlare, withGlare, reallocated, glError, readError,
      hdr: scene.hdr, samples: scene.samples, scale: scene.scale, guard: scene.guard,
      size: [scene.width, scene.height],
      renderSize: [scene.renderWidth, scene.renderHeight],
    };
  });

  expect(observed.glError).toBe(0);
  expect(observed.readError).toBe(0);
  expect(observed.reallocated).toBe(true);
  expect(observed.hdr).toBe(true);
  // Wider than it is shown, by the guard the refracted rays are drawn for.
  expect(observed.guard).toBeGreaterThanOrEqual(1);
  expect(observed.renderSize).toEqual(
    observed.size.map((side) => Math.round(side * observed.scale * observed.guard)));

  // A scene with nothing to refract never takes the copy.
  expect(observed.withoutTransmission.slice(0, 3)).toEqual([0, 0, 0]);

  // The copy holds the opaque quad and nothing else - red, no blue - at its
  // own brightness. Twenty-five times white has to survive as twenty-five.
  expect(observed.capturePixel[0]).toBeGreaterThan(10);
  expect(observed.capturePixel[2]).toBeLessThan(observed.capturePixel[0] / 2);
  // The canvas holds the blend over it, so blue has arrived by then.
  expect(observed.canvasPixel[2]).toBeGreaterThan(100);

  // Coverage is sampled by the samples and the scale together: either alone
  // leaves a staircase on anything thin and much brighter than white.
  expect(observed.samples * observed.scale * observed.scale).toBeGreaterThanOrEqual(8);
  const full = observed.capturePixel[0];
  const partlyCovered = observed.captureRow.filter((red) => red > 0.05 * full && red < 0.8 * full);
  expect(partlyCovered.length).toBeGreaterThan(0);

  // The frame is light, and the output pass is what makes it a picture: tone
  // mapped and encoded, the corner of the frame is the corner of the canvas.
  // Equal without the curve would mean something had mapped tones too early.
  const toneCurve = (x) => {
    const value = (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14);
    return Math.min(Math.max(value, 0), 1);
  };
  const toneMap = (rgb) => {
    const level = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    if (level <= 0) return [0, 0, 0];
    const mapped = toneCurve(level);
    const scaled = rgb.map((channel) => channel * (mapped / level));
    const overflow = Math.min(Math.max(Math.max.apply(null, scaled) - 1, 0), 1);
    return scaled.map((channel) => Math.min(Math.max(
      channel * (1 - overflow) + mapped * overflow, 0), 1));
  };
  const asPicture = toneMap(observed.captureCorner.slice(0, 3))
    .map((channel) => Math.round(255 * channel ** (1 / 2.2)));
  for (let channel = 0; channel < 3; channel += 1) {
    expect(Math.abs(asPicture[channel] - observed.canvasCorner[channel])).toBeLessThanOrEqual(3);
    expect(Math.abs(observed.captureCorner[channel] * 255 - observed.canvasCorner[channel]))
      .toBeGreaterThan(2);
  }

  // And the glare pass spreads light where the geometry put none: background
  // beside an emitter twenty-five times brighter than white is measurably
  // lighter with the pyramid than without it.
  expect(observed.withGlare[0]).toBeGreaterThan(observed.withoutGlare[0] + 4);
});

test('every material extension factor the table declares reaches the shader', async ({ page }) => {
  await page.goto('/index.html');
  // The uniform name follows from the property name, so declaring a field in
  // the extension table is all it takes for the renderer to send it. The half
  // that cannot be derived is the GLSL that reads it, and a field with no
  // uniform behind it fails silently: setting a null location is a no-op, and
  // the shader goes on using whatever the core model implies.
  const observed = await page.evaluate(async () => {
    const [{ Viewer }, { MATERIAL_EXTENSION_UNIFORMS }] = await Promise.all([
      import('/viewer.js'),
      import('/material-extensions.js'),
    ]);
    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:8px;height:8px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    // The barest program there is: if a factor survives here, it survives in
    // every richer permutation too.
    const surface = viewer.surfacePrograms.get([]);
    const missing = MATERIAL_EXTENSION_UNIFORMS
      .filter(({ uniform }) => surface.uniforms[uniform] == null)
      .map(({ property, uniform }) => `${property} -> ${uniform}`);
    const declared = MATERIAL_EXTENSION_UNIFORMS.map(({ uniform }) => uniform);
    viewer.dispose();
    canvas.remove();
    return { missing, declared };
  });

  expect(observed.missing).toEqual([]);
  // The list is derived, so a table that lost its factors would pass the check
  // above by having nothing to check.
  expect(observed.declared).toEqual(expect.arrayContaining([
    'uIor', 'uSpecularFactor', 'uSpecularColorFactor', 'uClearcoatFactor', 'uClearcoatRoughnessFactor',
  ]));
});

test('a morphed mesh keeps its material textures', async ({ page }) => {
  await page.goto('/index.html');
  // The morph deltas are a sampler2DArray and the material maps are sampler2D,
  // and GL forbids one texture unit being addressed as both in a single draw.
  // Nothing in the code said which units were free, so the morph array and the
  // KHR_materials_specular slot both took unit 9 — legal to bind, invalid to
  // draw with. A mesh that morphs *and* carries a specular map is the only
  // arrangement where the two meet.
  const observed = await page.evaluate(async (pngBytes) => {
    const [{ Viewer }, { createSceneDocument }, { buildViewerSceneFromDocument }, { hydrateSceneTextures }] =
      await Promise.all([
        import('/viewer.js'),
        import('/scene-document.js'),
        import('/scene-document-viewer.js'),
        import('/scene-document-textures.js'),
      ]);

    const bytes = (values) => new Uint8Array(
      values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength),
    );
    const accessor = (values, components, componentType = 5126) => ({
      bytes: bytes(values), componentType, components, count: values.length / components,
    });

    const document_ = createSceneDocument({
      resources: [{ mimeType: 'image/png', bytes: new Uint8Array(pngBytes), name: 'specular.png' }],
      textures: [{ resource: 0, sampler: {} }],
      materials: [{
        baseColorFactor: [0, 0, 0, 1],
        metallicFactor: 0,
        roughnessFactor: 1,
        // Emissive reads back as an exact colour whatever the environment does,
        // so "the draw happened" and "the draw was skipped" are two numbers
        // rather than two shades.
        emissiveFactor: [1, 0, 0],
        specularFactor: 0.5,
        specularTexture: { texture: 0 },
      }],
      accessors: [
        // At rest the quad sits behind the camera target's left edge; the morph
        // target is what brings it over the centre, so a draw that never ran
        // leaves the background there.
        accessor(new Float32Array([-3, -1, 0, -1, -1, 0, -1, 1, 0, -3, 1, 0]), 3),
        accessor(new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]), 3),
        accessor(new Float32Array([2, 0, 0, 2, 0, 0, 2, 0, 0, 2, 0, 0]), 3),
        accessor(new Uint16Array([0, 1, 2, 0, 2, 3]), 1, 5123),
      ],
      meshes: [{
        weights: [1],
        primitives: [{
          attributes: { POSITION: 0, NORMAL: 1 },
          targets: [{ POSITION: 2 }],
          indices: 3,
          material: 0,
        }],
      }],
      nodes: [{ name: 'Quad', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0, weights: [1] }],
      rootNodes: [0],
    });

    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);
    viewer.setScene(await hydrateSceneTextures(buildViewerSceneFromDocument(document_)));
    viewer.showGrid = false;
    viewer.camera.target.set([0, 0, 0]);
    viewer.camera.distance = 4;
    viewer.camera.azimuth = 0;
    viewer.camera.elevation = 0;
    viewer._render();
    const rgba = new Uint8Array(4);
    viewer.gl.readPixels(32, 32, 1, 1, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, rgba);
    const glError = viewer.gl.getError();
    viewer.dispose();
    canvas.remove();
    return { pixel: Array.from(rgba), glError };
  }, Array.from(solidColorPng()));

  expect(observed.glError).toBe(0);
  // The morphed quad covers the centre and emits red. Red *dominant* rather
  // than green *absent*: a colour bright enough to leave the display gamut
  // walks toward white on the way out, so the other channels are not zero and
  // a threshold on them would be a threshold on the tone curve.
  expect(observed.pixel[0]).toBeGreaterThan(200);
  expect(observed.pixel[0] - observed.pixel[1]).toBeGreaterThan(60);
  expect(observed.pixel[1] - observed.pixel[2]).toBeLessThan(30);
});

test('a SceneDocument texture only reaches the GPU once it is hydrated', async ({ page }) => {
  await page.goto('/index.html');
  // The document carries texture bytes and no decoded image, because the
  // adapter that builds the scene from it is deliberately DOM-free. Rendered as
  // it arrives, every texture is the opaque white texel uploadImage starts with;
  // the emissive slot is used here because it is additive and independent of the
  // environment, so "white placeholder" and "green image" are exact colours
  // rather than two shades of lit surface.
  const samples = await page.evaluate(async (pngBytes) => {
    const [{ Viewer }, { createSceneDocument }, { buildViewerSceneFromDocument }, { hydrateSceneTextures }] =
      await Promise.all([
        import('/viewer.js'),
        import('/scene-document.js'),
        import('/scene-document-viewer.js'),
        import('/scene-document-textures.js'),
      ]);

    const bytes = (values) => new Uint8Array(
      values.buffer.slice(values.byteOffset, values.byteOffset + values.byteLength),
    );
    const accessor = (values, components, componentType = 5126) => ({
      bytes: bytes(values), componentType, components, count: values.length / components,
    });

    const document_ = createSceneDocument({
      resources: [{ mimeType: 'image/png', bytes: new Uint8Array(pngBytes), name: 'emissive.png' }],
      textures: [{ resource: 0, sampler: { wrapS: 33071, wrapT: 33071, minFilter: 9728, magFilter: 9728 } }],
      materials: [{
        baseColorFactor: [0, 0, 0, 1],
        metallicFactor: 0,
        roughnessFactor: 1,
        emissiveFactor: [1, 1, 1],
        emissiveTexture: { texture: 0 },
      }],
      accessors: [
        accessor(new Float32Array([-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0]), 3),
        accessor(new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1]), 3),
        accessor(new Float32Array([0, 1, 1, 1, 1, 0, 0, 0]), 2),
        accessor(new Uint16Array([0, 1, 2, 0, 2, 3]), 1, 5123),
      ],
      meshes: [{
        primitives: [{
          attributes: { POSITION: 0, NORMAL: 1, TEXCOORD_0: 2 },
          indices: 3,
          material: 0,
        }],
      }],
      nodes: [{ name: 'Quad', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0 }],
      rootNodes: [0],
    });

    const canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:fixed;left:-100px;top:0;width:64px;height:64px';
    document.body.appendChild(canvas);
    const viewer = new Viewer(canvas);

    const render = (scene) => {
      viewer.setScene(scene);
      viewer.showGrid = false;
      viewer.camera.target.set([0, 0, 0]);
      viewer.camera.distance = 3;
      viewer.camera.azimuth = 0;
      viewer.camera.elevation = 0;
      viewer._render();
      const rgba = new Uint8Array(4);
      viewer.gl.readPixels(32, 32, 1, 1, viewer.gl.RGBA, viewer.gl.UNSIGNED_BYTE, rgba);
      return Array.from(rgba);
    };

    const dry = render(buildViewerSceneFromDocument(document_));
    const hydratedScene = await hydrateSceneTextures(buildViewerSceneFromDocument(document_));
    const wet = render(hydratedScene);
    const glError = viewer.gl.getError();
    viewer.dispose();
    canvas.remove();
    return { dry, wet, glError, warnings: hydratedScene.warnings, image: !!hydratedScene.textures[0].image };
  }, Array.from(solidColorPng()));

  expect(samples.glError).toBe(0);
  expect(samples.image).toBe(true);
  expect(samples.warnings).not.toContainEqual(expect.stringContaining('Failed to decode texture'));

  // Unhydrated: the white placeholder emits equally on every channel.
  expect(samples.dry[0]).toBeGreaterThan(200);
  expect(Math.abs(samples.dry[0] - samples.dry[1])).toBeLessThan(8);

  // Hydrated: only the green channel is emitted, so red collapses.
  expect(samples.wet[1]).toBeGreaterThan(200);
  expect(samples.wet[0]).toBeLessThan(samples.wet[1] / 2);
});

test('textures reading one image are hydrated into one bitmap', async ({ page }) => {
  await page.goto('/index.html');
  // A glTF routinely names the same image from many textures — the same map
  // through different sampler settings, or simply the same map on many
  // materials. The document records that as several textures over one resource,
  // and hydration has to preserve it: the viewer keys its GPU uploads on bitmap
  // identity, so a bitmap per texture means an upload per texture. On a real
  // asset (24 textures over one 4096² JPEG) that was ~1.7 GB of texture memory
  // and roughly a second of upload for one image's worth of pixels.
  const observed = await page.evaluate(async (pngBytes) => {
    const [{ createSceneDocument }, { buildViewerSceneFromDocument }, { hydrateSceneTextures }] =
      await Promise.all([
        import('/scene-document.js'),
        import('/scene-document-viewer.js'),
        import('/scene-document-textures.js'),
      ]);

    const sampler = (wrap) => ({ wrapS: wrap, wrapT: wrap, minFilter: 9728, magFilter: 9728 });
    const document_ = createSceneDocument({
      resources: [
        { mimeType: 'image/png', bytes: new Uint8Array(pngBytes), name: 'shared.png' },
        { mimeType: 'image/png', bytes: new Uint8Array(pngBytes), name: 'other.png' },
      ],
      // Three textures, two resources: the first two differ only in how they
      // are sampled, which is a GPU parameter and not a reason to decode twice.
      textures: [
        { resource: 0, sampler: sampler(33071) },
        { resource: 0, sampler: sampler(10497) },
        { resource: 1, sampler: sampler(33071) },
      ],
      materials: [{ baseColorFactor: [1, 1, 1, 1], baseColorTexture: { texture: 0 } }],
      accessors: [{
        bytes: new Uint8Array(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]).buffer),
        componentType: 5126,
        components: 3,
        count: 3,
      }],
      meshes: [{ primitives: [{ attributes: { POSITION: 0 }, material: 0 }] }],
      nodes: [{ name: 'Tri', translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1], mesh: 0 }],
      rootNodes: [0],
    });

    const scene = buildViewerSceneFromDocument(document_);
    // The bytes travel as the document holds them; a copy per texture is the
    // same duplication one step earlier.
    const shareBytes = scene.textures.every((texture, index) => (
      texture.bytes === document_.resources[document_.textures[index].resource].bytes
    ));
    await hydrateSceneTextures(scene);
    return {
      shareBytes,
      images: scene.textures.map((texture) => Boolean(texture.image)),
      distinctImages: new Set(scene.textures.map((texture) => texture.image)).size,
      sharedPair: scene.textures[0].image === scene.textures[1].image,
      separatePair: scene.textures[0].image === scene.textures[2].image,
      warnings: scene.warnings,
    };
  }, Array.from(solidColorPng()));

  expect(observed.warnings).not.toContainEqual(expect.stringContaining('Failed to decode texture'));
  expect(observed.images).toEqual([true, true, true]);
  expect(observed.shareBytes).toBe(true);
  // Two resources, so two decodes — not three.
  expect(observed.distinctImages).toBe(2);
  expect(observed.sharedPair).toBe(true);
  expect(observed.separatePair).toBe(false);
});

test('the glTF preview comes from the scene document the export reads', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);

  // The document is built when the file is opened and drives the summary and
  // every export; before this it was built and then ignored, while the preview
  // opened the same bytes with a second reader. A textured fixture is used
  // because texture bytes only reach the GPU through the hydration step that
  // exists solely for this path — a white frame would mean the switch happened
  // without it.
  await page.locator('#file-input').setInputFiles({
    name: 'emissive.gltf',
    mimeType: 'model/gltf+json',
    buffer: Buffer.from(emissiveTransformQuad({ offset: 0 })),
  });
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('previewing through the direct glTF reader');

  const source = await page.evaluate(async () => {
    const { state } = await import('/app/state.js');
    const scene = state.viewer.scene;
    return {
      document: !!state.currentSceneDocument,
      textures: scene.textures.length,
      // Only the document adapter hands over the source bytes; the direct
      // reader arrives with a decoded image and no bytes at all.
      carriesBytes: scene.textures.every((texture) => texture.bytes instanceof Uint8Array),
      hydrated: scene.textures.every((texture) => !!texture.image),
    };
  });
  expect(source.document).toBe(true);
  expect(source.textures).toBeGreaterThan(0);
  expect(source.carriesBytes).toBe(true);
  expect(source.hydrated).toBe(true);
});

test('a file the document cannot be built from still previews, and says so', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);

  await page.locator('#file-input').setInputFiles({
    name: 'emissive.gltf',
    mimeType: 'model/gltf+json',
    buffer: Buffer.from(emissiveTransformQuad({ offset: 0 })),
  });
  await expect(page.locator('#console')).toContainText('Preview ready');

  // Every glTF in testdata survives the document route, so there is no fixture
  // that fails it — that is a measured fact, not an assumption. The branch is
  // therefore driven directly: with no document in hand the preview has to
  // fall back to the reader that used to be the only path, and say why.
  const fallback = await page.evaluate(async () => {
    const { state } = await import('/app/state.js');
    const { loadPreview } = await import('/app/preview.js');
    state.currentSceneDocument = null;
    await loadPreview('gltf');
    const scene = state.viewer.scene;
    return {
      textures: scene.textures.length,
      // The direct reader decodes during the load and never carries bytes.
      hydrated: scene.textures.every((texture) => !!texture.image),
      carriesBytes: scene.textures.some((texture) => texture.bytes instanceof Uint8Array),
    };
  });

  await expect(page.locator('#console')).toContainText('previewing through the direct glTF reader');
  await expect(page.locator('#console')).toContainText('Preview ready');
  expect(fallback.textures).toBeGreaterThan(0);
  expect(fallback.hydrated).toBe(true);
  expect(fallback.carriesBytes).toBe(false);
});

test('a KTX2 texture reaches the GPU without being decoded to pixels', async ({ page }) => {
  // What this proves depends on the machine, and it says which: a context that
  // offers a block format has to take the compressed upload, and one that does
  // not has to fall back to pixels rather than to the white placeholder. The
  // failure this guards against is the third outcome, where the texture is
  // read, transcoded, and then quietly never uploaded at all.
  const ktx2 = Array.from(await readFile(path.join(repoRoot, 'testdata', 'ktx2', '2d_etc1s.ktx2')));
  const glb = Array.from(basisTexturedGlb(Uint8Array.from(ktx2)));

  await page.addInitScript(() => {
    window.__textureUploads = { compressed: [], plain: 0 };
    const compressed = WebGL2RenderingContext.prototype.compressedTexImage2D;
    WebGL2RenderingContext.prototype.compressedTexImage2D = function (target, level, format, width, height, border, data) {
      window.__textureUploads.compressed.push({ level, format, width, height, bytes: data ? data.byteLength : 0 });
      return compressed.apply(this, arguments);
    };
    const plain = WebGL2RenderingContext.prototype.texImage2D;
    WebGL2RenderingContext.prototype.texImage2D = function (target, level, internalFormat, ...rest) {
      // The one-texel white placeholder every texture starts with is not an
      // upload of anything; counting it would hide the case being tested.
      const source = rest[rest.length - 1];
      if (source instanceof ImageBitmap || source instanceof HTMLImageElement) window.__textureUploads.plain++;
      return plain.apply(this, arguments);
    };
  });

  await page.goto('/index.html');
  await waitForConverterReady(page);
  await page.locator('#file-input').setInputFiles({
    name: 'basis.glb',
    mimeType: 'model/gltf-binary',
    buffer: Buffer.from(glb),
  });
  await expect(page.locator('#console')).toContainText('Preview ready');
  await expect(page.locator('#console')).not.toContainText('requires a transcoder');
  await expect(page.locator('#console')).not.toContainText('could not be decoded');

  const uploads = await page.evaluate(() => window.__textureUploads);
  const supportsBlockFormat = await page.evaluate(() => {
    const gl = document.createElement('canvas').getContext('webgl2');
    return Boolean(gl && gl.getExtension('WEBGL_compressed_texture_s3tc'));
  });

  if (supportsBlockFormat) {
    // Every level, not just the base: a compressed texture cannot have its
    // mips generated, so an incomplete chain samples as black.
    expect(uploads.compressed.length).toBe(10);
    expect(uploads.compressed[0]).toMatchObject({ level: 0, width: 512, height: 512, bytes: 131072 });
    expect(uploads.compressed.at(-1)).toMatchObject({ level: 9, width: 1, height: 1 });
    console.log(`ktx2 upload: ${uploads.compressed.length} BC1 levels`);
  } else {
    expect(uploads.compressed.length).toBe(0);
    expect(uploads.plain).toBeGreaterThan(0);
    console.log('ktx2 upload: no block format on this context, uploaded as pixels');
  }

  // And it has to be on screen, not merely uploaded. The placeholder is
  // opaque white, so a textured quad filling the viewport cannot be white.
  //
  // Sampled from inside the draw rather than from a screenshot, because the
  // context does not preserve its drawing buffer: anything looking at it
  // between frames sees it already cleared. The viewer only redraws when
  // something marks it dirty, so the frame has to be provoked - resizing the
  // viewport is the least invasive way to ask for one.
  await page.evaluate(() => {
    window.__frameSamples = [];
    window.__frameOriginals = {};
    for (const name of ['drawElements', 'drawElementsInstanced', 'drawArrays', 'drawArraysInstanced']) {
      const original = WebGL2RenderingContext.prototype[name];
      window.__frameOriginals[name] = original;
      WebGL2RenderingContext.prototype[name] = function (...args) {
        const result = original.apply(this, args);
        if (window.__frameSamples.length < 64) {
          const pixel = new Uint8Array(4);
          this.readPixels(
            Math.floor(this.drawingBufferWidth / 2),
            Math.floor(this.drawingBufferHeight / 2),
            1, 1, this.RGBA, this.UNSIGNED_BYTE, pixel,
          );
          window.__frameSamples.push([...pixel]);
        }
        return result;
      };
    }
  });
  await page.setViewportSize({ width: 1180, height: 820 });
  await expect.poll(() => page.evaluate(() => window.__frameSamples.length)).toBeGreaterThan(0);
  const drawn = await page.evaluate(() => {
    for (const [name, original] of Object.entries(window.__frameOriginals)) {
      WebGL2RenderingContext.prototype[name] = original;
    }
    return window.__frameSamples;
  });

  expect(drawn.some((pixel) => pixel[0] !== 255 || pixel[1] !== 255 || pixel[2] !== 255)).toBe(true);
});

/**
 * A dropped folder, which is the only way to open a model whose companions sit
 * in a sibling directory.
 *
 * Built in the page rather than on disk: `setInputFiles` cannot express a
 * folder — every file arrives with a bare name — and the whole point here is
 * the path. The tree below is the shape that sent us looking, three.js's
 * DamagedHelmet: two models in two folders, the second reaching back through
 * `../` for the buffer the first one owns.
 */
test('a dropped folder offers its models and resolves a sibling directory', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);

  const observed = await page.evaluate(async () => {
    const positions = new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]);
    const encoder = new TextEncoder();
    const gltf = (uri) => encoder.encode(JSON.stringify({
      asset: { version: '2.0' },
      scene: 0,
      scenes: [{ nodes: [0] }],
      nodes: [{ mesh: 0 }],
      meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }],
      buffers: [{ byteLength: 36, uri }],
      bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }],
      accessors: [{
        bufferView: 0, componentType: 5126, count: 3, type: 'VEC3',
        min: [0, 0, 0], max: [1, 1, 0],
      }],
    }));

    // Which files were actually opened. A folder is affordable only because
    // most of it is never read, so this is the assertion that matters most.
    const opened = [];
    const fileFor = (name, bytes) => {
      const file = new File([bytes], name);
      const original = file.arrayBuffer.bind(file);
      Object.defineProperty(file, 'arrayBuffer', {
        value: () => { opened.push(name); return original(); },
      });
      return file;
    };

    const tree = {
      'glTF': {
        'Helmet.gltf': fileFor('Helmet.gltf', gltf('Helmet.bin')),
        'Helmet.bin': fileFor('Helmet.bin', new Uint8Array(positions.buffer)),
        'unused.bin': fileFor('unused.bin', new Uint8Array(36)),
      },
      'glTF-instancing': {
        'HelmetInstanced.gltf': fileFor('HelmetInstanced.gltf', gltf('../glTF/Helmet.bin')),
      },
      'README.md': fileFor('README.md', encoder.encode('# not a model')),
    };

    // The smallest stand-in for a FileSystemEntry the walker accepts.
    const entryFor = (name, node) => (node instanceof File ? {
      isFile: true, isDirectory: false, name,
      file: (onSuccess) => onSuccess(node),
    } : {
      isFile: false, isDirectory: true, name,
      createReader() {
        let done = false;
        return {
          readEntries(onSuccess) {
            if (done) return onSuccess([]);
            done = true;
            onSuccess(Object.entries(node).map(([key, value]) => entryFor(key, value)));
          },
        };
      },
    });

    const root = entryFor('Helmet', tree);
    const event = new Event('drop', { bubbles: true, cancelable: true });
    Object.defineProperty(event, 'dataTransfer', {
      value: { items: [{ webkitGetAsEntry: () => root }], files: [] },
    });
    document.getElementById('drop-zone').dispatchEvent(event);

    const { state } = await import('/app/state.js');
    for (let attempt = 0; attempt < 100 && !state.currentSourceData; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 50));
    }

    const picker = document.getElementById('file-model-picker');

    // The field takes the whole bar — the name's left edge to the remove
    // button's right — and the list is the width of the field, which is what
    // the control has always done. Squeezed into a column beside the name it
    // was less than half that, and every row came back cut to an ellipsis.
    const triggerButton = document.getElementById('file-model-trigger');
    triggerButton.click();
    await new Promise((resolve) => setTimeout(resolve, 50));
    const menu = document.getElementById('file-model-menu');
    const box = (element) => element.getBoundingClientRect();
    const layout = {
      sameWidth: Math.round(box(menu).width) === Math.round(box(triggerButton).width),
      spansBar: box(triggerButton).left <= box(document.getElementById('file-name')).left + 1
        && box(triggerButton).right >= box(document.getElementById('clear-file')).right - 1,
    };
    triggerButton.click();

    const chosen = () => ({
      path: state.currentModelPath,
      resources: Object.keys(state.currentSourceResources),
    });
    const first = chosen();

    // And the other model out of the same drop, without asking for it again.
    const select = document.getElementById('file-model');
    select.value = 'Helmet/glTF-instancing/HelmetInstanced.gltf';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    for (let attempt = 0; attempt < 100
      && state.currentModelPath !== 'Helmet/glTF-instancing/HelmetInstanced.gltf'; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 50));
    }

    return {
      first,
      layout,
      second: chosen(),
      offered: [...select.options].map((option) => [option.value, option.textContent]),
      pickerShown: !picker.hidden,
      opened,
      selected: state.currentSelection.length,
      // A path is longer than the panel it sits in, and the label is nowrap, so
      // the control has to truncate rather than push the panel wider.
      fits: picker.getBoundingClientRect().right
        <= document.getElementById('file-info').getBoundingClientRect().right + 1,
    };
  });

  // Both models are offered, labelled by what differs rather than by the folder
  // they share, and the shorter path is the one that opened.
  expect(observed.pickerShown).toBe(true);
  expect(observed.fits).toBe(true);
  expect(observed.layout.spansBar).toBe(true);
  expect(observed.layout.sameWidth).toBe(true);
  expect(observed.offered).toEqual([
    ['Helmet/glTF/Helmet.gltf', 'glTF/Helmet.gltf'],
    ['Helmet/glTF-instancing/HelmetInstanced.gltf', 'glTF-instancing/HelmetInstanced.gltf'],
  ]);
  expect(observed.first.path).toBe('Helmet/glTF/Helmet.gltf');
  expect(observed.first.resources).toEqual(['Helmet.bin']);

  // The case a file picker cannot supply at all: the buffer is a directory up
  // and back down, and it is keyed by the URI the document wrote.
  expect(observed.second.path).toBe('Helmet/glTF-instancing/HelmetInstanced.gltf');
  expect(observed.second.resources).toEqual(['../glTF/Helmet.bin']);

  // Five files in the folder; `unused.bin` and `README.md` were never read, and
  // `Helmet.bin` was read once per model that named it.
  expect(observed.selected).toBe(5);
  expect(observed.opened).toEqual([
    'Helmet.gltf', 'Helmet.bin', 'HelmetInstanced.gltf', 'Helmet.bin',
  ]);

  await expect(page.locator('#console')).not.toContainText('External resource denied');
});

/**
 * The same folder, chosen with the button rather than dropped.
 *
 * Dragging is not available to everyone — a keyboard user cannot drop at all —
 * so the folder route needs a control. `webkitdirectory` makes an input
 * folder-only, hence a second one beside the file picker rather than a mode on
 * the first. It fills `webkitRelativePath`, so what arrives is the same list of
 * paths a drop produces and takes the same route from there.
 */
test('a folder chosen with the button opens the same way a dropped one does', async ({ page }) => {
  await page.goto('/index.html');
  await waitForConverterReady(page);

  // Alternatives of the same weight, so they are the same size, and neither
  // may break its caption across two lines. Side by side in a sidebar this
  // narrow, the longer caption wrapped and left the pair ragged; stacked, both
  // take the column's width and the question does not arise. Checked at
  // several widths because the defect only appeared at one of them.
  const buttons = await page.evaluate(() => {
    const sidebar = document.querySelector('.sidebar');
    const original = sidebar.style.width;
    const measured = [];
    for (const width of ['360px', '300px', '260px', '220px', '190px']) {
      sidebar.style.width = width;
      const labels = [...document.querySelectorAll('.browse-row label')];
      const zone = document.getElementById('drop-zone').getBoundingClientRect();
      measured.push({
        width,
        count: labels.length,
        widths: labels.map((label) => Math.round(label.getBoundingClientRect().width)),
        wrapped: labels.some((label) => label.scrollWidth > label.clientWidth + 1),
        overflows: labels.some((label) => label.getBoundingClientRect().right > zone.right + 1),
      });
    }
    sidebar.style.width = original;
    return measured;
  });
  for (const measured of buttons) {
    expect(measured.count, `at ${measured.width}`).toBe(2);
    expect(measured.widths[0], `at ${measured.width}`).toBe(measured.widths[1]);
    expect(measured.wrapped, `at ${measured.width}`).toBe(false);
    expect(measured.overflows, `at ${measured.width}`).toBe(false);
  }

  const folder = path.join(repoRoot, 'testdata', 'Fox');
  await page.locator('#folder-input').setInputFiles(folder);

  await expect(page.locator('#console')).toContainText('Preview ready');
  // Fox/glTF/Fox.gltf names Fox.bin and Texture.png beside it — the selection
  // a file picker would have needed three separate clicks to assemble.
  await expect(page.locator('#console')).not.toContainText('External resource denied');
  await expect(page.locator('#console')).not.toContainText('not in the selection');

  const observed = await page.evaluate(async () => {
    const { state } = await import('/app/state.js');
    return {
      path: state.currentModelPath,
      resources: Object.keys(state.currentSourceResources).sort(),
      selected: state.currentSelection.length,
      pickerShown: !document.getElementById('file-model-picker').hidden,
    };
  });

  expect(observed.path).toBe('Fox/glTF/Fox.gltf');
  expect(observed.resources).toEqual(['Fox.bin', 'Texture.png']);
  // One model in the folder, so nothing to choose between and no picker.
  expect(observed.pickerShown).toBe(false);
  expect(observed.selected).toBeGreaterThan(observed.resources.length);
});
