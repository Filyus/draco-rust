/**
 * The runtime scene the render engine consumes.
 *
 * Three importers build this shape — the SceneDocument adapter, the glTF
 * loader and the FBX path — and the viewer is the only consumer. Declaring it
 * once here is what keeps those producers honest; the fields that only one
 * importer fills are optional and say so.
 */

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
export interface ViewerMaterial {
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
  emissiveStrength?: number;
  emissiveTexture?: ViewerTextureBinding | null;
  normalTexture?: ViewerTextureBinding | null;
  occlusionTexture?: ViewerTextureBinding | null;
  ior?: number;
  specularFactor?: number;
  specularColorFactor?: number[];
  specularTexture?: ViewerTextureBinding | null;
  specularColorTexture?: ViewerTextureBinding | null;
  clearcoatFactor?: number;
  clearcoatRoughnessFactor?: number;
  clearcoatTexture?: ViewerTextureBinding | null;
  clearcoatRoughnessTexture?: ViewerTextureBinding | null;
  clearcoatNormalTexture?: ViewerTextureBinding | null;
  doubleSided?: boolean;
  alphaMode?: string;
  alphaCutoff?: number;
  unlit?: boolean;
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
export interface ViewerScene {
  nodes: ViewerNode[];
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
