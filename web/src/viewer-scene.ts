/**
 * The runtime scene the render engine consumes.
 *
 * Three importers build this shape — the SceneDocument adapter, the glTF
 * loader and the FBX path — and the viewer is the only consumer. Declaring it
 * once here is what keeps those producers honest; the fields that only one
 * importer fills are optional and say so.
 */

import type { MaterialExtensionValues } from './material-extensions.ts';

export interface Trs {
  translation: number[];
  rotation: number[];
  scale: number[];
}

/**
 * An attribute or index buffer, still in its source component encoding.
 *
 * `bytes` is usually a Uint8Array, but the FBX morph path builds dense float
 * deltas directly and hands those over; the viewer normalizes both through
 * byteView before upload.
 */
export interface RuntimeAccessor {
  bytes: ArrayBufferView;
  componentType: number;
  components: number;
  normalized: boolean;
  count: number;
}

export interface ViewerNode {
  name: string;
  trs: Trs;
  children: number[];
  /** Absent on flat OBJ/PLY meshes, which have no source transform at all. */
  localMatrix?: Float32Array | null;
  /** Absent wherever the importer knows the node carries no morph weights. */
  weights?: Float32Array | number[];
  meshIndex: number;
  skinIndex: number;
  world: Float32Array;
  /** FBX only: the bind-rest pose the animation adapter rebases against. */
  restTrs?: Trs;
  /** FBX only: the static basis authored rotation keys compose with. */
  animationTrs?: Trs;
  /** FBX only: set when Model TRS keys are already in their local space. */
  usesAuthoredModelTrs?: boolean;
  /** FBX only: the source object id, which skin clusters reference. */
  id?: number | null;
  /** FBX only: the BindPose-derived basis the rest pose was built from. */
  bindTrs?: Trs;
  /** FBX only: set for nodes carrying pre/post rotation or pivot terms. */
  hasComplexTransformStack?: boolean;
  /** glTF loader only: the node's own index in the source document. */
  index?: number;
}

/**
 * What the FBX animation adapter actually reads off a node.
 *
 * Two different node representations flow through it: the runtime node below,
 * and the SceneDocument importer's own per-node state, which has no world
 * matrix because nothing ever renders it. This is their common ground.
 */
export interface AnimationTarget {
  weights?: Float32Array | number[];
  restTrs?: Trs;
  animationTrs?: Trs;
  usesAuthoredModelTrs?: boolean;
}

export interface ViewerPrimitive {
  attributes: Record<string, RuntimeAccessor>;
  mode: number;
  materialIndex: number;
  indices?: RuntimeAccessor;
  morphPositions?: (RuntimeAccessor | null)[];
  morphNormals?: (RuntimeAccessor | null)[];
}

export interface Aabb {
  min: number[];
  max: number[];
}

export interface ViewerMesh {
  name: string;
  primitives: ViewerPrimitive[];
  aabb: Aabb;
  /** Mesh-level morph defaults, when the source document carried them. */
  weights?: number[];
}

export interface ViewerJoint {
  node: ViewerNode;
  inverseBind: Float32Array;
}

export interface ViewerSkin {
  name: string;
  joints: ViewerJoint[];
}

export type AnimationPath = 'translation' | 'rotation' | 'scale' | 'weights';

export interface ViewerSampler {
  input: Float32Array;
  output: Float32Array;
  interpolation: string;
}

export interface ViewerChannel {
  node: ViewerNode;
  path: AnimationPath;
  /** Component count per key: 3 for TRS, the morph count for weights. */
  targetCount: number;
  sampler: ViewerSampler;
}

export interface ViewerClip {
  name: string;
  duration: number;
  channels: ViewerChannel[];
}

/**
 * A texture binding on a material slot.
 *
 * Indices are into `ViewerScene.textures`. `scale` belongs to a normal map and
 * `strength` to occlusion; `transform` is `KHR_texture_transform`, and the
 * renderer applies it on every slot through its shared slot table.
 */
export interface ViewerTextureBinding {
  index: number;
  texCoord: number;
  transform?: { offset: number[]; scale: number[]; rotation: number; texCoord?: number };
  scale?: number;
  strength?: number;
}

/**
 * What the renderer may read off a material.
 *
 * Every field is optional because the OBJ, PLY and FBX importers fill only the
 * few they have, and `applyMaterial` defaults each one. The list matters
 * anyway: it is the contract the glTF loader and the SceneDocument adapter both
 * have to satisfy, and the one place to look for what the preview understands.
 */
export interface ViewerMaterial extends MaterialExtensionValues<ViewerTextureBinding | null> {
  name?: string;
  baseColorFactor?: number[];
  /** Flattened: the renderer addresses base color through its own uniforms. */
  baseColorTexture?: number | null;
  baseColorTexCoord?: number;
  baseColorTextureTransform?: { offset: number[]; scale: number[]; rotation: number };
  metallic?: number;
  roughness?: number;
  metallicRoughnessTexture?: ViewerTextureBinding | null;
  emissiveFactor?: number[];
  emissiveTexture?: ViewerTextureBinding | null;
  normalTexture?: ViewerTextureBinding | null;
  occlusionTexture?: ViewerTextureBinding | null;
  doubleSided?: boolean;
  alphaMode?: string;
  alphaCutoff?: number;
  /** OBJ only: the companion file a texture still has to be resolved from. */
  baseColorTextureUri?: string;
}

