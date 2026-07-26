/**
 * Semantic FBX import boundary. It converts the FBX model tree, BindPose
 * skinning contract, materials, morphs, and animation takes into the shared
 * scene structure rendered by viewer.js. glTF intentionally does not import
 * this module.
 */

import { cloneTrs, decomposeMat4, identityMat4, identityTrs, invertMat4, multiplyMat4 } from './mat4.ts';
import { adaptFbxAnimation } from './fbx-animation-adapter.ts';
import { adaptFbxMaterial, adaptFbxTextures } from './fbx-material-adapter.ts';
import type { ResourceMap } from './scene-resources.ts';
import type { Renderable, Trs, ViewerNode, ViewerSkin } from './viewer-scene.ts';

/** The semantic FBX tree, walked node by node rather than trusted wholesale. */
type FbxJson = any;

/** The runtime scene, still being assembled from several importers. */
type ViewerSceneDraft = any;

/** Diagnostics sink shared with the rest of the import path. */
interface ImportHooks {
  onLog?: (message: string, level: string) => void;
}

export async function buildSceneFromFbx(
  parsed: FbxJson,
  resources: ResourceMap,
  hooks: ImportHooks,
  buildSceneFromMeshes: (parsed: FbxJson, resources: ResourceMap, hooks: ImportHooks) => Promise<ViewerSceneDraft>,
): Promise<ViewerSceneDraft> {
  const roots = parsed?.scene?.rootNodes;
  if (!Array.isArray(roots) || roots.length === 0) {
    return buildSceneFromMeshes(parsed, resources, hooks);
  }

  const flatMeshes: FbxJson[] = [];
  const collectMeshes = (node: FbxJson) => {
    flatMeshes.push(...(node.meshes || []));
    for (const child of node.children || []) collectMeshes(child);
  };
  for (const root of roots) collectMeshes(root);
  if (flatMeshes.length === 0) throw new Error('No meshes were decoded from this FBX scene');

  const scene = await buildSceneFromMeshes({ ...parsed, meshes: flatMeshes }, resources, hooks);
  applyFbxMaterials(scene, parsed, flatMeshes);
  expandFbxMorphTargets(scene, flatMeshes);
  await applyFbxTextures(scene, parsed, resources, hooks);

  const { nodes, renderables, nodeById, nodeByName, rootIndices } = buildFbxNodes(roots);
  scene.nodes = nodes;
  scene.rootIndices = rootIndices;
  scene.renderables = renderables;
  scene.skins = attachFbxSkins(roots, renderables, nodeById);

  const animations = parsed?.scene?.animations || parsed?.animations || [];
  if (animations.length > 0) {
    scene.animations = animations
      .map((clip: FbxJson) => adaptFbxAnimation(clip, nodeById, nodeByName))
      .filter(Boolean);
  }
  return scene;
}

function applyFbxMaterials(scene: ViewerSceneDraft, parsed: FbxJson, flatMeshes: FbxJson[]) {
  const fbxMaterials = parsed?.scene?.materials || parsed?.materials || [];
  if (fbxMaterials.length === 0) return;
  const converted = fbxMaterials.map(adaptFbxMaterial);
  scene.materials = scene.meshes.map((mesh: FbxJson, meshIndex: number) => {
    const materialIndex = typeof flatMeshes[meshIndex]?.material === 'number'
      ? flatMeshes[meshIndex].material : 0;
    return converted[materialIndex] || converted[0] || mesh._defaultMaterial;
  });
  scene.meshes.forEach((mesh: FbxJson, meshIndex: number) => {
    mesh.primitives.forEach((primitive: FbxJson) => { primitive.materialIndex = meshIndex; });
  });
}

