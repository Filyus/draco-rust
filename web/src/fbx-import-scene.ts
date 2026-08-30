/**
 * Semantic FBX import boundary. It converts the FBX model tree, BindPose
 * skinning contract, materials, morphs, and animation takes into the shared
 * scene structure rendered by viewer.js. glTF intentionally does not import
 * this module.
 */

import { cloneTrs, decomposeMat4, identityMat4, identityTrs, invertMat4, multiplyMat4 } from './mat4.ts';
import { adaptFbxAnimation } from './fbx-animation-adapter.ts';
import { fbxSpace } from './fbx-space.ts';
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
    // Copied rather than referenced, and V turned on the way past. FBX puts V's
    // origin at the opposite end of the image from glTF, which the preview
    // consumes; without this a textured FBX showed mirrored against the very
    // GLB this application exports from it. A copy because the same mesh objects
    // are what the export path reads, and it turns V itself.
    flatMeshes.push(...(node.meshes || []).map((mesh: FbxJson) => (mesh.uvs?.length
      ? { ...mesh, uvs: flipUvV(mesh.uvs) }
      : mesh)));
    for (const child of node.children || []) collectMeshes(child);
  };
  for (const root of roots) collectMeshes(root);
  if (flatMeshes.length === 0) throw new Error('No meshes were decoded from this FBX scene');

  const scene = await buildSceneFromMeshes({ ...parsed, meshes: flatMeshes }, resources, hooks);
  const documentMaterials = applyFbxMaterials(scene, parsed, flatMeshes);
  expandFbxMorphTargets(scene, flatMeshes);
  // After the morph expansion, which writes to the mesh's first primitive and
  // is then shared by every primitive the split makes from it. Both steps read
  // a material index against the document's list, so neither applies when the
  // scene kept the per-mesh defaults instead.
  if (documentMaterials) splitFbxPrimitivesByMaterial(scene, flatMeshes);
  await applyFbxTextures(scene, parsed, resources, hooks);
  if (documentMaterials) dropColourLayer(scene);

  const { nodes, renderables, nodeById, nodeByName, rootIndices } = buildFbxNodes(roots);
  scene.nodes = nodes;
  scene.rootIndices = axisBasisRoots(nodes, rootIndices, parsed?.scene?.globalSettings);
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

/**
 * Roots re-parented under the file's declared axes, when they are not glTF's.
 *
 * An FBX states which way is up in `GlobalSettings`, and a Z-up file drawn as
 * though it were Y-up lies on its side. The whole change is one signed
 * permutation, so it goes on one node above the roots rather than into every
 * node's own transform: skinning, animation and the geometric offsets below all
 * stay in the file's own space, and the basis divides out of the joint palette
 * exactly as any other shared ancestor does. Unit scale is deliberately left
 * alone -- this preview works in the file's units.
 */
function axisBasisRoots(nodes: ViewerNode[], rootIndices: number[], settings: FbxJson): number[] {
  const space = fbxSpace(settings);
  if (space.identity) return rootIndices;
  const localMatrix = Float32Array.from(space.basis);
  const trs = decomposeMat4(localMatrix);
  const index = nodes.length;
  nodes.push({
    id: null,
    name: 'fbx_axis_basis',
    trs: cloneTrs(trs),
    restTrs: cloneTrs(trs),
    bindTrs: cloneTrs(trs),
    animationTrs: cloneTrs(trs),
    hasComplexTransformStack: false,
    usesAuthoredModelTrs: false,
    localMatrix,
    children: rootIndices,
    weights: new Float32Array(0),
    meshIndex: -1,
    skinIndex: -1,
    world: new Float32Array(16),
  } as ViewerNode);
  return [index];
}

/** V runs the other way in FBX, whichever direction it is being carried. */
function flipUvV(values: ArrayLike<number>): Float32Array {
  const output = new Float32Array(values.length);
  for (let index = 0; index < values.length; index += 1) {
    output[index] = index % 2 === 1 ? 1 - values[index] : values[index];
  }
  return output;
}

/**
 * The document material index a mesh's triangle draws with.
 *
 * Out of range falls back to the first material, which is what a mesh naming
 * no material at all gets.
 */
function materialIndexOf(named: unknown, count: number): number {
  const index = typeof named === 'number' ? named : 0;
  return index >= 0 && index < count ? index : 0;
}

/**
 * Carry the document's materials into the scene, indexed as the document has
 * them.
 *
 * They used to be rebuilt one per mesh, which put the scene list in a
 * different index space from `parsed.materials` -- and `applyFbxTextures`
 * still walked the two together by position, so a mesh was given whichever
 * material happened to sit at its own ordinal. The two lists share one index
 * space now, and a primitive names its material within it.
 */
function applyFbxMaterials(
  scene: ViewerSceneDraft,
  parsed: FbxJson,
  flatMeshes: FbxJson[],
): boolean {
  const fbxMaterials = parsed?.scene?.materials || parsed?.materials || [];
  // Nothing to index into: the scene keeps the per-mesh defaults it was built
  // with, and a material index means what it meant there.
  if (fbxMaterials.length === 0) return false;
  scene.materials = fbxMaterials.map(adaptFbxMaterial);
  scene.meshes.forEach((mesh: FbxJson, meshIndex: number) => {
    const material = materialIndexOf(flatMeshes[meshIndex]?.material, scene.materials.length);
    mesh.primitives.forEach((primitive: FbxJson) => { primitive.materialIndex = material; });
  });
  return true;
}

