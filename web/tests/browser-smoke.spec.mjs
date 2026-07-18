import { expect, test } from '@playwright/test';

import {
  embeddedTriangle,
  externalTriangle,
  triangleBytes,
} from './smoke-fixtures.mjs';

test('document and compact APIs read data URI, GLB, and explicit resources', async ({ page }) => {
  await page.goto('/index.html');
  const result = await page.evaluate(async ({ embedded, external, resource }) => {
    const documentApi = await import('/pkg/gltf_document.js');
    const compactApi = await import('/pkg/gltf_compact.js');
    await Promise.all([documentApi.default(), compactApi.default()]);

    const data = new TextEncoder().encode(embedded);
    const document = new documentApi.GltfDocument(data, '2.0');
    const compact = new compactApi.CompactDocument(data, '2.0');
    const packed = compact.readPrimitive(0, 0);
    const glb = document.glb(2);
    const roundtrip = new documentApi.GltfDocument(glb, '2.0');
    let missing = false;
    try {
      new documentApi.GltfDocument(new TextEncoder().encode(external), '2.0');
    } catch (error) {
      missing = String(error).includes('missing.bin');
    }
    const resolved = documentApi.GltfDocument.withResources(
      new TextEncoder().encode(external),
      { 'missing.bin': new Uint8Array(resource) },
      '2.0',
    );
    return {
      document: document.summary(),
      compact: { meshes: compact.meshCount(), bytes: packed.attributeBytes(0).length },
      roundtrip: roundtrip.summary(),
      missing,
      resolved: resolved.summary(),
    };
  }, {
    embedded: embeddedTriangle(),
    external: externalTriangle(),
    resource: Array.from(triangleBytes()),
  });

  expect(result.document.success).toBe(true);
  expect(result.document.meshCount).toBe(1);
  expect(result.compact).toEqual({ meshes: 1, bytes: 36 });
  expect(result.roundtrip.success).toBe(true);
  expect(result.missing).toBe(true);
  expect(result.resolved.success).toBe(true);
});
