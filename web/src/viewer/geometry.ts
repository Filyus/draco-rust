import { byteView } from './gl-utils.ts';
import type { RuntimeAccessor, ViewerPrimitive } from '../viewer-scene.ts';

// How far a vertex's skin weights may sum from one before the preview rebuilds
// the attribute. Loose enough to pass ordinary quantization rounding, tight
// enough to catch a vertex that would otherwise be dragged toward the origin.
const WEIGHT_SUM_TOLERANCE = 1e-3;

/**
 * Attribute preparation done on the CPU before upload.
 *
 * Both passes here exist to make source data renderable without altering what
 * an exporter later writes: smooth normals are a preview convenience, and
 * weight renormalization repairs drift that would otherwise drag skinned
 * vertices toward the origin.
 */

export function buildSmoothNormalAttribute(primitive: ViewerPrimitive): RuntimeAccessor | null {
  const positions = primitive.attributes.POSITION;
  const normals = primitive.attributes.NORMAL;
  if (primitive.mode !== 4 || !positions
    || positions.componentType !== 5126 || positions.components !== 3
    || (normals && (normals.componentType !== 5126 || normals.components !== 3
      || positions.count !== normals.count))
    || positions.count === 0) {
    return null;
  }

  const count = positions.count;
  const positionBytes = byteView(positions.bytes);
  const normalBytes = normals ? byteView(normals.bytes) : null;
  if (positionBytes.byteLength !== count * 12
    || (normalBytes && normalBytes.byteLength !== count * 12)) return null;
  const positionView = new DataView(positionBytes.buffer, positionBytes.byteOffset, positionBytes.byteLength);
  const normalView = normalBytes
    ? new DataView(normalBytes.buffer, normalBytes.byteOffset, normalBytes.byteLength)
    : null;
  const position = (index: number, axis: number) => positionView.getFloat32((index * 3 + axis) * 4, true);
  const sourceNormal = (index: number, axis: number) => normalView
    ? normalView.getFloat32((index * 3 + axis) * 4, true)
    : (axis === 1 ? 1 : 0);

  // Join exactly coincident vertices for preview smoothing. When the asset
  // supplies normals, retain authored creases of 60 degrees or more instead
  // of rounding deliberately split cube edges and other hard surfaces.
  const groupIds = new Uint32Array(count);
  const groups = new Map();
  // Per weld group: the accumulated [nx, ny, nz, angleWeight] face samples.
  const contributions: number[][][] = [];
  for (let i = 0; i < count; i++) {
    const key = `${position(i, 0)},${position(i, 1)},${position(i, 2)}`;
    let group = groups.get(key);
    if (group === undefined) {
      group = contributions.length;
      groups.set(key, group);
      contributions.push([]);
    }
    groupIds[i] = group;
  }

  const indices = primitive.indices;
  let indexCount = count;
  let indexAt = (index: number) => index;
  if (indices) {
    const bytes = byteView(indices.bytes);
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    indexCount = indices.count;
    if (indices.componentType === 5121 && bytes.byteLength === indexCount) {
      indexAt = (index) => view.getUint8(index);
    } else if (indices.componentType === 5123 && bytes.byteLength === indexCount * 2) {
      indexAt = (index) => view.getUint16(index * 2, true);
    } else if (indices.componentType === 5125 && bytes.byteLength === indexCount * 4) {
      indexAt = (index) => view.getUint32(index * 4, true);
    } else {
      return null;
    }
  }
  if (indexCount % 3 !== 0) return null;

  const cornerAngle = (ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
    const divisor = Math.hypot(ax, ay, az) * Math.hypot(bx, by, bz);
    if (divisor <= 1e-12) return 0;
    return Math.acos(Math.max(-1, Math.min(1, (ax * bx + ay * by + az * bz) / divisor)));
  };
  for (let offset = 0; offset < indexCount; offset += 3) {
    const vertices = [indexAt(offset), indexAt(offset + 1), indexAt(offset + 2)];
    if (vertices.some((index) => index >= count)) return null;
    const points = vertices.map((index) => [position(index, 0), position(index, 1), position(index, 2)]);
    const edge1 = points[1].map((value, axis) => value - points[0][axis]);
    const edge2 = points[2].map((value, axis) => value - points[0][axis]);
    let face = [
      edge1[1] * edge2[2] - edge1[2] * edge2[1],
      edge1[2] * edge2[0] - edge1[0] * edge2[2],
      edge1[0] * edge2[1] - edge1[1] * edge2[0],
    ];
    const faceLength = Math.hypot(face[0], face[1], face[2]);
    if (faceLength <= 1e-12) continue;
    face = face.map((value) => value / faceLength);
    for (let corner = 0; corner < 3; corner++) {
      const point = points[corner];
      const a = points[(corner + 1) % 3].map((value: number, axis: number) => value - point[axis]);
      const b = points[(corner + 2) % 3].map((value: number, axis: number) => value - point[axis]);
      const weight = cornerAngle(a[0], a[1], a[2], b[0], b[1], b[2]);
      contributions[groupIds[vertices[corner]]].push([
        face[0],
        face[1],
        face[2],
        weight,
      ]);
    }
  }

  const output = new Float32Array(count * 3);
  const creaseCosine = Math.cos(Math.PI / 3);
  for (let i = 0; i < count; i++) {
    let reference: number[] | null = null;
    if (normalView) {
      const length = Math.hypot(sourceNormal(i, 0), sourceNormal(i, 1), sourceNormal(i, 2));
      if (length > 1e-12) {
        reference = [
          sourceNormal(i, 0) / length,
          sourceNormal(i, 1) / length,
          sourceNormal(i, 2) / length,
        ];
      }
    }
    const sum = [0, 0, 0];
    for (const [x, y, z, weight] of contributions[groupIds[i]]) {
      if (reference && x * reference[0] + y * reference[1] + z * reference[2]
        < creaseCosine - 1e-6) {
        continue;
      }
      sum[0] += x * weight;
      sum[1] += y * weight;
      sum[2] += z * weight;
    }
    const length = Math.hypot(...sum);
    for (let axis = 0; axis < 3; axis++) {
      output[i * 3 + axis] = length > 1e-12 ? sum[axis] / length : sourceNormal(i, axis);
    }
    const dot = normalView ? output[i * 3] * sourceNormal(i, 0)
      + output[i * 3 + 1] * sourceNormal(i, 1)
      + output[i * 3 + 2] * sourceNormal(i, 2) : 1;
    if (dot < 0) {
      output[i * 3] *= -1;
      output[i * 3 + 1] *= -1;
      output[i * 3 + 2] *= -1;
    }
  }
  return { bytes: output, componentType: 5126, components: 3, normalized: false, count };
}

