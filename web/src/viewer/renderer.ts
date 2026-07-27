import type { EnvironmentIbl } from '../environment-ibl.ts';
import { materialExtensionFactors } from '../material-extensions.ts';
import { mat4, vec3 } from '../math.ts';
import type { Mat4, Vec3 } from '../math.ts';
import { cameraPosition } from './camera.ts';
import type { CameraHost } from './camera.ts';
import { GL } from './gl-utils.ts';
import { MORPH_TEXTURE_UNIT } from './morph-texture.ts';
import { SHARED_TEXTURE_UNITS, assertTextureUnitBudget, materialTextureUnit } from './texture-units.ts';
import type { GlResources, UploadedPrimitive } from './primitive-upload.ts';
import { computeJointMatrices, updateWorldMatrices } from './scene-graph.ts';
import type { SceneGraphHost } from './scene-graph.ts';
import { MAX_ACTIVE_MORPH_TARGETS, MAX_JOINTS, TEXTURE_SLOTS } from './shaders.ts';
import type { TextureSlotName } from './shaders.ts';
import type { ViewerMaterial, ViewerTextureBinding } from '../viewer-scene.ts';

/**
 * What drawing a frame needs from the viewer: the context, the linked
 * programs and their uniform locations, the uploaded GL resources, and the
 * display flags the user toggles.
 */
/** World up, shared by every view matrix; hoisted out of the frame. */
const WORLD_UP = new Float32Array([0, 1, 0]);

/**
 * How one material texture slot reaches the GPU.
 *
 * The unit is not stated here: it follows from the slot's position in
 * `TEXTURE_SLOTS`, which is also the index the shader addresses it by. A slot
 * that carried its own number could disagree with both.
 */
interface TextureSlotBinding {
  textureUniform: string;
  hasUniform: string;
  read(material: ViewerMaterial | undefined): ViewerTextureBinding | null | undefined;
}

const SLOT_BINDINGS: Record<TextureSlotName, TextureSlotBinding> = {
  // Base color is the one slot ViewerMaterial keeps flattened, because OBJ can
  // carry a bare companion-file URI there. Rebuilding the binding here is what
  // lets it travel the same path as the other nine.
  BASE_COLOR: {
    textureUniform: 'uBaseColor',
    hasUniform: 'uHasTexture',
    read: (material) => (material?.baseColorTexture == null ? null : {
      index: material.baseColorTexture,
      texCoord: material.baseColorTexCoord ?? 0,
      transform: material.baseColorTextureTransform,
    }),
  },
  METALLIC_ROUGHNESS: {
    textureUniform: 'uMetallicRoughness',
    hasUniform: 'uHasMetallicRoughnessTexture',
    read: (material) => material?.metallicRoughnessTexture,
  },
  EMISSIVE: {
    textureUniform: 'uEmissive',
    hasUniform: 'uHasEmissiveTexture',
    read: (material) => material?.emissiveTexture,
  },
  NORMAL: {
    textureUniform: 'uNormalTexture',
    hasUniform: 'uHasNormalTexture',
    read: (material) => material?.normalTexture,
  },
  OCCLUSION: {
    textureUniform: 'uOcclusionTexture',
    hasUniform: 'uHasOcclusionTexture',
    read: (material) => material?.occlusionTexture,
  },
  SPECULAR: {
    textureUniform: 'uSpecularTexture',
    hasUniform: 'uHasSpecularTexture',
    read: (material) => material?.specularTexture,
  },
  SPECULAR_COLOR: {
    textureUniform: 'uSpecularColorTexture',
    hasUniform: 'uHasSpecularColorTexture',
    read: (material) => material?.specularColorTexture,
  },
  CLEARCOAT: {
    textureUniform: 'uClearcoatTexture',
    hasUniform: 'uHasClearcoatTexture',
    read: (material) => material?.clearcoatTexture,
  },
  CLEARCOAT_ROUGHNESS: {
    textureUniform: 'uClearcoatRoughnessTexture',
    hasUniform: 'uHasClearcoatRoughnessTexture',
    read: (material) => material?.clearcoatRoughnessTexture,
  },
  CLEARCOAT_NORMAL: {
    textureUniform: 'uClearcoatNormalTexture',
    hasUniform: 'uHasClearcoatNormalTexture',
    read: (material) => material?.clearcoatNormalTexture,
  },
};

