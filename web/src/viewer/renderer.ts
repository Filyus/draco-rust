import type { EnvironmentIbl } from '../environment-ibl.ts';
import {
  MATERIAL_EXTENSION_SLOTS, MATERIAL_EXTENSION_UNIFORMS, materialExtensionFactors,
} from '../material-extensions.ts';
import { mat4, vec3 } from '../math.ts';
import type { Mat4, Vec3 } from '../math.ts';
import { cameraPosition } from './camera.ts';
import type { CameraHost } from './camera.ts';
import { GL } from './gl-utils.ts';
import { ensureBloomChain } from './bloom.ts';
import type { BloomChain } from './bloom.ts';
import {
  beginScene, captureOpaqueHalf, ensureSceneTarget, GUARD_BAND, resolveScene,
  sceneTargetHdrSupported,
} from './scene-target.ts';
import type { SceneTarget } from './scene-target.ts';
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
import type { Renderable, ViewerMaterial, ViewerNode, ViewerTextureBinding } from '../viewer-scene.ts';

/**
 * What drawing a frame needs from the viewer: the context, the linked
 * programs and their uniform locations, the uploaded GL resources, and the
 * display flags the user toggles.
 */
/**
 * How much of the frame's light the output pass spreads as glare.
 *
 * A few percent: enough that a source far brighter than white reads as one,
 * which is what an optical system does with it, and not so much that the whole
 * image hazes over.
 */
const DEFAULT_BLOOM_STRENGTH = 0.05;

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
 * What one material's alpha mode asks the shader for.
 *
 * Only MASK cuts, and its cutoff defaults to the spec's half. Only BLEND keeps
 * the alpha channel: OPAQUE means the channel is *ignored*, which is not the
 * same as it happening to be one — a base colour texture is free to carry a
 * cut-out there, and a material that never asked to be transparent must not
 * inherit it. A cutoff of zero discards nothing, which is what the two
 * non-masking modes want and also what `alphaCutoff: 0` means on a MASK one.
 */
export function alphaModeUniforms(material: ViewerMaterial | undefined) {
  const mode = material?.alphaMode ?? 'OPAQUE';
  return {
    cutoff: mode === 'MASK' ? material?.alphaCutoff ?? 0.5 : 0,
    opaque: mode !== 'BLEND',
  };
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
  /** The linear frame everything is drawn into, and its opaque-half copy. */
  _sceneTarget?: SceneTarget | null;
  /** The glare pyramid the output pass reads. */
  _bloom?: BloomChain | null;
  /** Bound in place of the capture before there is one. */
  _snapshotPlaceholder?: WebGLTexture | null;
  /** Whether this machine can hold the frame as half floats. Asked once. */
  _sceneTargetHdr?: boolean;
  /** How much of the frame's light the output pass spreads as glare. */
  bloomStrength?: number;
  /** Draw the frame at twice the size and average it down. Costly; off. */
  supersample?: boolean;
  /** Stops applied to the frame before the tone curve. */
  exposure?: number;
  lineUniforms: Record<string, WebGLUniformLocation | null>;
  backgroundUniforms: Record<string, WebGLUniformLocation | null>;
  outputProgram: WebGLProgram;
  outputUniforms: Record<string, WebGLUniformLocation | null>;
  bloomDownProgram: WebGLProgram;
  bloomDownUniforms: Record<string, WebGLUniformLocation | null>;
  bloomUpProgram: WebGLProgram;
  bloomUpUniforms: Record<string, WebGLUniformLocation | null>;
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
  if (!host.scene || !host.glResources) {
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    return;
  }

  // Compute camera matrices. The frame is drawn wider than it is shown, so the
  // frustum opens by the same factor: the extra texels have to hold more
  // scene, not the same scene larger.
  const aspect = host.canvas.width / Math.max(1, host.canvas.height);
  const guard = 1 + 2 * Math.max(0, GUARD_BAND.margin);
  const guardedFov = 2 * Math.atan(Math.tan(host.camera.fov / 2) * guard);
  mat4.perspective(host._projection, guardedFov, aspect, host.camera.near, host.camera.far);

  const eye = cameraPosition(host, host._eye || (host._eye = vec3.create()));
  mat4.lookAt(host._view, eye, host.camera.target, WORLD_UP);

  updateWorldMatrices(host);

  const scene = ensureSceneResources(host);
  beginScene(gl, scene);

  drawBackground(host);

  // Grid (drawn first, depth-disabled so it sits behind everything)
  if (host.showGrid) drawGrid(host);

  // The surface program is chosen per primitive, so no program is current
  // until the first one asks for one.
  host._surfaceProgram = null;

  // Opaque first, then what blends over it or looks through it, back to front.
  // glTF asks for exactly that order, and the copy transmission reads is taken
  // inside the second pass, whenever the surface about to refract needs to see
  // something blended behind it.
  drawOpaqueSurfaces(host);
  drawDeferredSurfaces(host, scene);

  gl.depthMask(true);
  gl.disable(gl.BLEND);
  gl.bindVertexArray(null);

  resolveScene(gl, scene);
  drawGlare(host);
  drawOutput(host);
}

