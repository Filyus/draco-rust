import { expect, test } from '@playwright/test';
import { existsSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  animatedTranslation,
  embeddedTriangle,
  externalTriangle,
  normalMappedQuad,
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
  await page.locator('.anim-clip-option[data-value="1"]').click();
  await expect(page.locator('#anim-clip')).toHaveValue('1');
  await expect(animationTrigger).toContainText('Walk');
  await expect(animationTrigger).toHaveAttribute('aria-expanded', 'false');
  await animationTrigger.press('ArrowDown');
  await expect(page.locator('#anim-clip')).toHaveValue('2');
  await expect(animationTrigger).toContainText('Run');
  await animationTrigger.press('ArrowUp');
  await expect(page.locator('#anim-clip')).toHaveValue('1');
  await animationTrigger.click();
  const selectedClip = page.locator('.anim-clip-option.selected');
  await selectedClip.press('ArrowDown');
  const runClip = page.locator('.anim-clip-option[data-value="2"]');
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
