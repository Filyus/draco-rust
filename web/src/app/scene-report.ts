import type { SceneCapabilities, SceneDocument } from '../scene-document.ts';
import { assertValidSceneDocument } from '../scene-document.ts';
import { sceneExtensionReach, exportSidebar, meshStatFields, sceneCapabilitySummary, sceneClipList, sceneInfo, sceneMaterialList, scenePanel, sceneResourceList, sceneSection, sceneStatFields, sceneTree, workspace } from './dom.ts';
import { errorMessage, log } from './log.ts';
import type { LoadedFile } from './state.ts';
import { setWarningSource } from './warnings.ts';
import { describeExtensionReach, reportExtensionReach } from './extension-report.ts';
import { state } from './state.ts';

/**
 * Everything the panels display about a loaded scene: the statistics row, the
 * capability summary, the node tree and the companion lists.
 *
 * Read-only with respect to the document; it renders what the importers
 * produced and never edits it.
 */

export function renderSceneDocumentSummary(sceneDocument: SceneDocument, extraWarnings = []) {
  if (!sceneDocument) {
    scenePanel.hidden = true;
    sceneInfo.hidden = true;
    setWarningSource('scene', []);
    sceneSection.style.display = 'none';
    workspace.classList.remove('scene-loaded');
    return;
  }
  try {
    const validation = assertValidSceneDocument(sceneDocument);
    const morphs = sceneDocument.meshes.reduce(
      (total, mesh) => total + mesh.primitives.reduce((count, primitive) => count + (primitive.targets?.length || 0), 0),
      0,
    );
    sceneStatFields.nodes.textContent = sceneDocument.nodes.length.toLocaleString();
    sceneStatFields.meshes.textContent = sceneDocument.meshes.length.toLocaleString();
    sceneStatFields.materials.textContent = sceneDocument.materials.length.toLocaleString();
    sceneStatFields.skins.textContent = sceneDocument.skins.length.toLocaleString();
    sceneStatFields.morphs.textContent = morphs.toLocaleString();
    sceneStatFields.clips.textContent = sceneDocument.animations.length.toLocaleString();
    sceneStatFields.lights.textContent = (sceneDocument.lights?.length ?? 0).toLocaleString();
    renderSceneTree(sceneDocument);
    renderSceneCompanions(sceneDocument);
    sceneSection.style.display = 'flex';
    workspace.classList.add('scene-loaded');
    sceneCapabilitySummary.textContent = describeSceneCapabilities(validation.capabilities);
    renderExtensionReach();
    setWarningSource('scene', [...sceneDocument.warnings, ...validation.warnings, ...extraWarnings]);
    scenePanel.hidden = false;
    sceneInfo.hidden = false;
  } catch (error) {
    scenePanel.hidden = true;
    sceneInfo.hidden = true;
    setWarningSource('scene', []);
    sceneSection.style.display = 'none';
    exportSidebar.style.display = 'none';
    workspace.classList.remove('scene-loaded');
    workspace.classList.remove('export-loaded');
    log(`Scene details unavailable: ${errorMessage(error)}`, 'warning');
  }
}

/**
 * What became of each extension the file declared.
 *
 * Only for glTF sources: the provenance is where the file's own claims
 * survive, and no other format makes any.
 */
function renderExtensionReach() {
  const lines = describeExtensionReach(reportExtensionReach(state.currentGltfProvenance));
  sceneExtensionReach.hidden = lines.length === 0;
  sceneExtensionReach.replaceChildren(...lines.map((line) => {
    const item = document.createElement('li');
    item.textContent = line;
    return item;
  }));
}

export function describeSceneCapabilities(capabilities: Partial<SceneCapabilities> = {}) {
  const preserved: string[] = [];
  if (capabilities.resources) preserved.push('resources');
  if (capabilities.textures) preserved.push('textures');
  if (capabilities.materials) preserved.push('materials');
  if (capabilities.skins) preserved.push('skins');
  if (capabilities.morphTargets) preserved.push('morph targets');
  if (capabilities.animations) preserved.push('animation clips');
  if (capabilities.cubicAnimation) preserved.push('cubic animation samples');
  return preserved.length > 0
    ? `Preserved in the shared scene model: ${preserved.join(', ')}.`
    : 'The shared scene model contains hierarchy and geometry data.';
}