/**
 * The frame and the glare pyramid for the drawing buffer as it stands now.
 *
 * Half floats are asked for once: a frame that cannot hold light past white
 * still draws, it just clips everything an emitter does, and the tone curve
 * then has nothing left to roll off.
 */
function ensureSceneResources(host: RenderHost): SceneTarget {
  const gl = host.gl;
  if (host._sceneTargetHdr === undefined) {
    host._sceneTargetHdr = sceneTargetHdrSupported(gl);
    if (!host._sceneTargetHdr) {
      host._log('Float render targets unavailable; the frame clips at white', 'warning');
    }
  }
  const scene = ensureSceneTarget(
    gl, host._sceneTarget ?? null, gl.drawingBufferWidth, gl.drawingBufferHeight,
    host._sceneTargetHdr, host.supersample ?? false,
  );
  host._sceneTarget = scene;
  host._bloom = ensureBloomChain(
    gl, host._bloom ?? null, scene.renderWidth, scene.renderHeight, scene.hdr);
  return scene;
}

/**
 * Put one renderable's matrices on the host and return its joint palette.
 *
 * Split out of the pass because the deferred half no longer walks renderables
 * in order: it walks its own sorted list, and each entry has to arrive with
 * the same state around it that the straight walk left.
 */
function prepareRenderable(host: RenderHost, renderable: Renderable) {
  const node = renderable.node;
  mat4.copy(host._model, node.world);
  // Normal matrix = inverse-transpose(model)
  mat4.invert(host._normalMatrix, host._model);
  mat4.transpose(host._normalMatrix, host._normalMatrix);
  const skinIndex = renderable.skinIndex;
  const skin = skinIndex >= 0 ? host.scene!.skins[skinIndex] : null;
  // Recomputed rather than cached across the pass: the palette is written into
  // one buffer per skin, so two renderables sharing a skin would hand back the
  // same array with the last one's pose in it.
  return skin
    ? computeJointMatrices(host, skin, node.world, host.glResources!.jointMatrices?.[skinIndex] ?? null)
    : null;
}

/**
 * Draw one primitive with the matrices `prepareRenderable` left on the host.
 */