/**
 * The slots in shader index order: position here is the `SLOT_*` constant the
 * fragment program uses. Built by mapping over `TEXTURE_SLOTS` so the two
 * cannot drift apart. Fourteen samplers total, inside the WebGL2 floor of 16.
 */
const MATERIAL_TEXTURE_SLOTS = TEXTURE_SLOTS.map((name) => SLOT_BINDINGS[name]);
assertTextureUnitBudget(MATERIAL_TEXTURE_SLOTS.length);

/**
 * Write one slot's `KHR_texture_transform` as a column-major mat3.
 *
 * The spec composes it as translation * rotation * scale applied to the UV,
 * with the rotation running clockwise; expanded, that is exactly the arithmetic
 * the base color slot used to do in the shader. An absent transform writes the
 * identity, so the shader keeps a single code path.
 */
function writeTextureMatrix(
  out: Float32Array,
  at: number,
  transform: ViewerTextureBinding['transform'] | undefined,
) {
  const scale = transform?.scale || [1, 1];
  const offset = transform?.offset || [0, 0];
  const rotation = transform?.rotation || 0;
  const c = Math.cos(rotation);
  const s = Math.sin(rotation);
  out[at] = scale[0] * c;
  out[at + 1] = scale[0] * s;
  out[at + 2] = 0;
  out[at + 3] = -scale[1] * s;
  out[at + 4] = scale[1] * c;
  out[at + 5] = 0;
  out[at + 6] = offset[0];
  out[at + 7] = offset[1];
  out[at + 8] = 1;
}

export interface RenderHost extends CameraHost, SceneGraphHost {
  gl: WebGL2RenderingContext;
  glResources: GlResources | null;
  program: WebGLProgram;
  lineProgram: WebGLProgram;
  backgroundProgram: WebGLProgram;
  uniforms: Record<string, WebGLUniformLocation | null>;
  lineUniforms: Record<string, WebGLUniformLocation | null>;
  backgroundUniforms: Record<string, WebGLUniformLocation | null>;
  backgroundVao: WebGLVertexArrayObject | null;
  environmentIbl: EnvironmentIbl;
  wireframe: boolean;
  showGrid: boolean;
  baseColorOnly: boolean;
  smoothNormals: boolean;
  _projection: Mat4;
  _view: Mat4;
  _projectionView: Mat4;
  _inverseProjection: Mat4;
  _inverseView: Mat4;
  _model: Mat4;
  _normalMatrix: Mat4;
  _eye?: Vec3;
  _grid?: { buffer: WebGLBuffer; count: number } | null;
  _gridVao?: WebGLVertexArrayObject | null;
  _morphWeights?: Float32Array;
  _morphLayers?: Int32Array;
  /** Reused ordering buffer for the per-frame morph target pick. */
  _morphOrder?: number[];
  /** Reused per-material slot uniforms: UV set and texture transform. */
  _texCoordSlots?: Int32Array;
  _texMatrices?: Float32Array;
  _emptyMorphTexture?: WebGLTexture | null;
  _morphPlaceholderTexture?: WebGLTexture | null;
  _log(message: string, type?: string): void;
  _disposeGrid(): void;
}

/**
 * The frame: camera matrices, the backdrop, the grid, and one pass over the
 * scene's renderables.
 *
 * This is the only place where GL resources and per-frame state meet. It reads
 * the viewer's uploaded resources and display flags through the host it is
 * given rather than owning any of them.
 */