/**
 * Copy one morph accessor into its texel slot of a packed layer. Both loaders
 * reject targets that are not float vec3, and accessor bytes can start at an
 * unaligned offset, so the payload is read through a DataView.
 */

function weightScalars(attribute: RuntimeAccessor): Float32Array | null {
  const { buffer, byteOffset, byteLength } = byteView(attribute.bytes);
  switch (attribute.componentType) {
    case 5126:
      return new Float32Array(buffer, byteOffset, byteLength / 4);
    case 5121:
      return Float32Array.from(new Uint8Array(buffer, byteOffset, byteLength), (v) => v / 255);
    case 5123:
      return Float32Array.from(
        new Uint16Array(buffer, byteOffset, byteLength / 2),
        (v) => v / 65535,
      );
    default:
      return null;
  }
}

/**
 * Return `WEIGHTS_0` with every vertex summing to one.
 *
 * The skinning shader blends joint matrices weighted by this attribute and does
 * not renormalize, so a vertex whose weights sum to nearly zero is placed at the
 * model origin and stretches its triangles across the whole scene. Quantized
 * skins drift off unit sum routinely — Draco encodes weights lossily — so treat
 * the source buffer as advisory. A well-formed attribute is passed through
 * without copying.
 */
export function buildNormalizedWeightAttribute(
  primitive: ViewerPrimitive,
): { attribute: RuntimeAccessor | null; drifted: number } {
  const attribute = primitive.attributes.WEIGHTS_0;
  if (!attribute || attribute.components !== 4) return { attribute: attribute || null, drifted: 0 };
  const source = weightScalars(attribute);
  if (!source || source.length < attribute.count * 4) {
    return { attribute, drifted: 0 };
  }

  const sumAt = (vertex: number) => source[vertex * 4] + source[vertex * 4 + 1]
    + source[vertex * 4 + 2] + source[vertex * 4 + 3];
  let drifted = 0;
  for (let vertex = 0; vertex < attribute.count; vertex++) {
    if (Math.abs(sumAt(vertex) - 1) > WEIGHT_SUM_TOLERANCE) drifted++;
  }
  if (drifted === 0) return { attribute, drifted: 0 };

  const values = new Float32Array(attribute.count * 4);
  for (let vertex = 0; vertex < attribute.count; vertex++) {
    const base = vertex * 4;
    const sum = sumAt(vertex);
    if (sum > WEIGHT_SUM_TOLERANCE) {
      for (let c = 0; c < 4; c++) values[base + c] = source[base + c] / sum;
    } else {
      // No influence survived quantization. Binding the vertex rigidly to
      // its first joint keeps it on the body; normalizing a zero vector
      // cannot, and leaving it collapses the vertex onto the origin.
      values[base] = 1;
    }
  }
  return {
    attribute: {
      bytes: values,
      componentType: 5126,
      components: 4,
      normalized: false,
      count: attribute.count,
    },
    drifted,
  };
}

