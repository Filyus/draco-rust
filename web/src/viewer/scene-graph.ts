import { composeMatrix, mat4, vec3 } from '../math.ts';
import type { Mat4, Vec3 } from '../math.ts';
import type { ViewerNode, ViewerPrimitive, ViewerScene, ViewerSkin } from '../viewer-scene.ts';

/** What hierarchy evaluation reads and writes on the viewer. */
export interface SceneGraphHost {
  scene: ViewerScene | null;
  _visitedNodes?: Set<ViewerNode>;
  _boundsPoint?: Vec3;
  _boundsMatrix?: Mat4;
  _scratch: Mat4;
  _jointScratch?: Mat4;
}

/**
 * Scene hierarchy evaluation: world matrices, framing bounds and skin palettes.
 *
 * Free of WebGL, which is what lets the Node parity tests drive it directly.
 * The viewer keeps the scratch matrices and the visited set on itself, so this
 * reads and writes them through the host it is given.
 */

export function updateWorldMatrices(host: SceneGraphHost) {
  if (!host.scene) return;
  // Only ever reached from updateWorldMatrices, which returns early
  // without a scene.
  const nodes = host.scene!.nodes;
  const roots = host.scene.rootIndices || nodes.map((_: ViewerNode, i: number) => i);
  if (host._visitedNodes) host._visitedNodes.clear();
  else host._visitedNodes = new Set();
  for (const rootIndex of roots) {
    const node = nodes[rootIndex];
    if (node) updateNode(host, node, null);
  }
}

/** Recompute the framing bounds after node transforms have been applied. */
export function updateSceneBounds(host: SceneGraphHost) {
  if (!host.scene) return;
  const aabb = {
    min: [Infinity, Infinity, Infinity],
    max: [-Infinity, -Infinity, -Infinity],
  };
  const point = host._boundsPoint || (host._boundsPoint = vec3.create());
  const matrix = host._boundsMatrix || (host._boundsMatrix = mat4.create());
  const grow = (box: { min: number[]; max: number[] }, world: Mat4 | null) => {
    const { min, max } = box;
    for (const x of [min[0], max[0]]) {
      for (const y of [min[1], max[1]]) {
        for (const z of [min[2], max[2]]) {
          vec3.set(point, x, y, z);
          if (world) vec3.transformMat4(point, point, world);
          aabb.min[0] = Math.min(aabb.min[0], point[0]);
          aabb.min[1] = Math.min(aabb.min[1], point[1]);
          aabb.min[2] = Math.min(aabb.min[2], point[2]);
          aabb.max[0] = Math.max(aabb.max[0], point[0]);
          aabb.max[1] = Math.max(aabb.max[1], point[1]);
          aabb.max[2] = Math.max(aabb.max[2], point[2]);
        }
      }
    }
  };
  for (const renderable of host.scene.renderables || []) {
    const mesh = host.scene.meshes[renderable.meshIndex];
    const meshBox = mesh?.aabb;
    if (!meshBox) continue;
    const skin = renderable.skinIndex >= 0
      ? host.scene.skins[renderable.skinIndex]
      : null;
    const skinned = !!skin?.joints.length && meshIsSkinned(mesh.primitives);
    if (!skinned) {
      grow(meshBox, renderable.node.world);
      continue;
    }
    // A skinned vertex lands at jointWorld * IBM * position: the palette is
    // inverse(mesh world) * jointWorld * IBM and the shader multiplies by the
    // mesh world again, so the node's own transform divides out. What is left
    // is not the identity, though, and assuming it was is what left Soldier.glb
    // invisible — mesh and armature alike sit under one 0.01-scaled root, so it
    // renders 1.8 units tall while its bind pose measures 183, and the camera
    // was framed a hundred times too far out.
    //
    // Every vertex is a convex blend of its joints' results, so the union of
    // the bind box under each jointWorld * IBM contains all of them. Loose when
    // a distant joint drags the whole box along with it, but framing is a
    // camera fit, and loose beats wrong by two orders of magnitude.
    for (const joint of skin!.joints) {
      if (!joint?.node) continue;
      if (joint.inverseBind) mat4.multiply(matrix, joint.node.world, joint.inverseBind);
      else mat4.copy(matrix, joint.node.world);
      grow(meshBox, matrix);
    }
  }
  if (isFinite(aabb.min[0])) host.scene.aabb = aabb;
}

/**
 * Whether the draw will actually skin: the renderer falls back to the node
 * transform for a primitive that carries no influences, and the bounds have to
 * agree with whichever path runs.
 */
function meshIsSkinned(primitives: ViewerPrimitive[] | undefined) {
  return (primitives || []).some(
    (primitive) => primitive.attributes?.JOINTS_0 && primitive.attributes?.WEIGHTS_0,
  );
}

export function updateNode(host: SceneGraphHost, node: ViewerNode, parentWorld: Mat4 | null) {
  if (!node || !node.trs) return;
  const world = node.world;
  if (node.localMatrix) mat4.copy(world, node.localMatrix);
  else composeMatrix(world, node.trs.translation, node.trs.rotation, node.trs.scale);
  if (parentWorld) {
    mat4.multiply(world, parentWorld, world);
  }
  // Only ever reached from updateWorldMatrices, which returns early
  // without a scene.
  const nodes = host.scene!.nodes;
  const children = node.children || [];
  const visited = host._visitedNodes || (host._visitedNodes = new Set());
  visited.add(node);
  for (const child of children) {
    // glTF stores child references as node indices; mesh-loader uses
    // direct node objects. Support both.
    const childNode = typeof child === 'number' ? nodes[child] : child;
    if (childNode && childNode !== node && !visited.has(childNode)) {
      updateNode(host, childNode, world);
    }
  }
}

export function computeJointMatrices(
  host: SceneGraphHost,
  skin: ViewerSkin | null,
  meshWorld: Mat4,
  jointOut: Float32Array | null,
): Float32Array | null {
  if (!skin || !jointOut || !mat4.invert(host._scratch, meshWorld)) return null;
  const inverseMeshWorld = host._scratch;
  const tmp = host._jointScratch || (host._jointScratch = mat4.create());
  const count = jointOut.length / 16;
  for (let i = 0; i < count; i++) {
    const joint = skin.joints[i];
    if (!joint?.node) return null;
    // In glTF, the palette is inverse(mesh world) * joint world * IBM.
    mat4.multiply(tmp, inverseMeshWorld, joint.node.world);
    mat4.multiply(jointOut.subarray(i * 16, (i + 1) * 16), tmp, joint.inverseBind);
  }
  return jointOut;
}
