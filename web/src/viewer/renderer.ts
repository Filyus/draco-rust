import type { EnvironmentIbl } from '../environment-ibl.ts';
import {
  MATERIAL_EXTENSION_SLOTS, MATERIAL_EXTENSION_UNIFORMS, materialExtensionFactors,
} from '../material-extensions.ts';
import { mat4, vec3 } from '../math.ts';
import type { Mat4, Vec3 } from '../math.ts';
import { cameraPosition } from './camera.ts';
import type { CameraHost } from './camera.ts';
import { GL } from './gl-utils.ts';
import {
  beginFrameCapture, ensureFrameTarget, finishFrameCapture, frameTargetHdrSupported,
  resolveFrameCapture,
} from './frame-target.ts';
import type { FrameTarget } from './frame-target.ts';
import { MORPH_TEXTURE_UNIT } from './morph-texture.ts';
import {
  MAX_MATERIAL_TEXTURE_UNITS, SHARED_TEXTURE_UNITS, assertTextureUnitBudget, materialTextureUnit,
} from './texture-units.ts';
import type { GlResources, UploadedPrimitive } from './primitive-upload.ts';
import type { SurfaceProgram, SurfaceProgramCache, SurfaceUniforms } from './programs.ts';
import { computeJointMatrices, updateWorldMatrices } from './scene-graph.ts';
import type { SceneGraphHost } from './scene-graph.ts';
import {
  MAX_ACTIVE_MORPH_TARGETS, MAX_JOINTS, MAX_PUNCTUAL_LIGHTS, TEXTURE_SLOTS, TEXTURE_SLOT_SAMPLERS,
} from './shaders.ts';
import type { TextureSlotName } from './shaders.ts';
import type { ViewerMaterial, ViewerNode, ViewerTextureBinding } from '../viewer-scene.ts';

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
 * Neither the unit nor the sampler is stated here: the unit follows from the
 * slot's position in `TEXTURE_SLOTS`, which is also the index the shader
 * addresses it by, and the sampler comes from the same place the shader
 * declares it. What is left is where on the material the binding lives.
 */
interface TextureSlotBinding {
  read(material: ViewerMaterial | undefined): ViewerTextureBinding | null | undefined;
}

const SLOT_BINDINGS: Record<TextureSlotName, TextureSlotBinding> = {
  // Base color is the one slot ViewerMaterial keeps flattened, because OBJ can
  // carry a bare companion-file URI there. Rebuilding the binding here is what
  // lets it travel the same path as every other slot.
  BASE_COLOR: {
    read: (material) => (material?.baseColorTexture == null ? null : {
      index: material.baseColorTexture,
      texCoord: material.baseColorTexCoord ?? 0,
      transform: material.baseColorTextureTransform,
    }),
  },
  METALLIC_ROUGHNESS: { read: (material) => material?.metallicRoughnessTexture },
  EMISSIVE: { read: (material) => material?.emissiveTexture },
  NORMAL: { read: (material) => material?.normalTexture },
  OCCLUSION: { read: (material) => material?.occlusionTexture },
  // The layered extensions bind by the property the table names, so a new one
  // reaches the renderer without a line here.
  ...Object.fromEntries(MATERIAL_EXTENSION_SLOTS.map(({ slot, property }) => [slot, {
    read: (material: ViewerMaterial | undefined) => material?.[property] as ViewerTextureBinding | null | undefined,
  }])),
} as Record<TextureSlotName, TextureSlotBinding>;

/**
 * Which slots this material can actually sample on this primitive.
 *
 * A slot counts only when the material names a texture that was uploaded *and*
 * the primitive carries the UV set that texture reads: sampling a slot whose
 * UVs are absent would read garbage, and declaring one whose texture never
 * arrived would spend a unit on nothing. The answer is the program's identity,
 * so it is computed before anything is bound.
 */
function enabledMaterialSlots(
  host: RenderHost,
  material: ViewerMaterial | undefined,
  uploaded: UploadedPrimitive,
): TextureSlotName[] {
  return TEXTURE_SLOTS.filter((name) => {
    const binding = SLOT_BINDINGS[name].read(material);
    if (!binding) return false;
    const texCoord = binding.texCoord ?? 0;
    const hasUv = texCoord === 0 ? uploaded.hasTexCoords0
      : texCoord === 1 ? uploaded.hasTexCoords1 : false;
    return hasUv && !!host.glResources?.textures[binding.index];
  });
}

