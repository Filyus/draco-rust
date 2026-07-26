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

export interface Renderable {
    node: ViewerNode;
    meshIndex: number;
    skinIndex: number;
}