export function render(host: RenderHost) {
  const gl = host.gl;
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
  if (!host.scene || !host.glResources) return;

  // Compute camera matrices
  const aspect = host.canvas.width / Math.max(1, host.canvas.height);
  mat4.perspective(host._projection, host.camera.fov, aspect, host.camera.near, host.camera.far);

  const eye = cameraPosition(host, host._eye || (host._eye = vec3.create()));
  mat4.lookAt(host._view, eye, host.camera.target, WORLD_UP);

  updateWorldMatrices(host);

  drawBackground(host);

  // Grid (drawn first, depth-disabled so it sits behind everything)
  if (host.showGrid) drawGrid(host);

  gl.useProgram(host.program);
  gl.uniformMatrix4fv(host.uniforms.uProjection, false, host._projection);
  gl.uniformMatrix4fv(host.uniforms.uView, false, host._view);
  gl.uniform3fv(host.uniforms.uCameraPos, eye);
  bindEnvironmentIbl(host);

  for (const renderable of host.scene.renderables) {
    const node = renderable.node;
    const primitives = host.glResources.primitives[renderable.meshIndex];
    if (!primitives || primitives.length === 0) continue;

    mat4.copy(host._model, node.world);

    const skin = renderable.skinIndex >= 0 ? host.scene.skins[renderable.skinIndex] : null;
    const jointMatrices = skin
      ? computeJointMatrices(host, skin, node.world, host.glResources.jointMatrices?.[renderable.skinIndex] ?? null)
      : null;

    // Normal matrix = inverse-transpose(model)
    mat4.invert(host._normalMatrix, host._model);
    mat4.transpose(host._normalMatrix, host._normalMatrix);
    gl.uniformMatrix4fv(host.uniforms.uModel, false, host._model);
    gl.uniformMatrix4fv(host.uniforms.uNormalMatrix, false, host._normalMatrix);

    for (let i = 0; i < primitives.length; i++) {
      const { uploaded, materialIndex } = primitives[i];
      const usesSkin = !!(jointMatrices && uploaded.hasJoints && uploaded.hasWeights);
      gl.uniform1i(host.uniforms.uUseSkin, usesSkin ? 1 : 0);
      gl.uniform1i(host.uniforms.uJointCount, usesSkin ? jointMatrices.length / 16 : 0);
      if (usesSkin) gl.uniformMatrix4fv(host.uniforms.uJointMatrix, false, jointMatrices);
      gl.bindVertexArray(uploaded.vao);
      const morph = uploaded.morph;
      gl.activeTexture(gl.TEXTURE0 + MORPH_TEXTURE_UNIT);
      gl.bindTexture(gl.TEXTURE_2D_ARRAY, morph ? morph.texture : morphPlaceholder(host));
      gl.uniform1i(host.uniforms.uMorphDeltas, MORPH_TEXTURE_UNIT);
      gl.uniform1i(host.uniforms.uMorphCount, selectMorphTargets(host, morph, node.weights));
      gl.uniform1i(host.uniforms.uMorphStride, morph ? morph.stride : 1);
      gl.uniform1i(host.uniforms.uMorphWidth, morph ? morph.width : 1);
      gl.uniform1fv(host.uniforms.uMorphWeights, host._morphWeights!);
      gl.uniform1iv(host.uniforms.uMorphLayers, host._morphLayers!);
      const useSmoothNormals = host.smoothNormals
        && uploaded.hasSmoothNormals && uploaded.morphTargetCount === 0;
      gl.uniform1i(
        host.uniforms.uUseSmoothNormals,
        useSmoothNormals ? 1 : 0,
      );
      const material = host.scene.materials[materialIndex];
      applyMaterial(host, material, uploaded, useSmoothNormals);

      if (material?.doubleSided) gl.disable(gl.CULL_FACE);
      else gl.enable(gl.CULL_FACE);

      if (material?.alphaMode === 'BLEND') {
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        gl.depthMask(false);
      } else {
        gl.disable(gl.BLEND);
        gl.depthMask(true);
      }

      const mode = glMode(host, uploaded.mode, host.wireframe);
      if (uploaded.indexType !== undefined) {
        gl.drawElements(mode, uploaded.elementCount, uploaded.indexType, 0);
      } else {
        gl.drawArrays(mode, 0, uploaded.elementCount);
      }
    }
  }

  gl.depthMask(true);
  gl.disable(gl.BLEND);
  gl.bindVertexArray(null);
}