/**
 * Whether this material has to wait for the opaque half of the frame.
 *
 * Two reasons to defer a primitive, and they are not the same one: alpha
 * blending needs what is behind it already drawn, and transmission needs the
 * snapshot taken between the passes. A transmissive material is usually
 * OPAQUE, so asking only about the alpha mode would draw it too early and
 * refract a frame that was still empty.
 */
function needsCompletedFrame(material: ViewerMaterial | undefined): boolean {
  return material?.alphaMode === 'BLEND' || (material?.transmissionFactor ?? 0) > 0;
}

/**
 * Bind the surface program this slot set needs, and re-state what the frame
 * told the last one.
 *
 * Uniforms belong to a program, so every switch loses the camera and the
 * environment. Re-uploading them on a switch is a few calls against a handful
 * of distinct programs; tracking which program has seen which frame would cost
 * more to keep honest than it saves.
 */
function useSurfaceProgram(host: RenderHost, slots: readonly TextureSlotName[]) {
  assertTextureUnitBudget(slots.length);
  const surface = host.surfacePrograms.get(slots);
  if (host._surfaceProgram === surface.program) return surface;
  host._surfaceProgram = surface.program;
  host.uniforms = surface.uniforms;
  host.gl.useProgram(surface.program);
  host.gl.uniformMatrix4fv(surface.uniforms.uProjection, false, host._projection);
  host.gl.uniformMatrix4fv(surface.uniforms.uView, false, host._view);
  host.gl.uniform3fv(surface.uniforms.uCameraPos, host._eye!);
  host.gl.uniform1i(surface.uniforms.uLinearOutput, host._linearOutput ? 1 : 0);
  bindEnvironmentIbl(host);
  bindFrameSnapshot(host);
  bindPunctualLights(host);
  return surface;
}

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
  surfacePrograms: SurfaceProgramCache;
  lineProgram: WebGLProgram;
  backgroundProgram: WebGLProgram;
  /**
   * The uniform locations of the surface program currently bound.
   *
   * There is no longer one surface program, so this is state of the frame
   * rather than of the viewer: `useSurfaceProgram` replaces it, and reading it
   * before that has been called for the draw in hand reads someone else's
   * locations.
   */
  uniforms: SurfaceUniforms;
  /** The surface program bound right now, or null before the first draw. */
  _surfaceProgram: WebGLProgram | null;
  /** The linear render of the opaque frame a transmissive surface refracts. */
  _frameTarget?: FrameTarget | null;
  /**
   * Set while that render is being drawn, so the programs it uses write light
   * rather than a picture, and so nothing samples the target it is writing to.
   */
  _linearOutput?: boolean;
  /** Bound in place of the snapshot while the snapshot is being drawn. */
  _snapshotPlaceholder?: WebGLTexture | null;
  /** Whether this machine can hold the capture as half floats. Asked once. */
  _frameTargetHdr?: boolean;
  lineUniforms: Record<string, WebGLUniformLocation | null>;
  backgroundUniforms: Record<string, WebGLUniformLocation | null>;
  downsampleProgram: WebGLProgram;
  downsampleUniforms: Record<string, WebGLUniformLocation | null>;
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
  /** Instance transform buffers, keyed by the scene's own instancing records. */
  _instanceBuffers?: WeakMap<object, WebGLBuffer>;
  /** Reused per-frame light uniforms, resolved from the nodes that place them. */
  _lightTypes?: Int32Array;
  _lightColors?: Float32Array;
  _lightPositions?: Float32Array;
  _lightDirections?: Float32Array;
  _lightParams?: Float32Array;
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

  // What a transmissive surface will refract, drawn before the frame the user
  // sees rather than lifted out of it.
  captureTransmissionSnapshot(host);

  drawBackground(host);

  // Grid (drawn first, depth-disabled so it sits behind everything)
  if (host.showGrid) drawGrid(host);

  // The surface program is chosen per primitive, so no program is current
  // until the first one asks for one.
  host._surfaceProgram = null;

  // Opaque first, then what blends over it. glTF asks for exactly that order.
  drawSurfaces(host, false);
  drawSurfaces(host, true);

  gl.depthMask(true);
  gl.disable(gl.BLEND);
  gl.bindVertexArray(null);
}

/** Whether anything in the scene will read the snapshot at all. */
function sceneRefracts(host: RenderHost): boolean {
  return host.scene!.materials.some((material) => (material?.transmissionFactor ?? 0) > 0);
}

/**
 * Draw the opaque frame once more, into the linear target transmission reads.
 *
 * Skipped outright when no material in the scene transmits, which is the usual
 * case: the pass costs a second run over the opaque geometry, and a scene that
 * will never sample the result should not pay for it. The same question used
 * to be left unasked - the old capture copied the whole canvas and built its
 * mip chain every frame regardless.
 *
 * The grid stays out of it. It is an aid the viewer draws over the scene, not
 * something the asset contains, and glass that refracts the helper geometry
 * reads as a rendering fault rather than as a floor.
 */
