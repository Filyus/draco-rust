import { buildJointPalette, buildNormalizedWeightAttribute, buildSmoothNormalAttribute } from './geometry.ts';
import { byteView } from './gl-utils.ts';
import { uploadMorphTexture } from './morph-texture.ts';
import type { RuntimeAccessor, ViewerPrimitive } from '../viewer-scene.ts';

/** Attribute slot -> shader location, as the program layout reports it. */
type LocationMap = Record<string, number>;

/**
 * What the renderer needs to draw this primitive later.
 *
 * The `has*` flags answer questions the shader asks per material, so they are
 * recorded once at upload instead of re-derived per frame. `morph` and
 * `indexType` are optional because they are filled in after the literal, and
 * only when the primitive has targets or an index buffer at all.
 */
export interface UploadedPrimitive {
  vao: WebGLVertexArrayObject | null;
  buffers: (WebGLBuffer | null)[];
  hasNormals: boolean;
  hasSmoothNormals: boolean;
  hasTexCoords0: boolean;
  hasTexCoords1: boolean;
  hasColors: boolean;
  hasJoints: boolean;
  /**
   * Which skin joint each slot of `uJointMatrix` stands for, when this
   * primitive was renumbered onto its own palette. Absent when the joint
   * indices are the skin's own -- see `buildJointPalette`.
   */
  jointPalette?: Uint16Array;
  hasWeights: boolean;
  /** How many WEIGHTS_0 vertices had to be renormalized on upload. */
  driftedWeights: number;
  mode: number;
  elementCount: number;
  /**
   * The element type of the index buffer, and the sole record of whether the
   * primitive has one: present means indexed, absent means drawArrays.
   */
  indexType?: number;
  morph?: ReturnType<typeof uploadMorphTexture>;
  morphTargetCount?: number;
}

/** One drawable: its GPU buffers, and the material the scene paired it with. */
export interface UploadedPrimitiveSlot {
  uploaded: UploadedPrimitive;
  materialIndex: number;
}

/**
 * Everything the viewer holds on the GPU for the current scene.
 *
 * Parallel to the scene it was built from: `primitives[mesh][primitive]` and
 * `textures[index]` index the same arrays `ViewerScene` does. Null until a
 * scene is uploaded, and released wholesale when one is replaced.
 */
export interface GlResources {
  primitives: UploadedPrimitiveSlot[][];
  textures: (WebGLTexture | null)[];
  jointMatrices: Float32Array[] | null;
}

/** One primitive's VAO, buffers and attribute layout. */

/**
 * Vertex buffers already uploaded for this scene, keyed by the accessor.
 *
 * Several primitives of one mesh are commonly the same vertices drawn under
 * different materials, and they carry the very same accessor objects. Without
 * this each of them uploaded its own copy of every attribute: one character
 * split into thirteen primitives put 224 MiB on the GPU for 24 MiB of vertices.
 */
export type SharedVertexBuffers = Map<object, WebGLBuffer>;

export function uploadPrimitive(
  gl: WebGL2RenderingContext,
  primitive: ViewerPrimitive,
  locationMap: LocationMap,
  maxJoints: number,
  shared?: SharedVertexBuffers,
): UploadedPrimitive {
  const vao = gl.createVertexArray();
  gl.bindVertexArray(vao);

  const buffers: WebGLBuffer[] = [];
  const positions = primitive.attributes.POSITION;
  if (!positions) throw new Error('primitive is missing POSITION attribute');

  function bindAccessor(
    attr: RuntimeAccessor | null | undefined,
    semantic: string,
    location: number,
    desiredComponents?: number,
  ): boolean {
    if (!attr || location < 0) return false;
    if (desiredComponents && attr.components !== desiredComponents) {
      gl.disableVertexAttribArray(location);
      return false;
    }
    // Keyed by the accessor rather than its bytes: the accessor is what states
    // how they are read, and two primitives sharing one carry the same object.
    let buf = shared?.get(attr as object) ?? null;
    if (buf) {
      gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    } else {
      buf = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, buf);
      gl.bufferData(gl.ARRAY_BUFFER, byteView(attr.bytes), gl.STATIC_DRAW);
      shared?.set(attr as object, buf);
    }
    // Listed on every primitive that reads it; release deletes each once.
    buffers.push(buf);
    const normalized = attr.normalized
      || semantic.startsWith('COLOR_') || semantic.startsWith('WEIGHTS_');
    gl.enableVertexAttribArray(location);
    gl.vertexAttribPointer(location, attr.components, attr.componentType, normalized, 0, 0);
    return true;
  }

  function bindAttribute(semantic: string, location: number, desiredComponents?: number) {
    return bindAccessor(primitive.attributes[semantic], semantic, location, desiredComponents);
  }

  const skinWeights = buildNormalizedWeightAttribute(primitive);
  const skinJoints = buildJointPalette(primitive, maxJoints);
  const layout = {
    position: locationMap.position,
    normal: locationMap.normal,
    texCoord: locationMap.texCoord,
    texCoord1: locationMap.texCoord1,
    color: locationMap.color,
    joints: locationMap.joints,
    weights: locationMap.weights,
    smoothNormal: locationMap.smoothNormal,
  };

  const info: UploadedPrimitive = {
    vao,
    buffers,
    hasNormals: !!bindAttribute('NORMAL', layout.normal),
    hasSmoothNormals: !!bindAccessor(
      buildSmoothNormalAttribute(primitive),
      'SMOOTH_NORMAL',
      layout.smoothNormal,
      3,
    ),
    hasTexCoords0: !!bindAttribute('TEXCOORD_0', layout.texCoord),
    hasTexCoords1: !!bindAttribute('TEXCOORD_1', layout.texCoord1),
    hasColors: !!bindAttribute('COLOR_0', layout.color),
    hasJoints: !!bindAccessor(skinJoints.attribute, 'JOINTS_0', layout.joints),
    hasWeights: !!bindAccessor(skinWeights.attribute, 'WEIGHTS_0', layout.weights),
    driftedWeights: skinWeights.drifted,
    mode: primitive.mode,
    elementCount: 0,
    ...(skinJoints.palette ? { jointPalette: skinJoints.palette } : {}),
  };

  bindAttribute('POSITION', layout.position);
  // Layers are indexed exactly like the mesh weights, so picking targets at
  // draw time is a plain lookup by target index.
  info.morph = uploadMorphTexture(gl, primitive, positions.count);
  info.morphTargetCount = info.morph ? info.morph.layerCount : 0;

  let indexBuffer: WebGLBuffer | null = null;
  if (primitive.indices) {
    const idx = primitive.indices;
    const bytes = byteView(idx.bytes);
    indexBuffer = gl.createBuffer();
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, indexBuffer);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, bytes, gl.STATIC_DRAW);
    info.elementCount = idx.count;
    info.indexType = idx.componentType;
    buffers.push(indexBuffer);
  } else {
    info.elementCount = positions.count;
  }

  gl.bindVertexArray(null);
  return info;
}
