/**
 * Reading glTF component types out of raw bytes.
 *
 * Both the importers and the exporters walk accessor payloads, and every one
 * of them needs the same three facts about a component type: how wide it is,
 * how to read it, and how a normalized integer maps back to its float range.
 * Keeping one copy here means a new width is added in one place.
 *
 * The one payload-shaped reader here, `morphDeltaAccessor`, is here for the
 * same reason: two importers need the identical expansion, and neither may
 * depend on the other.
 */

const COMPONENT_BYTES = new Map<number, number>([
  [5120, 1], [5121, 1], [5122, 2], [5123, 2], [5125, 4], [5126, 4],
]);

/** Width in bytes, or undefined for a type outside the supported set. */
export function componentByteWidth(componentType: number): number | undefined {
  return COMPONENT_BYTES.get(componentType);
}

/** Width in bytes, rejecting anything outside the supported set. */
export function componentByteSize(componentType: number): number {
  const width = COMPONENT_BYTES.get(componentType);
  if (width === undefined) throw new Error(`Unsupported component type ${componentType}`);
  return width;
}

/** Read one little-endian component at a byte offset. */
export function readComponent(view: DataView, offset: number, componentType: number): number {
  switch (componentType) {
    case 5120: return view.getInt8(offset);
    case 5121: return view.getUint8(offset);
    case 5122: return view.getInt16(offset, true);
    case 5123: return view.getUint16(offset, true);
    case 5125: return view.getUint32(offset, true);
    case 5126: return view.getFloat32(offset, true);
    default: throw new Error(`Unsupported component type ${componentType}`);
  }
}

/**
 * Map a normalized integer component onto its float range.
 *
 * Signed types clamp at -1 because the most negative value of a two's
 * complement integer has no positive counterpart, which glTF resolves by
 * mapping both it and the next value to -1.
 */
export function normalizeComponent(value: number, componentType: number): number {
  switch (componentType) {
    case 5120: return Math.max(value / 127, -1);
    case 5121: return value / 255;
    case 5122: return Math.max(value / 32767, -1);
    case 5123: return value / 65535;
    default: return value;
  }
}

/** Whether `normalized` on this component type means anything. */
export function isNormalizedIntegerType(componentType: number): boolean {
  return componentType === 5120 || componentType === 5121
    || componentType === 5122 || componentType === 5123;
}

/**
 * The fields any accessor payload has to expose to be read component-wise.
 *
 * `bytes` is an `ArrayBufferView` rather than a `Uint8Array` because the FBX
 * morph path hands over dense float deltas directly; only the buffer, offset
 * and length matter here.
 */
export interface AccessorBytes {
  bytes: ArrayBufferView;
  componentType: number;
  components: number;
  normalized?: boolean;
  count: number;
}

/** One morph target expanded to float triples. */
export interface FloatDeltaAccessor {
  componentType: 5126;
  components: 3;
  count: number;
  normalized: false;
  bytes: Uint8Array;
  data: Float32Array;
}

/**
 * Materialize morph deltas as float triples, or null when the accessor cannot
 * describe deltas for a primitive of `vertexCount` vertices.
 *
 * `KHR_mesh_quantization` stores deltas as integers in the same space as the
 * base attribute — normalized ones as unit fractions, plain ones as raw counts
 * that the node scale turns back into model units — while the morph texture the
 * preview blends through is float only.
 *
 * It lives here rather than with either importer because both of them need it:
 * the glTF loader expands on the way in, and the SceneDocument adapter expands
 * on the way out of a document that is required to keep the source spelling.
 * While only the loader had it, the same asset animated through one path and
 * stood in its rest pose through the other.
 */
export function morphDeltaAccessor<T extends AccessorBytes>(
  target: T,
  vertexCount: number,
): T | FloatDeltaAccessor | null {
  if (target.components !== 3 || target.count !== vertexCount) return null;
  if (target.componentType === 5126) return target;
  const width = componentByteWidth(target.componentType);
  if (width === undefined || target.componentType === 5125) return null;
  const length = target.count * 3;
  if (target.bytes.byteLength < length * width) return null;
  const view = new DataView(target.bytes.buffer, target.bytes.byteOffset, target.bytes.byteLength);
  const values = new Float32Array(length);
  const normalized = Boolean(target.normalized) && isNormalizedIntegerType(target.componentType);
  for (let index = 0; index < length; index += 1) {
    const value = readComponent(view, index * width, target.componentType);
    values[index] = normalized ? normalizeComponent(value, target.componentType) : value;
  }
  return {
    componentType: 5126,
    components: 3,
    count: target.count,
    normalized: false,
    bytes: new Uint8Array(values.buffer),
    data: values,
  };
}
