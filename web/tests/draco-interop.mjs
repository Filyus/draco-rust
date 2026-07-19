import draco3d from 'draco3d';

const GLB_MAGIC = 0x46546c67;
const JSON_CHUNK = 0x4e4f534a;
const BIN_CHUNK = 0x004e4942;

function firstDracoPayload(glb) {
  const bytes = glb instanceof Uint8Array ? glb : new Uint8Array(glb);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (bytes.byteLength < 20 || view.getUint32(0, true) !== GLB_MAGIC) {
    throw new Error('expected a GLB container');
  }
  if (view.getUint32(4, true) !== 2) {
    throw new Error('official draco3d smoke currently expects GLB v2');
  }
  const declaredLength = view.getUint32(8, true);
  if (declaredLength !== bytes.byteLength) {
    throw new Error(`GLB length ${declaredLength} does not match ${bytes.byteLength} bytes`);
  }

  let json;
  let binary;
  for (let offset = 12; offset < declaredLength;) {
    if (offset + 8 > declaredLength) {
      throw new Error('truncated GLB chunk header');
    }
    const length = view.getUint32(offset, true);
    const type = view.getUint32(offset + 4, true);
    const start = offset + 8;
    const end = start + length;
    if (end > declaredLength) {
      throw new Error('truncated GLB chunk');
    }
    if (type === JSON_CHUNK) {
      json = JSON.parse(new TextDecoder().decode(bytes.subarray(start, end)));
    } else if (type === BIN_CHUNK) {
      binary = bytes.subarray(start, end);
    }
    offset = end;
  }
  if (!json || !binary) {
    throw new Error('GLB must contain JSON and BIN chunks');
  }

  const primitive = json.meshes?.[0]?.primitives?.[0];
  const extension = primitive?.extensions?.KHR_draco_mesh_compression;
  if (!extension) {
    throw new Error('first primitive is not Draco-compressed');
  }
  const bufferView = json.bufferViews?.[extension.bufferView];
  if (!bufferView || (bufferView.buffer ?? 0) !== 0) {
    throw new Error('Draco buffer view does not reference the GLB BIN chunk');
  }
  const start = bufferView.byteOffset ?? 0;
  const end = start + bufferView.byteLength;
  if (end > binary.byteLength) {
    throw new Error('Draco buffer view exceeds the GLB BIN chunk');
  }
  const positionAccessor = primitive.attributes?.POSITION;
  return {
    bytes: binary.subarray(start, end),
    declaredPoints: json.accessors?.[positionAccessor]?.count,
    declaredIndices: primitive.indices === undefined
      ? undefined
      : json.accessors?.[primitive.indices]?.count,
  };
}

export async function decodeFirstDracoPrimitive(glb) {
  const payload = firstDracoPayload(glb);
  const module = await draco3d.createDecoderModule({});
  const decoder = new module.Decoder();
  const buffer = new module.DecoderBuffer();
  const mesh = new module.Mesh();
  let status;
  try {
    const signedBytes = new Int8Array(
      payload.bytes.buffer,
      payload.bytes.byteOffset,
      payload.bytes.byteLength,
    );
    buffer.Init(signedBytes, signedBytes.byteLength);
    status = decoder.DecodeBufferToMesh(buffer, mesh);
    if (!status.ok()) {
      throw new Error(`official draco3d rejected the payload: ${status.error_msg()}`);
    }
    return {
      points: mesh.num_points(),
      faces: mesh.num_faces(),
      declaredPoints: payload.declaredPoints,
      declaredIndices: payload.declaredIndices,
    };
  } finally {
    if (status) module.destroy(status);
    module.destroy(mesh);
    module.destroy(buffer);
    module.destroy(decoder);
  }
}
