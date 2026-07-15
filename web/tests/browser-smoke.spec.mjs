import { expect, test } from '@playwright/test';

import {
  embeddedTriangle,
  externalImageTriangle,
  externalTriangle,
  triangleBytes,
  triangleMesh,
} from './smoke-fixtures.mjs';

test('data URI, GLB roundtrip, and missing buffer/image resources', async ({ page }) => {
  await page.goto('/index.html');
  const result = await page.evaluate(async ({ embedded, external, externalImage, mesh, resource }) => {
    const reader = await import('/pkg/gltf_reader.js');
    const writer = await import('/pkg/gltf_writer.js');
    await Promise.all([reader.default(), writer.default()]);

    const parsed = reader.parse_gltf(embedded);
    const created = writer.create_gltf([mesh], { use_draco: false, format: 'glb' });
    const roundtrip = created.success
      ? reader.parse_glb(new Uint8Array(created.binary_data))
      : created;
    const missing = reader.parse_gltf_with_resources(
      new TextEncoder().encode(external),
      {},
    );
    const resolved = reader.parse_gltf_with_resources(
      new TextEncoder().encode(external),
      { 'missing.bin': new Uint8Array(resource) },
    );
    const missingImage = reader.parse_gltf_with_resources(
      new TextEncoder().encode(externalImage),
      { 'missing.bin': new Uint8Array(resource) },
    );
    const resolvedImage = reader.parse_gltf_with_resources(
      new TextEncoder().encode(externalImage),
      {
        'missing.bin': new Uint8Array(resource),
        'missing.png': new Uint8Array([0]),
      },
    );
    return { parsed, created, roundtrip, missing, resolved, missingImage, resolvedImage };
  }, {
    embedded: embeddedTriangle(),
    external: externalTriangle(),
    externalImage: externalImageTriangle(),
    mesh: triangleMesh(),
    resource: Array.from(triangleBytes()),
  });

  expect(result.parsed.success).toBe(true);
  expect(result.parsed.meshes[0].indices).toHaveLength(3);
  expect(result.created.success).toBe(true);
  expect(result.roundtrip.success).toBe(true);
  expect(result.roundtrip.meshes[0].indices).toHaveLength(3);
  expect(result.missing.success).toBe(false);
  expect(result.missing.error).toContain('missing.bin');
  expect(result.resolved.success).toBe(true);
  expect(result.resolved.meshes[0].indices).toHaveLength(3);
  expect(result.missingImage.success).toBe(false);
  expect(result.missingImage.error).toContain('missing.png');
  expect(result.resolvedImage.success).toBe(true);
});