/**
 * A texture as the scene carries it before upload.
 *
 * The glTF and OBJ paths decode in the browser and arrive with `image`; the
 * SceneDocument adapter stays DOM-free and hands over the source bytes for a
 * hydration step to decode.
 */
export interface ViewerTexture {
  name?: string;
  image?: ImageBitmap | HTMLImageElement | null;
  bytes?: Uint8Array;
  mimeType?: string;
  resource?: number;
  flipY?: boolean;
  wrapS?: number;
  wrapT?: number;
  minFilter?: number;
  magFilter?: number;
}

/** The whole runtime scene, as the viewer holds it between frames. */
/**
 * A punctual light the renderer can shade with, already placed.
 *
 * The node that carries it is kept rather than its world matrix, because the
 * matrix changes every frame an animation touches that node and the renderer
 * reads it there anyway.
 */
export interface ViewerLight {
  type: 'directional' | 'point' | 'spot';
  node: ViewerNode;
  color: number[];
  intensity: number;
  range: number;
  innerConeAngle: number;
  outerConeAngle: number;
}

export interface ViewerScene {
  nodes: ViewerNode[];
  /** Placed punctual lights; empty for a scene that declares none. */
  lights?: ViewerLight[];
  /** Named material variants the source offered, for a consumer to choose from. */
  variants?: string[];
  rootIndices: number[];
  meshes: ViewerMesh[];
  skins: ViewerSkin[];
  materials: ViewerMaterial[];
  textures: ViewerTexture[];
  animations: ViewerClip[];
  renderables: Renderable[];
  aabb: Aabb;
  warnings: string[];
}

export interface Renderable {
  node: ViewerNode;
  meshIndex: number;
  skinIndex: number;
}

/**
 * Morph targets the preview can blend in one frame.
 *
 * Mirrors the viewer's shader loop bound; a mesh may declare any number of
 * targets as long as no single frame drives more than this many at once.
 */
export const MAX_ACTIVE_MORPH_TARGETS = 32;

/**
 * What a producer of this scene says when the renderer cannot show something.
 *
 * These are statements about the preview, not about the file, so they belong
 * with the shape the preview consumes rather than with any one importer. Two
 * importers reach the same renderer through the same limits, and a user who
 * loads one asset through both must not be told two different things about it.
 */
export const VIEWER_LIMIT_WARNINGS = {
  morphTarget: (target: number, mesh: number, primitive: number) =>
    `Morph target ${target} on mesh ${mesh} primitive ${primitive} has an unsupported POSITION accessor and was ignored`,
  morphNormal: (target: number, mesh: number, primitive: number) =>
    `Morph normal ${target} on mesh ${mesh} primitive ${primitive} has an unsupported accessor and was ignored`,
  morphTangents: (mesh: number, primitive: number) =>
    `Morph tangents on mesh ${mesh} primitive ${primitive} are ignored because the preview derives its tangent frame from deformed geometry and UVs`,
  morphMeshWeights: (mesh: string, active: number) =>
    `Morph mesh ${mesh} holds ${active} non-zero weights; the preview blends the ${MAX_ACTIVE_MORPH_TARGETS} strongest`,
  morphKeyframeWeights: (animation: string, active: number) =>
    `Animation ${animation}: a weights keyframe drives ${active} targets at once; the preview blends the ${MAX_ACTIVE_MORPH_TARGETS} strongest`,
} as const;

/**
 * Most morph weights a weights sampler ever holds at once.
 *
 * Cubic keyframes store [inTangent, value, outTangent], so only their middle
 * block is a pose, and an interpolated segment can carry both of its endpoint
 * poses at the same time.
 */
export function peakActiveMorphWeights(
  sampler: { output: ArrayLike<number>; interpolation?: string },
  targetCount: number,
): number {
  if (targetCount <= 0 || !sampler.output) return 0;
  const cubic = String(sampler.interpolation || 'LINEAR').toUpperCase() === 'CUBICSPLINE';
  const stride = cubic ? targetCount * 3 : targetCount;
  const offset = cubic ? targetCount : 0;
  const keyframes = Math.floor(sampler.output.length / stride);
  const poses: number[][] = [];
  for (let key = 0; key < keyframes; key += 1) {
    const pose: number[] = [];
    for (let target = 0; target < targetCount; target += 1) {
      if (sampler.output[key * stride + offset + target]) pose.push(target);
    }
    poses.push(pose);
  }
  const blends = String(sampler.interpolation || 'LINEAR').toUpperCase() !== 'STEP';
  let peak = 0;
  for (let key = 0; key < poses.length; key += 1) {
    const segment = blends && key + 1 < poses.length
      ? new Set([...poses[key], ...poses[key + 1]])
      : new Set(poses[key]);
    if (segment.size > peak) peak = segment.size;
  }
  return peak;
}