function captureTransmissionSnapshot(host: RenderHost) {
  const gl = host.gl;
  if (!sceneRefracts(host)) return;
  if (host._frameTargetHdr === undefined) {
    host._frameTargetHdr = frameTargetHdrSupported(gl);
    // Said once, and only when a scene actually refracts: eight bits over
    // linear light band in the darks and clip anything brighter than white,
    // which a transmissive surface shows as steps in what it looks through.
    if (!host._frameTargetHdr) {
      host._log('Float render targets unavailable; transmission refracts an LDR frame', 'warning');
    }
  }
  host._frameTarget = ensureFrameTarget(
    gl, host._frameTarget ?? null, gl.drawingBufferWidth, gl.drawingBufferHeight,
    host._frameTargetHdr,
  );

  host._linearOutput = true;
  // Nothing may sample the texture now being drawn into, and the unit still
  // holds it from the previous frame - so the placeholder goes in first.
  host._surfaceProgram = null;
  beginFrameCapture(gl, host._frameTarget);
  drawBackground(host);
  drawSurfaces(host, false);
  resolveFrameCapture(gl, host._frameTarget);
  drawCaptureDownsample(host);
  finishFrameCapture(gl, host._frameTarget);
  host._linearOutput = false;
  // Force the next draw to re-state the frame uniforms: every program bound
  // during the capture was told to write light and to read the placeholder.
  host._surfaceProgram = null;
}

/**
 * One pass over the scene's renderables: either what can be drawn straight
 * away, or what had to wait for the rest of the frame.
 *
 * The two halves differ only in which primitives they take, so they share
 * every uniform decision below rather than being written twice.
 */