export function renderSceneTree(sceneDocument: SceneDocument) {
  sceneTree.replaceChildren();
  if (sceneDocument.nodes.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'scene-tree-empty';
    empty.textContent = 'This loaded document has no scene nodes.';
    sceneTree.appendChild(empty);
    return;
  }
  const animatedNodes = new Set(sceneDocument.animations.flatMap((clip) => clip.channels.map((channel) => channel.node)));
  const appendNode = (nodeIndex: number, depth: number, visited: Set<number>, target: any) => {
    if (visited.has(nodeIndex)) return;
    visited.add(nodeIndex);
    const node = sceneDocument.nodes[nodeIndex] || {};
    const children = (node.children || []).filter((child) => Number.isInteger(child) && child >= 0 && child < sceneDocument.nodes.length);
    const branching = children.length > 0;
    const wrapper = branching
      ? document.createElement('details')
      : document.createElement('div') as HTMLElement as HTMLDetailsElement;
    wrapper.className = branching ? 'scene-tree-node' : 'scene-tree-leaf';
    if (branching) wrapper.open = true;
    const row = document.createElement(branching ? 'summary' : 'div');
    row.className = 'scene-tree-row';
    row.dataset.nodeIndex = String(nodeIndex);
    row.dataset.depth = String(depth);
    row.setAttribute('role', 'treeitem');
    // Nesting supplies the indentation, so rows no longer need inline padding math.
    const twisty = document.createElement('span');
    // Leaves get an invisible spacer instead of a control, so only real branches show a box.
    twisty.className = branching ? 'scene-tree-twisty' : 'scene-tree-twisty scene-tree-twisty-empty';
    twisty.setAttribute('aria-hidden', 'true');
    row.appendChild(twisty);
    const label = document.createElement('span');
    label.className = 'scene-tree-label';
    label.textContent = node.name || `Node ${nodeIndex}`;
    row.appendChild(label);
    const badges = document.createElement('span');
    badges.className = 'scene-tree-badges';
    const addBadge = (text: string, kind: string) => {
      const badge = document.createElement('span');
      badge.className = `scene-tree-badge scene-tree-badge-${kind}`;
      badge.textContent = text;
      badges.appendChild(badge);
    };
    if (node.mesh !== undefined) addBadge('mesh', 'mesh');
    if (node.skin !== undefined) addBadge('skin', 'skin');
    if (animatedNodes.has(nodeIndex)) addBadge('animated', 'animation');
    row.appendChild(badges);
    wrapper.appendChild(row);
    if (branching) {
      const childList = document.createElement('div');
      childList.className = 'scene-tree-children';
      childList.setAttribute('role', 'group');
      wrapper.appendChild(childList);
      children.forEach((child, position) => {
        const before = childList.childElementCount;
        appendNode(child, depth + 1, visited, childList);
        // Mark the visually last child so its guide line can stop at the elbow.
        if (childList.childElementCount > before && position === children.length - 1) {
          childList.lastElementChild!.classList.add('scene-tree-last');
        }
      });
    }
    target.appendChild(wrapper);
  };
  const visited = new Set<number>();
  for (const root of sceneDocument.rootNodes) appendNode(root, 0, visited, sceneTree);
  const orphans = document.createElement('div');
  orphans.className = 'scene-tree-orphans';
  sceneDocument.nodes.forEach((_, index: number) => appendNode(index, 0, visited, orphans));
  if (orphans.childElementCount > 0) {
    const heading = document.createElement('div');
    heading.className = 'scene-tree-orphans-title';
    heading.textContent = 'Detached nodes';
    sceneTree.append(heading, orphans);
  }
}

export function renderSceneCompanions(sceneDocument: SceneDocument) {
  const formatNames = (items: { name?: string }[], fallback: string) => {
    if (!items.length) return fallback;
    const names = items.map((item, index: number) => item.name || `${fallback} ${index + 1}`);
    return names.length > 3 ? `${names.slice(0, 3).join(', ')} +${names.length - 3}` : names.join(', ');
  };
  sceneResourceList.textContent = formatNames(sceneDocument.resources, 'none');
  sceneMaterialList.textContent = formatNames(sceneDocument.materials, 'none');
  sceneClipList.textContent = formatNames(sceneDocument.animations, 'none');
}

// Display mesh information
export function displayMeshInfo(result: LoadedFile) {
  if (result.document) {
    // The geometry figures are counted from the SceneDocument, so a file whose
    // document could not be built shows a dash rather than crashing the panel
    // that was about to explain why.
    const count = (value: unknown) => (typeof value === 'number' ? value.toLocaleString() : '—');
    meshStatFields.meshes.textContent = count(result.meshCount);
    meshStatFields.vertices.textContent = count(result.vertexCount);
    meshStatFields.triangles.textContent = count(result.triangleCount);
    meshStatFields.hasNormals.textContent = result.hasNormals ? 'Yes' : 'No';
    meshStatFields.hasUvs.textContent = result.hasUvs ? 'Yes' : 'No';
    setWarningSource('mesh', result.warnings || []);
    return;
  }
  const meshes = result.meshes || [];
  
  let totalVertices = 0;
  let totalTriangles = 0;
  let hasNormals = false;
  let hasUvs = false;
  
  for (const mesh of meshes) {
    totalVertices += (mesh.positions?.length || 0) / 3;
    totalTriangles += (mesh.indices?.length || 0) / 3;
    if ((mesh.normals?.length ?? 0) > 0) hasNormals = true;
    if ((mesh.uvs?.length ?? 0) > 0) hasUvs = true;
  }
  
  meshStatFields.meshes.textContent = String(meshes.length);
  meshStatFields.vertices.textContent = totalVertices.toLocaleString();
  meshStatFields.triangles.textContent = totalTriangles.toLocaleString();
  meshStatFields.hasNormals.textContent = hasNormals ? 'Yes' : 'No';
  meshStatFields.hasUvs.textContent = hasUvs ? 'Yes' : 'No';

  setWarningSource('mesh', result.warnings || []);
}