function expandFbxMorphTargets(scene: ViewerSceneDraft, flatMeshes: FbxJson[]) {
  // FBX morphs are control-point sparse while WebGL uses expanded render
  // vertices; retain source sparsity for FBX export and build dense preview
  // attributes only at this format boundary.
  for (let meshIndex = 0; meshIndex < scene.meshes.length; meshIndex++) {
    const sourceMesh = flatMeshes[meshIndex];
    const primitive = scene.meshes[meshIndex]?.primitives?.[0];
    if (!primitive || !sourceMesh?.morphTargets?.length) continue;
    primitive.morphPositions = [];
    primitive.morphNormals = [];
    const vertexCount = (sourceMesh.positions?.length || 0) / 3;
    for (const target of sourceMesh.morphTargets) {
      const position = new Float32Array(vertexCount * 3);
      const renderIndices = target.renderPointIndices || [];
      const renderDeltas = target.renderPositionDeltas || [];
      for (let entry = 0; entry < renderIndices.length; entry++) {
        const render = renderIndices[entry] * 3;
        const delta = entry * 3;
        if (render + 2 >= position.length || delta + 2 >= renderDeltas.length) continue;
        position[render] = renderDeltas[delta] || 0;
        position[render + 1] = renderDeltas[delta + 1] || 0;
        position[render + 2] = renderDeltas[delta + 2] || 0;
      }
      primitive.morphPositions.push({ bytes: position, componentType: 5126, components: 3, normalized: false, count: vertexCount });
      const normalDeltas = target.renderNormalDeltas;
      if (!normalDeltas?.length) {
        primitive.morphNormals.push(null);
        continue;
      }
      const normal = new Float32Array(vertexCount * 3);
      for (let entry = 0; entry < renderIndices.length; entry++) {
        const render = renderIndices[entry] * 3;
        const delta = entry * 3;
        if (render + 2 >= normal.length || delta + 2 >= normalDeltas.length) continue;
        normal[render] = normalDeltas[delta] || 0;
        normal[render + 1] = normalDeltas[delta + 1] || 0;
        normal[render + 2] = normalDeltas[delta + 2] || 0;
      }
      primitive.morphNormals.push({ bytes: normal, componentType: 5126, components: 3, normalized: false, count: vertexCount });
    }
  }
}

async function applyFbxTextures(
  scene: ViewerSceneDraft,
  parsed: FbxJson,
  resources: ResourceMap,
  hooks: ImportHooks,
) {
  const sourceTextures = parsed?.scene?.textures || parsed?.textures || [];
  if (sourceTextures.length === 0) return;
  const warnings = parsed.warnings || (parsed.warnings = []);
  const textures = await adaptFbxTextures(sourceTextures, resources, warnings, hooks);
  if (textures.length === 0) return;
  scene.textures = textures;
  const materials = parsed?.scene?.materials || parsed?.materials || [];
  for (let index = 0; index < scene.materials.length && index < materials.length; index++) {
    for (const binding of materials[index]?.textures || []) {
      if (!(binding.textureIndex in textures)) continue;
      const target = scene.materials[index];
      if (binding.slot === 'diffuse') target.baseColorTexture = binding.textureIndex;
      else if (binding.slot === 'normal') target.normalTexture = { index: binding.textureIndex };
      else if (binding.slot === 'emissive') target.emissiveTexture = { index: binding.textureIndex };
      else if (binding.slot === 'roughness' || binding.slot === 'metallic') target.metallicRoughnessTexture = { index: binding.textureIndex };
    }
  }
}

