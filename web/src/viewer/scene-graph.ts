import { composeMatrix, mat4, vec3 } from '../math.ts';
import type { Mat4, Vec3 } from '../math.ts';
import type { ViewerNode, ViewerScene, ViewerSkin } from '../viewer-scene.ts';

/** What hierarchy evaluation reads and writes on the viewer. */
export interface SceneGraphHost {
  scene: ViewerScene | null;
  _visitedNodes?: Set<ViewerNode>;
  _boundsPoint?: Vec3;
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
  for (const renderable of host.scene.renderables || []) {
    const meshBox = host.scene.meshes[renderable.meshIndex]?.aabb;
    if (!meshBox) continue;
    const { min, max } = meshBox;
    for (const x of [min[0], max[0]]) {
      for (const y of [min[1], max[1]]) {
        for (const z of [min[2], max[2]]) {
          vec3.set(point, x, y, z);
          vec3.transformMat4(point, point, renderable.node.world);
          aabb.min[0] = Math.min(aabb.min[0], point[0]);
          aabb.min[1] = Math.min(aabb.min[1], point[1]);
          aabb.min[2] = Math.min(aabb.min[2], point[2]);
          aabb.max[0] = Math.max(aabb.max[0], point[0]);
          aabb.max[1] = Math.max(aabb.max[1], point[1]);
          aabb.max[2] = Math.max(aabb.max[2], point[2]);
        }
      }
    }
  }
  if (isFinite(aabb.min[0])) host.scene.aabb = aabb;
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