/**
 * Stage this draw's active morph targets into `_morphLayers`/`_morphWeights`
 * and return how many the shader should blend.
 *
 * The shader loop is bounded, so a mesh with more targets than that keeps
 * its strongest-weighted ones. Real clips stay far below the bound: a glTF
 * weight track blends two neighbouring poses at a time, and even a facial
 * rig drives only a handful of shapes at once.
 */
export function selectMorphTargets(
  host: RenderHost,
  morph: UploadedPrimitive['morph'],
  weights: ArrayLike<number> | undefined,
) {
  const staged = host._morphWeights
    || (host._morphWeights = new Float32Array(MAX_ACTIVE_MORPH_TARGETS));
  const layers = host._morphLayers
    || (host._morphLayers = new Int32Array(MAX_ACTIVE_MORPH_TARGETS));
  staged.fill(0);
  layers.fill(0);
  if (!morph || !weights) return 0;

  const order = host._morphOrder || (host._morphOrder = []);
  order.length = 0;
  for (let i = 0; i < morph.layerCount; i++) {
    if (weights[i] && morph.filled[i]) order.push(i);
  }
  // Ties keep the lower target index so a held pose stays stable.
  order.sort((a: number, b: number) => Math.abs(weights[b]) - Math.abs(weights[a]) || a - b);

  const count = Math.min(order.length, MAX_ACTIVE_MORPH_TARGETS);
  for (let slot = 0; slot < count; slot++) {
    layers[slot] = order[slot];
    staged[slot] = weights[order[slot]];
  }
  return count;
}

/**
 * Array texture bound when a primitive has no morph targets. A sampler must
 * reference a complete texture even on the branch that never samples it.
 */
export function morphPlaceholder(host: RenderHost) {
  if (!host._emptyMorphTexture) {
    const gl = host.gl;
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, texture);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texStorage3D(gl.TEXTURE_2D_ARRAY, 1, gl.RGBA32F, 1, 1, 1);
    host._emptyMorphTexture = texture;
  }
  return host._emptyMorphTexture;
}

export function glMode(host: RenderHost, mode: number, wireframe: boolean) {
  const gl = host.gl;
  if (wireframe) return gl.LINES;
  switch (mode) {
    case 0: return gl.POINTS;
    case 1: return gl.LINES;
    case 2: return gl.LINE_LOOP;
    case 3: return gl.LINE_STRIP;
    case 5: return gl.TRIANGLE_STRIP;
    case 6: return gl.TRIANGLE_FAN;
    default: return gl.TRIANGLES;
  }
}