function buildFbxNodes(roots: FbxJson[]) {
  const bindPoseByNodeId = new Map<number, number[]>();
  const collectBindPoses = (source: FbxJson) => {
    for (const mesh of source.meshes || []) {
      for (const entry of mesh.skin?.bindPose || []) {
        if (typeof entry?.nodeId === 'number' && Array.isArray(entry.matrix)
          && entry.matrix.length === 16 && !bindPoseByNodeId.has(entry.nodeId)) {
          bindPoseByNodeId.set(entry.nodeId, entry.matrix);
        }
      }
    }
    for (const child of source.children || []) collectBindPoses(child);
  };
  roots.forEach(collectBindPoses);

  const nodes: ViewerNode[] = [];
  const renderables: Renderable[] = [];
  const nodeById = new Map<unknown, ViewerNode>();
  const nodeByName = new Map<string, ViewerNode>();
  let meshIndex = 0;
  const appendNode = (source: FbxJson, parentBindMatrix: number[] | null = null): number => {
    const nodeId = typeof source.id === 'number' ? source.id : null;
    const bindMatrix = nodeId === null ? null : bindPoseByNodeId.get(nodeId);
    const sourceMatrix = Array.isArray(source.matrix) && source.matrix.length === 16 ? source.matrix : null;
    const localMatrix = bindMatrix
      ? Float32Array.from(parentBindMatrix ? (multiplyMat4(invertMat4(parentBindMatrix), bindMatrix) || bindMatrix) : bindMatrix)
      : sourceMatrix ? Float32Array.from(sourceMatrix) : null;
    // BindPose world matrices are authoritative for static skin
    // placement. Animated Lcl properties use the Model's own static
    // transform basis when it is ordinary TRS. For nodes with FBX
    // pre/post rotation or pivot terms, the BindPose local is the
    // equivalent baked basis emitted by the decoder; it preserves the
    // existing Mixamo convention without applying that correction to
    // plain-TRS rigs such as Samba Dancing.
    const bindTrs = localMatrix ? decomposeMat4(localMatrix) : identityTrs();
    const animationTrs = sourceMatrix && !source.hasComplexTransformStack
      ? decomposeMat4(sourceMatrix)
      : cloneTrs(bindTrs);
    const usesAuthoredModelTrs = Boolean(sourceMatrix && !source.hasComplexTransformStack);
    const nodeIndex = nodes.length;
    const node: ViewerNode = {
      id: nodeId,
      name: source.name || `node_${nodes.length}`,
      trs: cloneTrs(bindTrs),
      restTrs: cloneTrs(bindTrs),
      bindTrs,
      animationTrs,
      hasComplexTransformStack: Boolean(source.hasComplexTransformStack),
      usesAuthoredModelTrs,
      localMatrix,
      children: [],
      weights: Float32Array.from((source.meshes?.[0]?.morphTargets || []).map((target: FbxJson) => (Number(target.defaultWeight) || 0) / 100)),
      meshIndex: -1,
      skinIndex: -1,
      world: new Float32Array(16),
    };
    nodes.push(node);
    if (source.name) nodeByName.set(source.name, node);
    if (nodeId !== null) nodeById.set(nodeId, node);
    for (const mesh of source.meshes || []) {
      renderables.push({ node, meshIndex, skinIndex: -1 });
      if (node.meshIndex < 0) node.meshIndex = meshIndex;
      meshIndex += 1;
    }
    node.children = (source.children || []).map((child: FbxJson) => appendNode(child, bindMatrix || parentBindMatrix));
    return nodeIndex;
  };
  const rootIndices = roots.map((root: FbxJson) => appendNode(root));
  return { nodes, renderables, nodeById, nodeByName, rootIndices };
}

function attachFbxSkins(
  roots: FbxJson[],
  renderables: Renderable[],
  nodeById: Map<unknown, ViewerNode>,
): ViewerSkin[] {
  const skins: ViewerSkin[] = [];
  let flatMeshIndex = 0;
  const attach = (source: FbxJson, ownerNode: ViewerNode | undefined) => {
    for (const sourceMesh of source.meshes || []) {
      if (sourceMesh.skin?.clusters?.length) {
        const bindPose = new Map<unknown, number[]>((sourceMesh.skin.bindPose || []).map((entry: FbxJson) => [entry.nodeId, entry.matrix]));
        const joints = sourceMesh.skin.clusters.map((cluster: FbxJson) => {
          const meshBind = bindPose.get(ownerNode?.id) || cluster.meshBindTransform || identityMat4();
          const jointNode = nodeById.get(cluster.jointNodeId);
          // For plain-TRS bones the Cluster TransformLink is the
          // authored bone rest matrix. FBX BindPose entries may
          // carry an exporter axis conversion instead (Samba
          // Dancing's toe/arm bones do). Nodes with a pre/post or
          // pivot stack retain the BindPose as their baked basis,
          // which is the established Mixamo path.
          const jointBind = jointNode?.hasComplexTransformStack
            ? (bindPose.get(cluster.jointNodeId) || cluster.jointBindTransform || identityMat4())
            : (cluster.jointBindTransform || bindPose.get(cluster.jointNodeId) || identityMat4());
          const inverseJointBind = invertMat4(jointBind) || identityMat4();
          return {
            node: nodeById.get(cluster.jointNodeId),
            inverseBind: Float32Array.from(multiplyMat4(inverseJointBind, meshBind) || inverseJointBind),
          };
        });
        if (joints.every((joint: FbxJson) => joint.node)) {
          const skinIndex = skins.length;
          skins.push({ name: `${sourceMesh.name || 'mesh'}_skin`, joints });
          renderables[flatMeshIndex].skinIndex = skinIndex;
        }
      }
      flatMeshIndex += 1;
    }
    for (const child of source.children || []) attach(child, nodeById.get(child.id));
  };
  roots.forEach((root: FbxJson) => attach(root, nodeById.get(root.id)));
  return skins;
}