function drawPrimitive(
  host: RenderHost,
  renderable: Renderable,
  primitiveIndex: number,
  jointMatrices: Float32Array | null,
) {
  const gl = host.gl;
  const node = renderable.node;
  const { uploaded, materialIndex } = host.glResources!.primitives[renderable.meshIndex][primitiveIndex];
  const material = host.scene!.materials[materialIndex];
  // Which program draws this primitive is settled first: everything below
  // writes uniforms, and uniforms belong to whichever program is bound.
  const surface = useSurfaceProgram(host, enabledMaterialSlots(host, material, uploaded));
  gl.uniformMatrix4fv(host.uniforms.uModel, false, host._model);
  gl.uniformMatrix4fv(host.uniforms.uNormalMatrix, false, host._normalMatrix);
  const usesSkin = !!(jointMatrices && uploaded.hasJoints && uploaded.hasWeights);
  gl.uniform1i(host.uniforms.uUseSkin, usesSkin ? 1 : 0);
  gl.uniform1i(host.uniforms.uJointCount, usesSkin ? jointMatrices!.length / 16 : 0);
  if (usesSkin) gl.uniformMatrix4fv(host.uniforms.uJointMatrix, false, jointMatrices!);
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
  gl.uniform1i(host.uniforms.uUseSmoothNormals, useSmoothNormals ? 1 : 0);
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

/** Everything that can be drawn before the frame holds anything else. */
function drawOpaqueSurfaces(host: RenderHost) {
  for (const renderable of host.scene!.renderables) {
    const primitives = host.glResources!.primitives[renderable.meshIndex];
    if (!primitives || primitives.length === 0) continue;
    const jointMatrices = prepareRenderable(host, renderable);
    for (let i = 0; i < primitives.length; i++) {
      const material = host.scene!.materials[primitives[i].materialIndex];
      if (needsCompletedFrame(material)) continue;
      drawPrimitive(host, renderable, i, jointMatrices);
    }
  }
}

/** One primitive that had to wait, and how far from the eye it sits. */
export interface DeferredDraw {
  renderable: Renderable;
  primitiveIndex: number;
  distance: number;
  transmissive: boolean;
}

/**
 * The primitives that had to wait, farthest first.
 *
 * Sorted by the eye distance of the mesh box's centre in world space. That is
 * per object rather than per fragment, so two surfaces that interpenetrate
 * still resolve by whichever centre is farther — the ordinary approximation,
 * and the granularity a preview can afford.
 */
export function deferredDrawOrder(host: RenderHost): DeferredDraw[] {
  const draws: DeferredDraw[] = [];
  const centre = vec3.create();
  for (const renderable of host.scene!.renderables) {
    const meshIndex = renderable.meshIndex;
    const primitives = host.glResources!.primitives[meshIndex];
    if (!primitives || primitives.length === 0) continue;
    const box = host.scene!.meshes[meshIndex]?.aabb;
    let distance = 0;
    if (box) {
      vec3.set(
        centre,
        (box.min[0] + box.max[0]) / 2,
        (box.min[1] + box.max[1]) / 2,
        (box.min[2] + box.max[2]) / 2,
      );
      vec3.transformMat4(centre, centre, renderable.node.world);
      const eye = host._eye!;
      distance = Math.hypot(centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]);
    }
    for (let i = 0; i < primitives.length; i++) {
      const material = host.scene!.materials[primitives[i].materialIndex];
      if (!needsCompletedFrame(material)) continue;
      draws.push({
        renderable,
        primitiveIndex: i,
        distance,
        transmissive: (material?.transmissionFactor ?? 0) > 0,
      });
    }
  }
  return draws.sort((a, b) => b.distance - a.distance);
}

/**
 * Draw what waited, back to front, refreshing the copy transmission reads.
 *
 * A transmissive surface refracts a copy of the frame, and the frame has to
 * already hold everything behind it — including surfaces that blend rather
 * than write depth. Taking one copy after the opaque half, which is what this
 * used to do, leaves a blended surface out of every transmissive lookup:
 * TransmissionOrderTest's blended alpha simply vanished where the glass
 * covered it, while the masked and opaque rows showed through as they should.
 *
 * So the copy is retaken lazily, before the first transmissive draw that
 * follows a blended one. That costs a blit and a mip chain per hand-off, and
 * a scene alternating the two kinds in depth pays it once per alternation;
 * the ordinary scene, with its transmissive surfaces in one stretch of the
 * order, pays it exactly once.
 */
export function capturePoints(draws: readonly DeferredDraw[]): boolean[] {
  let stale = true;
  return draws.map((draw) => {
    if (!draw.transmissive) {
      stale = true;
      return false;
    }
    const take = stale;
    stale = false;
    return take;
  });
}