export function applyMaterial(
  host: RenderHost,
  material: ViewerMaterial | undefined,
  uploaded: UploadedPrimitive,
  useSmoothNormals: boolean,
) {
  const gl = host.gl;
  gl.uniform1i(host.uniforms.uHasNormals, uploaded.hasNormals || useSmoothNormals ? 1 : 0);
  gl.uniform1i(host.uniforms.uHasVertexColors, uploaded.hasColors ? 1 : 0);
  gl.uniform1i(host.uniforms.uUnlit, material?.unlit ? 1 : 0);
  gl.uniform1i(host.uniforms.uBaseColorOnly, host.baseColorOnly ? 1 : 0);

  // Every slot at once: bind its texture, say whether it is usable, and record
  // the UV set and transform the shader will sample it with.
  const texCoordSlots = host._texCoordSlots ??= new Int32Array(MATERIAL_TEXTURE_SLOTS.length);
  const texMatrices = host._texMatrices ??= new Float32Array(MATERIAL_TEXTURE_SLOTS.length * 9);
  for (const [slot, { textureUniform, hasUniform, read }] of MATERIAL_TEXTURE_SLOTS.entries()) {
    const unit = materialTextureUnit(slot);
    const binding = read(material);
    const texCoord = binding?.texCoord ?? 0;
    const hasUv = texCoord === 0 ? uploaded.hasTexCoords0
      : texCoord === 1 ? uploaded.hasTexCoords1 : false;
    const texture = host.glResources?.textures[binding?.index ?? -1];
    const enabled = !!texture && hasUv;
    gl.uniform1i(host.uniforms[hasUniform], enabled ? 1 : 0);
    texCoordSlots[slot] = texCoord;
    writeTextureMatrix(texMatrices, slot * 9, binding?.transform);
    if (enabled) {
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.uniform1i(host.uniforms[textureUniform], unit);
    }
  }
  gl.uniform1iv(host.uniforms.uTexCoordSlot, texCoordSlots);
  gl.uniformMatrix3fv(host.uniforms.uTexMatrix, false, texMatrices);

  const factor = material?.baseColorFactor || [1, 1, 1, 1];
  gl.uniform4f(host.uniforms.uBaseColorFactor, factor[0], factor[1], factor[2], factor[3]);
  gl.uniform1f(host.uniforms.uMetallic, material?.metallic ?? 0);
  gl.uniform1f(host.uniforms.uRoughness, material?.roughness ?? 1);
  // KHR_materials_emissive_strength scales the factor rather than reaching the
  // shader on its own: the two are always multiplied together anyway.
  // OBJ, PLY and FBX materials never had these properties, and the portable
  // form drops any that equal the core model's value, so the defaults come
  // from the table that decided what "equal to the core model" means.
  const layered = materialExtensionFactors(material) as Record<string, any>;
  const emissive = material?.emissiveFactor || [0, 0, 0];
  const emissiveStrength = layered.emissiveStrength;
  gl.uniform3f(
    host.uniforms.uEmissiveFactor,
    emissive[0] * emissiveStrength,
    emissive[1] * emissiveStrength,
    emissive[2] * emissiveStrength,
  );
  gl.uniform1f(host.uniforms.uNormalScale, material?.normalTexture?.scale ?? 1);
  gl.uniform1f(host.uniforms.uOcclusionStrength, material?.occlusionTexture?.strength ?? 1);
  gl.uniform1f(host.uniforms.uIor, layered.ior);
  gl.uniform1f(host.uniforms.uSpecularFactor, layered.specularFactor);
  const specularColor = layered.specularColorFactor;
  gl.uniform3f(host.uniforms.uSpecularColorFactor, specularColor[0], specularColor[1], specularColor[2]);
  gl.uniform1f(host.uniforms.uClearcoatFactor, layered.clearcoatFactor);
  gl.uniform1f(host.uniforms.uClearcoatRoughnessFactor, layered.clearcoatRoughnessFactor);
  gl.uniform1f(host.uniforms.uClearcoatNormalScale, material?.clearcoatNormalTexture?.scale ?? 1);
}

export function drawGrid(host: RenderHost) {
  const gl = host.gl;
  mat4.multiply(host._projectionView, host._projection, host._view);
  if (!host._grid) buildSceneGrid(host);
  if (!host._grid) return;
  gl.useProgram(host.lineProgram);
  gl.uniformMatrix4fv(host.lineUniforms.uProjectionView, false, host._projectionView);
  gl.uniform3f(host.lineUniforms.uColor, 0.31, 0.40, 0.56);
  gl.bindBuffer(gl.ARRAY_BUFFER, host._grid.buffer);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
  gl.drawArrays(gl.LINES, 0, host._grid.count);
}