/**
 * Give each material of a multi-material mesh its own primitive.
 *
 * FBX assigns a material per polygon, and a character is usually one mesh with
 * a material for the face, one for the hair, one for each piece of clothing.
 * Drawing that mesh with a single material puts one texture over the whole
 * body: on one character, 92 311 of its 101 381 triangles were a single mesh
 * carrying ten materials, and nine of them never reached the screen.
 *
 * The split is of the index buffer alone. Every primitive keeps the mesh's
 * vertex attributes and morph targets, which are per vertex and shared.
 */
function splitFbxPrimitivesByMaterial(scene: ViewerSceneDraft, flatMeshes: FbxJson[]) {
  const materialCount = scene.materials.length;
  if (materialCount === 0) return;
  scene.meshes.forEach((mesh: FbxJson, meshIndex: number) => {
    const perTriangle = flatMeshes[meshIndex]?.materialIndices;
    const primitive = mesh.primitives?.[0];
    if (!primitive?.indices || !perTriangle || mesh.primitives.length !== 1) return;
    const indices = primitive.indices.bytes as { length: number; [index: number]: number };
    // The decoder aligns these with the fan-triangulated faces; anything else
    // is not a correspondence this can act on.
    if (perTriangle.length * 3 !== indices.length) return;
    const groups = new Map<number, number[]>();
    for (let triangle = 0; triangle < perTriangle.length; triangle++) {
      const material = materialIndexOf(perTriangle[triangle], materialCount);
      let group = groups.get(material);
      if (!group) groups.set(material, group = []);
      const corner = triangle * 3;
      group.push(indices[corner], indices[corner + 1], indices[corner + 2]);
    }
    if (groups.size <= 1) {
      // Still authoritative over the mesh-level `material`, which is only the
      // first one the decoder saw.
      for (const [material] of groups) primitive.materialIndex = material;
      return;
    }
    mesh.primitives = [...groups].map(([material, group]) => ({
      ...primitive,
      materialIndex: material,
      indices: {
        ...primitive.indices,
        bytes: Uint32Array.from(group),
        componentType: 5125,
        count: group.length,
      },
    }));
  });
}

/**
 * Keep the FBX colour layer out of the preview entirely.
 *
 * glTF's COLOR_0 multiplies the base colour, and an FBX colour layer is not
 * that. FBX materials have no vertex-colour term: the layer is data a shader
 * may sample, and a Phong material never says to. Engines export masks in it --
 * one character carried (1, 1, 0) across its body -- and the reference glTF
 * written from that same file keeps no COLOR_0 on any of its primitives, so
 * neither does the preview.
 *
 * The layer used to survive where the material states no texture, on the
 * reasoning that there it was the only surface colour there is. What such a
 * file carries there is still a mask: with the layer applied, that character's
 * face came out in patches of the mask's own purple and red.
 *
 * Binding it also brings on a secondary fault whose cause is not known, but
 * whose shape has been measured. Drawing the primitive that carries the layer
 * fills a screen-aligned region with the layer's own colours -- over the
 * background as well, which no per-vertex multiply can reach. The fill tracks
 * the data, not just the binding: a layer forced to all ones draws clean --
 * pixel for pixel the frame that dropping the layer gives, away from the model
 * itself -- and zeroing one channel of it recolours the wash accordingly, so
 * the corrupt pixels are painted out of the attribute's values. That is what
 * rules out the ordinary explanation: geometry misplaced by a bad vertex fetch
 * would be misplaced whatever colour it carried, and a white one would wash
 * the background grey rather than leave it untouched. Nor does the wash follow
 * the primitive's coverage -- squeezed to a full-width slit, its fragments
 * never reach the affected pixels, though the draw is still what summons them.
 * The attribute's values read back intact, so the fault sits between the
 * vertex fetch and the framebuffer, and removing the input hides it rather
 * than fixes it.
 *
 * The export path is untouched: it reads the layer from the parsed document,
 * where it is kept for the round trip.
 */
function dropColourLayer(scene: ViewerSceneDraft) {
  for (const mesh of scene.meshes as FbxJson[]) {
    for (const primitive of mesh.primitives as FbxJson[]) {
      if (!primitive.attributes?.COLOR_0) continue;
      // Cloned because the split primitives of one mesh share this object.
      const { COLOR_0: _dropped, ...rest } = primitive.attributes;
      primitive.attributes = rest;
    }
  }
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
  // Walked together by position because `applyFbxMaterials` builds the scene
  // list from this one, entry for entry. It is the same index space or none.
  for (let index = 0; index < scene.materials.length && index < materials.length; index++) {
    for (const binding of materials[index]?.textures || []) {
      if (!(binding.textureIndex in textures)) continue;
      const target = scene.materials[index];
      if (binding.slot === 'diffuse') {
        target.baseColorTexture = binding.textureIndex;
        // A texture connected to a property stands in for that property's
        // value; it does not modulate it. Keeping the constant as a factor
        // dimmed every textured surface by the exporter's default DiffuseColor,
        // which for Autodesk's own is 0.8. The alpha is a separate property and
        // stays.
        const alpha = target.baseColorFactor?.[3] ?? 1;
        target.baseColorFactor = [1, 1, 1, alpha];
      }
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
      ...(source.kind === 'joint' || source.kind === 'null' ? { kind: source.kind } : {}),
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
      const geometric = mesh?.geometricTransform?.matrix;
      renderables.push({
        node,
        meshIndex,
        skinIndex: -1,
        ...(geometric?.length === 16 ? { geometricMatrix: Float32Array.from(geometric) } : {}),
      });
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