function drawDeferredSurfaces(host: RenderHost, scene: SceneTarget) {
  const draws = deferredDrawOrder(host);
  const captures = capturePoints(draws);
  draws.forEach((draw, index) => {
    if (captures[index]) {
      captureOpaqueHalf(host.gl, scene);
      // The copy did not exist when the last program was bound.
      host._surfaceProgram = null;
    }
    const jointMatrices = prepareRenderable(host, draw.renderable);
    drawPrimitive(host, draw.renderable, draw.primitiveIndex, jointMatrices);
  });
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
  const alpha = alphaModeUniforms(material);
  gl.uniform1f(host.uniforms.uAlphaCutoff, alpha.cutoff);
  gl.uniform1i(host.uniforms.uAlphaOpaque, alpha.opaque ? 1 : 0);

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

/**
 * The scene-referred light that comes out of the output pass as this colour.
 *
 * The grid is authored the way it should look, not as a quantity of light, and
 * everything now goes through a tone curve on the way to the canvas. Sending
 * the authored value as radiance would have the curve darken it, so it is run
 * backwards once: the tone curve is a quadratic in disguise, and inverting it
 * is the positive root. Luminance carries the whole transform, exactly as the
 * forward direction does, so the hue survives the round trip.
 */
function toneMapInverse(display: readonly number[]): [number, number, number] {
  const target = display.map((channel) => channel ** 2.2);
  const level = 0.2126 * target[0] + 0.7152 * target[1] + 0.0722 * target[2];
  if (level <= 0) return [0, 0, 0];
  const [a, b, c, d, e] = [2.51, 0.03, 2.43, 0.59, 0.14];
  const quadratic = a - level * c;
  const linear = b - level * d;
  const scene = (-linear + Math.sqrt(linear * linear + 4 * quadratic * level * e))
    / (2 * quadratic);
  const gain = scene / level;
  return [target[0] * gain, target[1] * gain, target[2] * gain];
}

/** The grid's own colour, as authored, and the light that reproduces it. */
const GRID_DISPLAY_COLOR = [0.31, 0.40, 0.56];
const GRID_RADIANCE = toneMapInverse(GRID_DISPLAY_COLOR);

export function drawGrid(host: RenderHost) {
  const gl = host.gl;
  mat4.multiply(host._projectionView, host._projection, host._view);
  if (!host._grid) buildSceneGrid(host);
  if (!host._grid) return;
  gl.useProgram(host.lineProgram);
  gl.uniformMatrix4fv(host.lineUniforms.uProjectionView, false, host._projectionView);
  gl.uniform3f(host.lineUniforms.uColor, ...GRID_RADIANCE);
  gl.bindBuffer(gl.ARRAY_BUFFER, host._grid.buffer);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
  gl.drawArrays(gl.LINES, 0, host._grid.count);
}

/**
 * Build the glare pyramid from the resolved frame.
 *
 * Down with a thirteen-tap filter, back up with a tent added into the level
 * above; the sum of levels is the long tail. The frame itself is level zero
 * only as the source - the pyramid starts half its size, so what this costs is
 * a third of a frame's worth of texels however many levels there are.
 */
export function drawGlare(host: RenderHost) {
  const gl = host.gl;
  const chain = host._bloom;
  const scene = host._sceneTarget;
  if (!chain || !scene || chain.levels.length === 0) return;
  gl.disable(gl.DEPTH_TEST);
  gl.depthMask(false);
  gl.disable(gl.BLEND);
  gl.bindVertexArray(host.backgroundVao);
  gl.activeTexture(gl.TEXTURE0 + SHARED_TEXTURE_UNITS.frameSnapshot);

  gl.useProgram(host.bloomDownProgram);
  gl.uniform1i(host.bloomDownUniforms.uSource, SHARED_TEXTURE_UNITS.frameSnapshot);
  let sourceTexture = scene.resolved;
  let sourceWidth = scene.renderWidth;
  let sourceHeight = scene.renderHeight;
  for (const level of chain.levels) {
    gl.bindFramebuffer(gl.FRAMEBUFFER, level.framebuffer);
    gl.viewport(0, 0, level.width, level.height);
    gl.bindTexture(gl.TEXTURE_2D, sourceTexture);
    gl.uniform2f(host.bloomDownUniforms.uTexel, 1 / sourceWidth, 1 / sourceHeight);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    sourceTexture = level.texture;
    sourceWidth = level.width;
    sourceHeight = level.height;
  }

  // Each level adds into the one above it, so the coarse tails accumulate
  // rather than replace what the finer levels already spread.
  gl.useProgram(host.bloomUpProgram);
  gl.uniform1i(host.bloomUpUniforms.uSource, SHARED_TEXTURE_UNITS.frameSnapshot);
  gl.enable(gl.BLEND);
  gl.blendFunc(gl.ONE, gl.ONE);
  for (let index = chain.levels.length - 1; index > 0; index -= 1) {
    const source = chain.levels[index];
    const destination = chain.levels[index - 1];
    gl.bindFramebuffer(gl.FRAMEBUFFER, destination.framebuffer);
    gl.viewport(0, 0, destination.width, destination.height);
    gl.bindTexture(gl.TEXTURE_2D, source.texture);
    gl.uniform2f(host.bloomUpUniforms.uTexel, 1 / source.width, 1 / source.height);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }
  gl.disable(gl.BLEND);
  gl.bindVertexArray(null);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  gl.enable(gl.DEPTH_TEST);
  gl.depthMask(true);
}

/**
 * The one pass that writes to the canvas: glare in, tone curve, transfer
 * function out.
 */
export function drawOutput(host: RenderHost) {
  const gl = host.gl;
  const scene = host._sceneTarget;
  const chain = host._bloom;
  if (!scene) return;
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
  gl.disable(gl.DEPTH_TEST);
  gl.depthMask(false);
  gl.disable(gl.BLEND);
  gl.useProgram(host.outputProgram);
  gl.activeTexture(gl.TEXTURE0 + SHARED_TEXTURE_UNITS.frameSnapshot);
  gl.bindTexture(gl.TEXTURE_2D, scene.resolved);
  gl.uniform1i(host.outputUniforms.uScene, SHARED_TEXTURE_UNITS.frameSnapshot);
  gl.activeTexture(gl.TEXTURE0 + SHARED_TEXTURE_UNITS.morphDeltas);
  gl.bindTexture(gl.TEXTURE_2D, chain?.levels[0]?.texture ?? scene.resolved);
  gl.uniform1i(host.outputUniforms.uBloom, SHARED_TEXTURE_UNITS.morphDeltas);
  gl.uniform1f(host.outputUniforms.uBloomStrength, host.bloomStrength ?? DEFAULT_BLOOM_STRENGTH);
  gl.uniform1f(host.outputUniforms.uExposure, host.exposure ?? 1);
  gl.uniform1f(host.outputUniforms.uSceneCrop, 1 / scene.guard);
  gl.uniform1i(host.outputUniforms.uToneMap, host.baseColorOnly ? 0 : 1);
  gl.bindVertexArray(host.backgroundVao);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  gl.bindVertexArray(null);
  gl.enable(gl.DEPTH_TEST);
  gl.depthMask(true);
  // Both units held something else; the next surface program re-states them.
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
  gl.bindVertexArray(host.backgroundVao);
  gl.drawArrays(gl.TRIANGLES, 0, 3);
  gl.bindVertexArray(null);
  gl.depthMask(true);
  gl.enable(gl.DEPTH_TEST);
}

/**
 * A one-texel stand-in for the opaque-half copy, bound before there is one.
 *
 * A sampler has to reference a complete texture even on the branch that never
 * reads it, and on the frame's first opaque pass the copy has not been taken.
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
  const scene = host._sceneTarget;
  gl.activeTexture(gl.TEXTURE0 + SHARED_TEXTURE_UNITS.frameSnapshot);
  gl.bindTexture(gl.TEXTURE_2D, scene ? scene.capture : snapshotPlaceholder(host));
  gl.uniform1i(host.uniforms.uFrameSnapshot, SHARED_TEXTURE_UNITS.frameSnapshot);
  if (!scene) return;
  gl.uniform2f(host.uniforms.uFrameSize, scene.renderWidth, scene.renderHeight);
  // The mip chain is the blur a rough transmissive surface reads through.
  gl.uniform1f(
    host.uniforms.uFrameMaxLod, Math.log2(Math.max(scene.renderWidth, scene.renderHeight)));
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
