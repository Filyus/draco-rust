/**
 * Every element the shell addresses, resolved once at module load.
 *
 * index.html owns these ids, so the app has always treated them as present;
 * `element` states that instead of threading a null check through each of
 * eighty-odd lookups. Genuinely optional elements keep their guards at the
 * point of use.
 */

/**
 * Resolve an element this page owns.
 *
 * index.html declares every id used below, so the app has always treated them
 * as present; this states that instead of threading a null check through each
 * of eighty-odd lookups. Genuinely optional elements keep their explicit
 * guards at the point of use.
 */
export function element<T extends HTMLElement = HTMLElement>(id: string): T {
  return document.getElementById(id) as T;
}

export function query<T extends HTMLElement = HTMLElement>(selector: string): T {
  return document.querySelector(selector) as T;
}

// DOM Elements
export const dropZone = element('drop-zone');
export const fileInput = element<HTMLInputElement>('file-input');
export const fileInfo = element('file-info');
export const fileName = element('file-name');
export const fileSize = element('file-size');
export const clearFileBtn = element<HTMLButtonElement>('clear-file');
export const previewSection = element('preview-section');
export const exportSection = element('export-section');
export const exportFormat = element<HTMLSelectElement>('export-format');
export const useDraco = element<HTMLInputElement>('use-draco');
export const useDracoLabel = element('use-draco-label');
export const dracoOptions = element('draco-options');
export const dracoSettings = element('draco-settings');
export const encodingSpeed = element<HTMLSelectElement>('encoding-speed');
export const encodingMethod = element<HTMLSelectElement>('encoding-method');
export const positionBits = element<HTMLInputElement>('position-bits');
export const normalBits = element<HTMLInputElement>('normal-bits');
export const texcoordBits = element<HTMLInputElement>('texcoord-bits');
export const exportBtn = element<HTMLButtonElement>('export-btn');
export const consoleEl = element('console');

// 3D preview DOM references
export const viewerSection = element('viewer-section');
export const viewerCanvas = element<HTMLCanvasElement>('viewer-canvas');
export const viewerResetBtn = element<HTMLButtonElement>('viewer-reset');
export const viewerAutoRotateBtn = element<HTMLButtonElement>('viewer-autorotate');
export const viewerWireframeBtn = element<HTMLButtonElement>('viewer-wireframe');
export const viewerBaseColorBtn = element<HTMLButtonElement>('viewer-base-color');
export const viewerSmoothNormalsBtn = element<HTMLButtonElement>('viewer-smooth-normals');
export const viewerGridBtn = element<HTMLButtonElement>('viewer-grid');
export const viewerAnimation = element('viewer-animation');
export const animPlayBtn = element<HTMLButtonElement>('anim-play');
export const animClipSelect = element<HTMLSelectElement>('anim-clip');
export const animClipTrigger = element<HTMLButtonElement>('anim-clip-trigger');
export const animClipLabel = element('anim-clip-label');
export const animClipMenu = element('anim-clip-menu');
export const animLoopCheckbox = element<HTMLInputElement>('anim-loop');
export const animTimeLabel = element('anim-time');
export const animScrub = element<HTMLInputElement>('anim-scrub');
export const animSpeed = element<HTMLInputElement>('anim-speed');
export const animSpeedValue = element('anim-speed-value');
export const viewerControls = [
  viewerResetBtn,
  viewerAutoRotateBtn,
  viewerWireframeBtn,
  viewerBaseColorBtn,
  viewerSmoothNormalsBtn,
  viewerGridBtn,
];
export const scenePanel = element('scene-panel');
export const sceneSection = element('scene-section');
export const workspace = query('.workspace');
export const sidebar = query('.sidebar');
export const exportSidebar = element('export-sidebar');
export const sceneTree = element('scene-tree');
export const sceneResourceList = element('scene-resource-list');
export const sceneMaterialList = element('scene-material-list');
export const sceneClipList = element('scene-clip-list');
export const sceneStatFields = {
  nodes: element('scene-node-stat'),
  meshes: element('mesh-count'),
  materials: element('scene-material-stat'),
  skins: element('scene-skin-stat'),
  morphs: element('scene-morph-stat'),
  clips: element('scene-clip-stat'),
  lights: element('scene-light-stat'),
};
/** The geometry readout under the viewport, shared by both mesh summaries. */
export const meshStatFields = {
  meshes: element('mesh-count'),
  vertices: element('vertex-count'),
  triangles: element('triangle-count'),
  hasNormals: element('has-normals'),
  hasUvs: element('has-uvs'),
};

/** The Draco compression report, shown only after an encode. */
export const compressionStats = element('compression-stats');
export const compressionStatFields = {
  method: element('stats-method'),
  speed: element('stats-speed'),
  prediction: element('stats-prediction'),
  size: element('stats-size'),
};

export const sceneCapabilitySummary = element('scene-capability-summary');
export const sceneInfo = element('scene-info');
export const sceneWarningList = element('scene-warning-list');
export const sceneWarnings = element<HTMLDetailsElement>('scene-warnings');
export const sceneWarningsSection = element('scene-warnings-section');
export const sceneWarningCount = element('scene-warning-count');
export const sceneTreeExpandButton = element<HTMLButtonElement>('scene-tree-expand');
export const sceneTreeCollapseButton = element<HTMLButtonElement>('scene-tree-collapse');

const setSceneTreeExpanded = (expanded: boolean) => {
  for (const branch of sceneTree.querySelectorAll<HTMLDetailsElement>('details.scene-tree-node')) branch.open = expanded;
};
sceneTreeExpandButton?.addEventListener('click', () => setSceneTreeExpanded(true));
sceneTreeCollapseButton?.addEventListener('click', () => setSceneTreeExpanded(false));

// Keep the workflow columns source-neutral: import + hierarchy on the left,
// export/report on the right. The existing viewport statistics stay in place.
if (sidebar && sceneSection && sceneSection.parentElement !== sidebar) {
  sidebar.append(sceneSection);
}
// Contents is its own card below the scene card, so it cannot squeeze the tree.
if (sidebar && sceneInfo && sceneInfo.parentElement !== sidebar) {
  sidebar.append(sceneInfo);
}
if (exportSidebar && exportSection && exportSection.parentElement !== exportSidebar) {
  exportSidebar.append(exportSection);
}
// Every warning the app produces lands in this one card, directly under Export.
if (exportSidebar && sceneWarningsSection && sceneWarningsSection.parentElement !== exportSidebar) {
  exportSidebar.append(sceneWarningsSection);
}