/** Render the same radiance cubemap used by material IBL. */
export function drawBackground(host: RenderHost) {
  const gl = host.gl;
  if (!mat4.invert(host._inverseProjection, host._projection)
    || !mat4.invert(host._inverseView, host._view)) return;
  gl.disable(gl.DEPTH_TEST);
  gl.depthMask(false);
  gl.useProgram(host.backgroundProgram);
  gl.uniformMatrix4fv(host.backgroundUniforms.uInverseProjection, false, host._inverseProjection);
  gl.uniformMatrix4fv(host.backgroundUniforms.uInverseView, false, host._inverseView);
  gl.activeTexture(gl.TEXTURE5);
  gl.bindTexture(gl.TEXTURE_CUBE_MAP, host.environmentIbl.environment);
  gl.uniform1i(host.backgroundUniforms.uEnvironment, 5);
  gl.bindVertexArray(host.backgroundVao);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  gl.bindVertexArray(null);
  gl.depthMask(true);
  gl.enable(gl.DEPTH_TEST);
}

export function bindEnvironmentIbl(host: RenderHost) {
  const gl = host.gl;
  gl.activeTexture(gl.TEXTURE0 + SHARED_TEXTURE_UNITS.irradiance);
  gl.bindTexture(gl.TEXTURE_CUBE_MAP, host.environmentIbl.irradiance);
  gl.uniform1i(host.uniforms.uIrradianceMap, SHARED_TEXTURE_UNITS.irradiance);
  gl.activeTexture(gl.TEXTURE0 + SHARED_TEXTURE_UNITS.prefiltered);
  gl.bindTexture(gl.TEXTURE_CUBE_MAP, host.environmentIbl.prefiltered);
  gl.uniform1i(host.uniforms.uPrefilteredMap, SHARED_TEXTURE_UNITS.prefiltered);
  gl.activeTexture(gl.TEXTURE0 + SHARED_TEXTURE_UNITS.brdfLut);
  gl.bindTexture(gl.TEXTURE_2D, host.environmentIbl.brdfLut);
  gl.uniform1i(host.uniforms.uBrdfLut, SHARED_TEXTURE_UNITS.brdfLut);
  gl.uniform1f(host.uniforms.uEnvironmentMaxLod, host.environmentIbl.maxLod);
}

/** Build a grid scaled to the loaded model's AABB. */
export function buildSceneGrid(host: RenderHost) {
  const box = host.scene?.aabb;
  if (!box) return;
  const dx = box.max[0] - box.min[0];
  const dz = box.max[2] - box.min[2];
  const span = Math.max(dx, dz);
  if (!isFinite(span) || span <= 0) return;

  // Round the cell size up to a "nice" power of ten so the grid reads well.
  const targetCells = 20;
  const rawStep = span / targetCells;
  const magnitude = Math.pow(10, Math.floor(Math.log10(rawStep)));
  const step = [1, 2, 5, 10]
    .map((m) => m * magnitude)
    .find((s) => s >= rawStep) || rawStep;

  const cx = (box.min[0] + box.max[0]) * 0.5;
  const cz = (box.min[2] + box.max[2]) * 0.5;
  const halfExtent = Math.ceil(span / step / 2) + 2;
  const half = halfExtent * step;

  const positions = [];
  const minI = Math.round((cx - half) / step);
  const maxI = Math.round((cx + half) / step);
  const minJ = Math.round((cz - half) / step);
  const maxJ = Math.round((cz + half) / step);
  // Nudged below the model so the two never z-fight; relative to the cell size,
  // because a fixed offset sinks a centimetre-sized asset under its own grid.
  const gridY = box.min[1] - step * 0.01;
  for (let i = minI; i <= maxI; i++) {
    const x = i * step;
    positions.push(x, gridY, minJ * step, x, gridY, maxJ * step);
  }
  for (let j = minJ; j <= maxJ; j++) {
    const z = j * step;
    positions.push(minI * step, gridY, z, maxI * step, gridY, z);
  }

  const buffer = host.gl.createBuffer();
  host.gl.bindBuffer(host.gl.ARRAY_BUFFER, buffer);
  host.gl.bufferData(host.gl.ARRAY_BUFFER, new Float32Array(positions), host.gl.STATIC_DRAW);
  host._grid = { buffer, count: positions.length / 3 };
}