function drawSurfaces(host: RenderHost, deferred: boolean) {
  const gl = host.gl;
  for (const renderable of host.scene!.renderables) {
    const node = renderable.node;
    const primitives = host.glResources!.primitives[renderable.meshIndex];
    if (!primitives || primitives.length === 0) continue;

    mat4.copy(host._model, node.world);

    const skin = renderable.skinIndex >= 0 ? host.scene!.skins[renderable.skinIndex] : null;
    const jointMatrices = skin
      ? computeJointMatrices(host, skin, node.world, host.glResources!.jointMatrices?.[renderable.skinIndex] ?? null)
      : null;

    // Normal matrix = inverse-transpose(model)
    mat4.invert(host._normalMatrix, host._model);
    mat4.transpose(host._normalMatrix, host._normalMatrix);

    for (let i = 0; i < primitives.length; i++) {
      const { uploaded, materialIndex } = primitives[i];
      const material = host.scene!.materials[materialIndex];
      if (needsCompletedFrame(material) !== deferred) continue;
      // Which program draws this primitive is settled first: everything below
      // writes uniforms, and uniforms belong to whichever program is bound.
      const surface = useSurfaceProgram(host, enabledMaterialSlots(host, material, uploaded));
      gl.uniformMatrix4fv(host.uniforms.uModel, false, host._model);
      gl.uniformMatrix4fv(host.uniforms.uNormalMatrix, false, host._normalMatrix);
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
      applyMaterial(host, surface, material, uploaded, useSmoothNormals);

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
      const instances = bindInstances(host, node);
      if (uploaded.indexType !== undefined) {
        if (instances > 0) gl.drawElementsInstanced(mode, uploaded.elementCount, uploaded.indexType, 0, instances);
        else gl.drawElements(mode, uploaded.elementCount, uploaded.indexType, 0);
      } else if (instances > 0) {
        gl.drawArraysInstanced(mode, 0, uploaded.elementCount, instances);
      } else {
        gl.drawArrays(mode, 0, uploaded.elementCount);
      }
    }
  }
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
  surface: SurfaceProgram,
  material: ViewerMaterial | undefined,
  uploaded: UploadedPrimitive,
  useSmoothNormals: boolean,
) {
  const gl = host.gl;
  gl.uniform1i(host.uniforms.uHasNormals, uploaded.hasNormals || useSmoothNormals ? 1 : 0);
  gl.uniform1i(host.uniforms.uHasVertexColors, uploaded.hasColors ? 1 : 0);
  gl.uniform1i(host.uniforms.uUnlit, material?.unlit ? 1 : 0);
  gl.uniform1i(host.uniforms.uBaseColorOnly, host.baseColorOnly ? 1 : 0);

  // Only the slots this program declares: it was built for exactly the ones
  // the material can sample, so there is no "absent" case left to report, and
  // the units are dense over them.
  const texCoordSlots = host._texCoordSlots ??= new Int32Array(MAX_MATERIAL_TEXTURE_UNITS);
  const texMatrices = host._texMatrices ??= new Float32Array(MAX_MATERIAL_TEXTURE_UNITS * 9);
  for (const [slot, name] of surface.slots.entries()) {
    const unit = materialTextureUnit(slot);
    const binding = SLOT_BINDINGS[name].read(material)!;
    texCoordSlots[slot] = binding.texCoord ?? 0;
    writeTextureMatrix(texMatrices, slot * 9, binding.transform);
    gl.activeTexture(gl.TEXTURE0 + unit);
    gl.bindTexture(gl.TEXTURE_2D, host.glResources!.textures[binding.index]);
    gl.uniform1i(host.uniforms[TEXTURE_SLOT_SAMPLERS[name]], unit);
  }
  if (surface.slots.length > 0) {
    gl.uniform1iv(host.uniforms.uTexCoordSlot, texCoordSlots.subarray(0, surface.slots.length));
    gl.uniformMatrix3fv(host.uniforms.uTexMatrix, false, texMatrices.subarray(0, surface.slots.length * 9));
  }

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
  gl.uniform1f(host.uniforms.uClearcoatNormalScale, material?.clearcoatNormalTexture?.scale ?? 1);
  // Every extension factor the table declares, sent by the name the table
  // implies. A field added there reaches the shader without being named again;
  // what to do with it in GLSL is the only part left to write.
  for (const { property, uniform, components } of MATERIAL_EXTENSION_UNIFORMS) {
    const location = host.uniforms[uniform];
    if (components === 1) gl.uniform1f(location, layered[property] as number);
    else gl.uniform3fv(location, layered[property] as number[]);
  }
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

/**
 * Bring the capture down to the size the frame reads it at.
 *
 * Drawn between the resolve and the mip chain, which is the only place it can
 * be: the samples have to be resolved before they can be averaged, and the mip
 * levels have to be built from the result.
 */
export function drawCaptureDownsample(host: RenderHost) {
  const gl = host.gl;
  const frame = host._frameTarget!;
  gl.disable(gl.DEPTH_TEST);
  gl.depthMask(false);
  gl.disable(gl.BLEND);
  gl.useProgram(host.downsampleProgram);
  gl.activeTexture(gl.TEXTURE0 + SHARED_TEXTURE_UNITS.frameSnapshot);
  gl.bindTexture(gl.TEXTURE_2D, frame.resolved);
  gl.uniform1i(host.downsampleUniforms.uCapture, SHARED_TEXTURE_UNITS.frameSnapshot);
  gl.bindVertexArray(host.backgroundVao);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  gl.bindVertexArray(null);
  gl.depthMask(true);
  gl.enable(gl.DEPTH_TEST);
  // The unit is the snapshot's own, and it now holds the wrong texture.
  host._surfaceProgram = null;
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
  gl.uniform1i(host.backgroundUniforms.uLinearOutput, host._linearOutput ? 1 : 0);
  gl.bindVertexArray(host.backgroundVao);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  gl.bindVertexArray(null);
  gl.depthMask(true);
  gl.enable(gl.DEPTH_TEST);
}

/**
 * A one-texel stand-in for the snapshot, bound while the snapshot is drawn.
 *
 * Sampling a texture that is attached to the framebuffer being drawn into is
 * undefined however carefully the shader avoids reading it, and the unit still
 * holds the target from the frame before. Every material the capture draws is
 * opaque, so what the stand-in contains is never read - only that it is not
 * the attachment.
 */
function snapshotPlaceholder(host: RenderHost) {
  if (!host._snapshotPlaceholder) {
    const gl = host.gl;
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    host._snapshotPlaceholder = texture;
  }
  return host._snapshotPlaceholder;
}

/**
 * Offer the snapshot of the opaque frame to the program now bound.
 *
 * Every program takes it, because whether a given material refracts is a
 * runtime question and the sampler costs a unit either way. Before the first
 * capture there is nothing to bind, and no material that would read it has
 * been drawn.
 */
export function bindFrameSnapshot(host: RenderHost) {
  const gl = host.gl;
  const frame = host._frameTarget;
  if (!frame) return;
  gl.activeTexture(gl.TEXTURE0 + SHARED_TEXTURE_UNITS.frameSnapshot);
  if (host._linearOutput) {
    gl.bindTexture(gl.TEXTURE_2D, snapshotPlaceholder(host));
    gl.uniform1i(host.uniforms.uFrameSnapshot, SHARED_TEXTURE_UNITS.frameSnapshot);
    return;
  }
  gl.bindTexture(gl.TEXTURE_2D, frame.color);
  gl.uniform1i(host.uniforms.uFrameSnapshot, SHARED_TEXTURE_UNITS.frameSnapshot);
  gl.uniform2f(host.uniforms.uFrameSize, frame.width, frame.height);
  // The mip chain is the blur a rough transmissive surface reads through.
  gl.uniform1f(host.uniforms.uFrameMaxLod, Math.log2(Math.max(frame.width, frame.height)));
}

/**
 * Hand the program the scene's punctual lights, in world space.
 *
 * Resolved from the node that places each one, every frame: the node's world
 * matrix is what an animation moves, so a light baked once would stay behind.
 * A light's forward is -Z, which is what every format that has lights means by
 * the direction of a spot or a sun.
 */
/** The four attribute locations one instance matrix occupies, as columns. */
const INSTANCE_COLUMNS = [7, 8, 9, 10];

/**
 * Point the instance attributes at this node's copies, or at the identity.
 *
 * Attribute pointers and divisors belong to the bound VAO, and a VAO is shared
 * by every node that draws the same mesh - so leaving them set would instance
 * the next node that did not ask for it. Both branches therefore write all
 * four columns: a node with copies binds its buffer, one without disables the
 * arrays and leaves the constant identity the shader multiplies by.
 *
 * @returns The instance count, or 0 to draw once.
 */
function bindInstances(host: RenderHost, node: ViewerNode): number {
  const gl = host.gl;
  const instancing = node.instancing;
  if (!instancing) {
    for (const [column, location] of INSTANCE_COLUMNS.entries()) {
      gl.disableVertexAttribArray(location);
      gl.vertexAttrib4f(location, column === 0 ? 1 : 0, column === 1 ? 1 : 0, column === 2 ? 1 : 0, column === 3 ? 1 : 0);
    }
    return 0;
  }
  const buffers = host._instanceBuffers ??= new WeakMap();
  let buffer = buffers.get(instancing);
  if (!buffer) {
    buffer = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, instancing.matrices, gl.STATIC_DRAW);
    buffers.set(instancing, buffer);
  } else {
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  }
  for (const [column, location] of INSTANCE_COLUMNS.entries()) {
    gl.enableVertexAttribArray(location);
    gl.vertexAttribPointer(location, 4, gl.FLOAT, false, 64, column * 16);
    gl.vertexAttribDivisor(location, 1);
  }
  return instancing.count;
}