/**
 * Reads `JOINTS_0` as plain indices, whatever width it was stored at.
 *
 * Unlike weights these are not normalized values, so an unsigned byte means
 * joint 200, not 200/255.
 */
function jointIndices(attribute: RuntimeAccessor): Uint16Array | null {
  // `byteView` throws on anything that is not binary, and a caller here is
  // asking a question about the data rather than promising there is any: a
  // hand-built primitive that carries no real payload is answered with `null`,
  // not with an exception that would take the whole scene down with it.
  if (!ArrayBuffer.isView(attribute.bytes)) return null;
  const { buffer, byteOffset, byteLength } = byteView(attribute.bytes);
  switch (attribute.componentType) {
    case 5121:
      return Uint16Array.from(new Uint8Array(buffer, byteOffset, byteLength));
    case 5123:
      return new Uint16Array(buffer, byteOffset, byteLength / 2);
    default:
      return null;
  }
}

/**
 * Renumber `JOINTS_0` onto the joints this primitive actually uses.
 *
 * The shader holds one fixed uniform array of joint matrices, so a joint index
 * is a slot in it, and a skin with more joints than slots cannot be drawn as
 * authored. A skin that large is ordinary: one measured character carries 489
 * joints across four skins while no single primitive references more than 90
 * of them, and the ones it references are spread to index 415. Truncating the
 * palette at the slot count therefore does not drop unused joints, it drops
 * used ones, and every vertex bound to them collapses.
 *
 * The renumbering removes the whole class: slots are handed out in order of
 * first appearance, so a primitive needs as many as it references and no more.
 * `palette[slot]` is the skin joint that slot stands for, which is what the
 * matrices are then computed from.
 *
 * `null` leaves the attribute alone -- there is nothing to gain when the
 * indices already fit, and nothing to be done when the primitive genuinely
 * references more joints than the shader has slots.
 */
export function referencedJoints(primitive: ViewerPrimitive): Uint16Array | null {
  const attribute = primitive.attributes.JOINTS_0;
  if (!attribute) return null;
  const source = jointIndices(attribute);
  const width = attribute.components;
  if (!source || width < 1 || source.length < attribute.count * width) return null;
  const seen = new Set<number>();
  const found: number[] = [];
  for (let i = 0; i < attribute.count * width; i++) {
    const joint = source[i];
    if (!seen.has(joint)) {
      seen.add(joint);
      found.push(joint);
    }
  }
  return Uint16Array.from(found);
}

export function buildJointPalette(
  primitive: ViewerPrimitive,
  maxJoints: number,
): { attribute: RuntimeAccessor | null; palette: Uint16Array | null } {
  const attribute = primitive.attributes.JOINTS_0;
  if (!attribute) return { attribute: null, palette: null };
  const source = jointIndices(attribute);
  const width = attribute.components;
  if (!source || width < 1 || source.length < attribute.count * width) {
    return { attribute, palette: null };
  }

  const slotOf = new Map<number, number>();
  const palette: number[] = [];
  const remapped = new Uint16Array(attribute.count * width);
  for (let i = 0; i < attribute.count * width; i++) {
    const joint = source[i];
    let slot = slotOf.get(joint);
    if (slot === undefined) {
      slot = palette.length;
      slotOf.set(joint, slot);
      palette.push(joint);
    }
    remapped[i] = slot;
  }

  if (palette.length > maxJoints) return { attribute, palette: null };
  // Already dense and in order: the renumbering would be the identity, and the
  // attribute the file carries is the one to bind.
  if (palette.every((joint, slot) => joint === slot)) return { attribute, palette: null };

  return {
    attribute: {
      bytes: remapped,
      componentType: 5123,
      components: width,
      normalized: false,
      count: attribute.count,
    },
    palette: Uint16Array.from(palette),
  };
}

/**
 * Build GPU buffers for one Mesh primitive.
 * Returns an object describing attribute locations, VAO, index/element counts.
 */
