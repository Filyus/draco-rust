export const TRIANGLE_BASE64 = 'AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA';

export function triangleBytes() {
  const binary = atob(TRIANGLE_BASE64);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

export function embeddedTriangle() {
  return JSON.stringify({
    asset: { version: '2.0' },
    buffers: [{
      byteLength: 36,
      uri: `data:application/octet-stream;base64,${TRIANGLE_BASE64}`,
    }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }],
    accessors: [{
      bufferView: 0,
      componentType: 5126,
      count: 3,
      type: 'VEC3',
    }],
    meshes: [{
      name: 'Triangle',
      primitives: [{ attributes: { POSITION: 0 } }],
    }],
    nodes: [{ mesh: 0 }],
    scenes: [{ nodes: [0] }],
    scene: 0,
  });
}

export function externalTriangle() {
  return JSON.stringify({
    asset: { version: '2.0' },
    buffers: [{ uri: 'missing.bin', byteLength: 36 }],
    bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 36 }],
    accessors: [{
      bufferView: 0,
      componentType: 5126,
      count: 3,
      type: 'VEC3',
    }],
    meshes: [{ primitives: [{ attributes: { POSITION: 0 } }] }],
  });
}

export function externalImageTriangle() {
  const document = JSON.parse(externalTriangle());
  document.images = [{ uri: 'missing.png' }];
  return JSON.stringify(document);
}

export function triangleMesh() {
  return {
    name: 'Triangle',
    positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
    indices: [0, 1, 2],
    normals: null,
    uvs: null,
  };
}

export function assertSmokeResults(reader, writer) {
  const embedded = reader.parse_gltf(embeddedTriangle());
  if (!embedded.success || embedded.meshes.length !== 1 || embedded.meshes[0].indices.length !== 3) {
    throw new Error(`data-URI glTF smoke failed: ${JSON.stringify(embedded)}`);
  }

  const created = writer.create_gltf([triangleMesh()], {
    use_draco: false,
    format: 'glb',
  });
  if (!created.success || !created.binary_data) {
    throw new Error(`GLB writer smoke failed: ${JSON.stringify(created)}`);
  }
  const roundtrip = reader.parse_glb(new Uint8Array(created.binary_data));
  if (!roundtrip.success || roundtrip.meshes.length !== 1 || roundtrip.meshes[0].indices.length !== 3) {
    throw new Error(`GLB roundtrip smoke failed: ${JSON.stringify(roundtrip)}`);
  }

  const createdDraco = writer.create_gltf([triangleMesh()], {
    use_draco: true,
    format: 'glb',
    encoding_speed: 7,
    decoding_speed: 6,
    encoding_method: 0,
  });
  if (
    !createdDraco.success
    || !createdDraco.binary_data
    || createdDraco.draco_stats?.method !== 'sequential'
    || createdDraco.compression_report?.compressed_primitives.length !== 1
  ) {
    throw new Error(`Draco writer smoke failed: ${JSON.stringify(createdDraco)}`);
  }
  const dracoRoundtrip = reader.parse_glb(new Uint8Array(createdDraco.binary_data));
  if (!dracoRoundtrip.success || dracoRoundtrip.meshes[0].indices.length !== 3) {
    throw new Error(`Draco writer roundtrip failed: ${JSON.stringify(dracoRoundtrip)}`);
  }
  const embeddedDraco = writer.create_gltf([triangleMesh()], {
    use_draco: true,
    format: 'gltf',
  });
  if (!embeddedDraco.success || !embeddedDraco.json_data || embeddedDraco.binary_data) {
    throw new Error(`embedded Draco writer smoke failed: ${JSON.stringify(embeddedDraco)}`);
  }
  const embeddedDracoRoundtrip = reader.parse_gltf(embeddedDraco.json_data);
  if (!embeddedDracoRoundtrip.success || embeddedDracoRoundtrip.meshes[0].indices.length !== 3) {
    throw new Error(`embedded Draco roundtrip failed: ${JSON.stringify(embeddedDracoRoundtrip)}`);
  }

  const missing = reader.parse_gltf_with_resources(
    new TextEncoder().encode(externalTriangle()),
    {},
  );
  if (missing.success || !missing.error?.includes('missing.bin')) {
    throw new Error(`missing-resource smoke failed: ${JSON.stringify(missing)}`);
  }

  const resolved = reader.parse_gltf_with_resources(
    new TextEncoder().encode(externalTriangle()),
    { 'missing.bin': triangleBytes() },
  );
  if (!resolved.success || resolved.meshes.length !== 1 || resolved.meshes[0].indices.length !== 3) {
    throw new Error(`companion-resource smoke failed: ${JSON.stringify(resolved)}`);
  }

  const missingImage = reader.parse_gltf_with_resources(
    new TextEncoder().encode(externalImageTriangle()),
    { 'missing.bin': triangleBytes() },
  );
  if (missingImage.success || !missingImage.error?.includes('missing.png')) {
    throw new Error(`missing-image smoke failed: ${JSON.stringify(missingImage)}`);
  }
  const resolvedImage = reader.parse_gltf_with_resources(
    new TextEncoder().encode(externalImageTriangle()),
    { 'missing.bin': triangleBytes(), 'missing.png': new Uint8Array([0]) },
  );
  if (!resolvedImage.success) {
    throw new Error(`image companion smoke failed: ${JSON.stringify(resolvedImage)}`);
  }

  const compressed = writer.compress_gltf_document_with_resources(
    new TextEncoder().encode(externalTriangle()),
    { 'missing.bin': triangleBytes() },
    {
      format: 'glb',
      encoding_speed: 7,
      decoding_speed: 6,
      encoding_method: 0,
    },
  );
  if (
    !compressed.success
    || !compressed.binary_data
    || compressed.draco_stats?.method !== 'sequential'
    || compressed.draco_stats?.speed !== 7
    || compressed.compression_report?.compressed_primitives.length !== 1
    || compressed.compression_report?.preserved_primitives.length !== 0
  ) {
    throw new Error(`document-compression smoke failed: ${JSON.stringify(compressed)}`);
  }
  const compressedRoundtrip = reader.parse_glb(new Uint8Array(compressed.binary_data));
  if (
    !compressedRoundtrip.success
    || compressedRoundtrip.meshes.length !== 1
    || compressedRoundtrip.meshes[0].indices.length !== 3
  ) {
    throw new Error(`compressed GLB roundtrip failed: ${JSON.stringify(compressedRoundtrip)}`);
  }
}