export function bindPunctualLights(host: RenderHost) {
  const gl = host.gl;
  const lights = host.scene?.lights ?? [];
  const count = Math.min(lights.length, MAX_PUNCTUAL_LIGHTS);
  gl.uniform1i(host.uniforms.uLightCount, count);
  if (count === 0) return;

  const types = host._lightTypes ??= new Int32Array(MAX_PUNCTUAL_LIGHTS);
  const colors = host._lightColors ??= new Float32Array(MAX_PUNCTUAL_LIGHTS * 3);
  const positions = host._lightPositions ??= new Float32Array(MAX_PUNCTUAL_LIGHTS * 3);
  const directions = host._lightDirections ??= new Float32Array(MAX_PUNCTUAL_LIGHTS * 3);
  const params = host._lightParams ??= new Float32Array(MAX_PUNCTUAL_LIGHTS * 4);
  for (let index = 0; index < count; index += 1) {
    const light = lights[index];
    const world = light.node.world;
    types[index] = light.type === 'directional' ? 0 : light.type === 'point' ? 1 : 2;
    colors.set(light.color.slice(0, 3), index * 3);
    // Column-major: the translation is the fourth column, and -Z of the
    // rotation is where the light looks.
    positions.set([world[12], world[13], world[14]], index * 3);
    const forward = [-world[8], -world[9], -world[10]];
    const length = Math.hypot(...forward) || 1;
    directions.set(forward.map((value) => value / length), index * 3);
    params.set([light.intensity, light.range, light.innerConeAngle, light.outerConeAngle], index * 4);
  }
  gl.uniform1iv(host.uniforms.uLightType, types);
  gl.uniform3fv(host.uniforms.uLightColor, colors);
  gl.uniform3fv(host.uniforms.uLightPosition, positions);
  gl.uniform3fv(host.uniforms.uLightDirection, directions);
  gl.uniform4fv(host.uniforms.uLightParams, params);
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
