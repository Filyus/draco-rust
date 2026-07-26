/**
 * Draco 3D Format Converter - Main Application
 *
 * This application loads separate WASM modules for each reader/writer format
 * and provides a unified interface for 3D file format conversion.
 */

import { Viewer } from './viewer.ts';
import type { SceneDocument } from './scene-document.ts';
import type { ResourceMap } from './scene-resources.ts';
import type { FbxSceneProvenance } from './fbx-scene-provenance.ts';
import { buildFbxSceneFromGltf, buildFlatMeshesFromGltf, buildSceneFromGltf } from './gltf-loader.ts';
import { buildSceneDocumentFromGltf } from './gltf-scene-document.ts';
import { buildSceneFromFbx, buildSceneFromMeshes } from './mesh-loader.ts';
import { buildSceneDocumentWithFbxProvenance } from './fbx-scene-document.ts';
import { buildFbxSceneFromDocument } from './fbx-scene-document-writer.ts';
import { serializeSceneDocumentToGlb } from './scene-document-gltf.ts';
import { assertValidSceneDocument } from './scene-document.ts';
import { basename } from './scene-resources.ts';

/** One lazily loaded wasm-pack module and whether its init has completed. */
interface ModuleSlot {
    loaded: boolean;
    module: any;
}

// Module state
const modules: Record<string, ModuleSlot> = {
    obj: { loaded: false, module: null },
    ply: { loaded: false, module: null },
    gltf: { loaded: false, module: null },
    fbx: { loaded: false, module: null },
};

// Current loaded mesh data
let currentMeshData: any = null;
let currentFileType: string | null = null;
let currentSourceData: Uint8Array | null = null;
let currentSourceResources: ResourceMap = Object.create(null);
// FBX uses the source-neutral SceneDocument for cross-format GLB export.
// Direct glTF/GLB inputs continue to use their lossless source-byte route.
let currentSceneDocument: SceneDocument | null = null;
let currentFbxProvenance: FbxSceneProvenance | null = null;

// 3D preview viewer (lazily created on first use)
let viewer: Viewer | null = null;

// Opt-in diagnostics for importer/exporter development. Normal conversion
// warnings still go through the visible log panel; this only replaces noisy
// object dumps that were previously emitted for every PLY/export operation.
const debugLogging = new URLSearchParams(globalThis.location?.search || '').has('debug');
function debugLog(...values: unknown[]) {
    if (debugLogging) console.debug('[Draco debug]', ...values);
}

/**
 * Resolve an element this page owns.
 *
 * index.html declares every id used below, so the app has always treated them
 * as present; this states that instead of threading a null check through each
 * of eighty-odd lookups. Genuinely optional elements keep their explicit
 * guards at the point of use.
 */
function element<T extends HTMLElement = HTMLElement>(id: string): T {
    return document.getElementById(id) as T;
}

function query<T extends HTMLElement = HTMLElement>(selector: string): T {
    return document.querySelector(selector) as T;
}

function errorMessage(error: unknown) {
    if (error && typeof (error as Error).message === 'string') {
        return (error as Error).message;
    }
    return String(error);
}

// DOM Elements
const dropZone = element('drop-zone');
const fileInput = element<HTMLInputElement>('file-input');
const fileInfo = element('file-info');
const fileName = element('file-name');
const fileSize = element('file-size');
const clearFileBtn = element<HTMLButtonElement>('clear-file');
const previewSection = element('preview-section');
const exportSection = element('export-section');
const exportFormat = element<HTMLSelectElement>('export-format');
const useDraco = element<HTMLInputElement>('use-draco');
const useDracoLabel = element('use-draco-label');
const dracoOptions = element('draco-options');
const dracoSettings = element('draco-settings');
const encodingSpeed = element<HTMLSelectElement>('encoding-speed');
const encodingMethod = element<HTMLSelectElement>('encoding-method');
const positionBits = element<HTMLInputElement>('position-bits');
const normalBits = element<HTMLInputElement>('normal-bits');
const texcoordBits = element<HTMLInputElement>('texcoord-bits');
const exportBtn = element<HTMLButtonElement>('export-btn');
const consoleEl = element('console');

// 3D preview DOM references
const viewerSection = element('viewer-section');
const viewerCanvas = element<HTMLCanvasElement>('viewer-canvas');
const viewerResetBtn = element<HTMLButtonElement>('viewer-reset');
const viewerAutoRotateBtn = element<HTMLButtonElement>('viewer-autorotate');
const viewerWireframeBtn = element<HTMLButtonElement>('viewer-wireframe');
const viewerBaseColorBtn = element<HTMLButtonElement>('viewer-base-color');
const viewerSmoothNormalsBtn = element<HTMLButtonElement>('viewer-smooth-normals');
const viewerGridBtn = element<HTMLButtonElement>('viewer-grid');
const viewerAnimation = element('viewer-animation');
const animPlayBtn = element<HTMLButtonElement>('anim-play');
const animClipSelect = element<HTMLSelectElement>('anim-clip');
const animClipTrigger = element<HTMLButtonElement>('anim-clip-trigger');
const animClipLabel = element('anim-clip-label');
const animClipMenu = element('anim-clip-menu');
const animLoopCheckbox = element<HTMLInputElement>('anim-loop');
const animTimeLabel = element('anim-time');
const animScrub = element<HTMLInputElement>('anim-scrub');
const animSpeed = element<HTMLInputElement>('anim-speed');
const animSpeedValue = element('anim-speed-value');
const viewerControls = [
    viewerResetBtn,
    viewerAutoRotateBtn,
    viewerWireframeBtn,
    viewerBaseColorBtn,
    viewerSmoothNormalsBtn,
    viewerGridBtn,
];
const scenePanel = element('scene-panel');
const sceneSection = element('scene-section');
const workspace = query('.workspace');
const sidebar = query('.sidebar');
const exportSidebar = element('export-sidebar');
const sceneTree = element('scene-tree');
const sceneResourceList = element('scene-resource-list');
const sceneMaterialList = element('scene-material-list');
const sceneClipList = element('scene-clip-list');
const sceneStatFields = {
    nodes: element('scene-node-stat'),
    meshes: element('mesh-count'),
    materials: element('scene-material-stat'),
    skins: element('scene-skin-stat'),
    morphs: element('scene-morph-stat'),
    clips: element('scene-clip-stat'),
};
const sceneCapabilitySummary = element('scene-capability-summary');
const sceneInfo = element('scene-info');
const sceneWarningList = element('scene-warning-list');
const sceneWarnings = element<HTMLDetailsElement>('scene-warnings');
const sceneWarningsSection = element('scene-warnings-section');
const sceneWarningCount = element('scene-warning-count');
const sceneTreeExpandButton = element<HTMLButtonElement>('scene-tree-expand');
const sceneTreeCollapseButton = element<HTMLButtonElement>('scene-tree-collapse');

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

function setViewerControlsEnabled(enabled: boolean) {
    for (const control of viewerControls) control.disabled = !enabled;
}

function syncViewerToolbar() {
    if (!viewer) return;
    syncAutoRotateButton(viewer.autoRotate);
    const toggles: [HTMLButtonElement, boolean][] = [
        [viewerWireframeBtn, viewer.wireframe],
        [viewerBaseColorBtn, viewer.baseColorOnly],
        [viewerSmoothNormalsBtn, viewer.smoothNormals],
        [viewerGridBtn, viewer.showGrid],
    ];
    for (const [button, enabled] of toggles) {
        button.classList.toggle('active', enabled);
        button.setAttribute('aria-pressed', String(enabled));
    }
}

function syncAutoRotateButton(enabled: boolean) {
    viewerAutoRotateBtn.classList.toggle('active', enabled);
    viewerAutoRotateBtn.setAttribute('aria-pressed', String(enabled));
}

function setupChoiceControl(select: HTMLSelectElement) {
    const control = document.querySelector(`[data-choice-for="${select.id}"]`);
    if (!control) return;
    const buttons = Array.from(control.querySelectorAll<HTMLButtonElement>('button[data-value]'));
    const sync = () => {
        for (const button of buttons) {
            const selected = button.dataset.value === select.value;
            button.classList.toggle('selected', selected);
            button.setAttribute('aria-pressed', String(selected));
        }
    };
    for (const button of buttons) {
        button.addEventListener('click', () => {
            if (button.dataset.value === select.value) return;
            select.value = button.dataset.value!;
            select.dispatchEvent(new Event('change', { bubbles: true }));
        });
    }
    select.addEventListener('change', sync);
    sync();
}

setupChoiceControl(exportFormat);
setupChoiceControl(encodingMethod);

// Initialize application
async function init() {
    log('Initializing Draco 3D Format Converter...', 'info');
    
    // Load all WASM modules
    await loadAllModules();
    
    // Setup event listeners
    setupEventListeners();
    
    log('Ready to convert 3D files!', 'success');
}

// Load all WASM modules
async function loadAllModules() {
    // Cache-bust to ensure fresh WASM/JS are loaded (helps avoid stale cached files during development)
    const CACHE_BUST = `?v=${Date.now()}`;
    const moduleConfigs = [
        { key: 'obj', path: `./pkg/obj.js${CACHE_BUST}`, statusId: 'obj-status' },
        { key: 'ply', path: `./pkg/ply.js${CACHE_BUST}`, statusId: 'ply-status' },
        { key: 'gltf', path: `./pkg/gltf.js${CACHE_BUST}`, statusId: 'gltf-status' },
        { key: 'fbx', path: `./pkg/fbx.js${CACHE_BUST}`, statusId: 'fbx-status' },
    ];

    const loadPromises = moduleConfigs.map(config => loadModule(config));
    await Promise.allSettled(loadPromises);
}

// Load a single WASM module
async function loadModule({ key, path, statusId }: { key: string; path: string; statusId: string }) {
    const statusEl = element(statusId);
    const indicator = statusEl.querySelector('.status-indicator')!;
    // ensure initial loading state
    if (indicator) {
        indicator.classList.remove('ready','error');
        indicator.classList.add('loading');
        const statusTextInit = indicator.querySelector('.status-text');
        if (statusTextInit) statusTextInit.textContent = 'Loading...';
        statusEl.removeAttribute('aria-label');
    }
    
    try {
        const module = await import(path);
        const wasmUrl = new URL(path.replace(/\.js(\?.*)?$/, '_bg.wasm$1'), window.location.href);
        await module.default(wasmUrl);
        
        modules[key].module = module;
        modules[key].loaded = true;
        if (key === 'gltf') {
            updateDracoEncoderAvailability();
        }
        
        // Update visual indicator (dot + aria label)
        const statusText = indicator.querySelector('.status-text');
        const statusDot = indicator.querySelector('.status-dot');
        if (statusText) statusText.textContent = 'Ready';
        indicator.classList.remove('loading','error');
        indicator.classList.add('ready');
        indicator.setAttribute('aria-label', 'Ready');
        if (statusDot) {
            statusDot.classList.remove('dot-loading','dot-error','dot-ready');
            // visual state is controlled by the parent .status-indicator class
        }
        
        const version = module.version ? module.version() : '?';
        log(`${key} v${version} loaded`, 'success');
    } catch (error) {
        const statusText = indicator.querySelector('.status-text');
        const statusDot = indicator.querySelector('.status-dot');
        if (statusText) statusText.textContent = 'Error';
        indicator.classList.remove('loading','ready');
        indicator.classList.add('error');
        indicator.setAttribute('aria-label', 'Error');
        if (statusDot) {
            statusDot.classList.remove('dot-loading','dot-ready','dot-error');
            // visual state is controlled by the parent .status-indicator class
        }
        log(`Failed to load ${key}: ${errorMessage(error)}`, 'error');
    }
}

// Setup event listeners
function setupEventListeners() {
    // Drag and drop
    dropZone.addEventListener('dragover', (e) => {
        e.preventDefault();
        dropZone.classList.add('drag-over');
    });
    
    dropZone.addEventListener('dragleave', () => {
        dropZone.classList.remove('drag-over');
    });
    
    dropZone.addEventListener('drop', (e) => {
        e.preventDefault();
        dropZone.classList.remove('drag-over');
        
        const files = e.dataTransfer!.files;
        if (files.length > 0) {
            handleFiles(files);
        }
    });
    
    // File input
    fileInput.addEventListener('change', () => {
        if (fileInput.files && fileInput.files.length > 0) {
            handleFiles(fileInput.files);
        }
    });
    
    // Clear file
    clearFileBtn.addEventListener('click', clearFile);
    
    // Export format change
    exportFormat.addEventListener('change', updateExportOptions);
    
    // Draco checkbox
    useDraco.addEventListener('change', () => {
        dracoSettings.style.display = useDraco.checked ? 'grid' : 'none';
    });
    
    // Quantization sliders
    encodingSpeed.addEventListener('input', () => {
        element('speed-value').textContent = encodingSpeed.value;
    });
    positionBits.addEventListener('input', () => {
        element('position-bits-value').textContent = positionBits.value;
    });
    normalBits.addEventListener('input', () => {
        element('normal-bits-value').textContent = normalBits.value;
    });
    texcoordBits.addEventListener('input', () => {
        element('texcoord-bits-value').textContent = texcoordBits.value;
    });
    
    // Export button
    exportBtn.addEventListener('click', exportFile);

    // 3D preview toolbar
    viewerResetBtn.addEventListener('click', () => {
        if (!viewer) return;
        viewer.resetView();
    });
    viewerAutoRotateBtn.addEventListener('click', () => {
        if (!viewer) return;
        viewer.setAutoRotate(!viewer.autoRotate);
    });
    viewerWireframeBtn.addEventListener('click', () => {
        if (!viewer) return;
        viewer.wireframe = !viewer.wireframe;
        viewerWireframeBtn.classList.toggle('active', viewer.wireframe);
        viewerWireframeBtn.setAttribute('aria-pressed', String(viewer.wireframe));
    });
    viewerBaseColorBtn.addEventListener('click', () => {
        if (!viewer) return;
        viewer.baseColorOnly = !viewer.baseColorOnly;
        viewerBaseColorBtn.classList.toggle('active', viewer.baseColorOnly);
        viewerBaseColorBtn.setAttribute('aria-pressed', String(viewer.baseColorOnly));
    });
    viewerSmoothNormalsBtn.addEventListener('click', () => {
        if (!viewer) return;
        viewer.smoothNormals = !viewer.smoothNormals;
        viewerSmoothNormalsBtn.classList.toggle('active', viewer.smoothNormals);
        viewerSmoothNormalsBtn.setAttribute('aria-pressed', String(viewer.smoothNormals));
    });
    viewerGridBtn.addEventListener('click', () => {
        if (!viewer) return;
        viewer.showGrid = !viewer.showGrid;
        viewerGridBtn.classList.toggle('active', viewer.showGrid);
        viewerGridBtn.setAttribute('aria-pressed', String(viewer.showGrid));
    });

    // Animation controls
    animPlayBtn.addEventListener('click', toggleAnimationPlayback);
    animClipSelect.addEventListener('change', () => {
        if (!viewer) return;
        const idx = Number(animClipSelect.value);
        viewer.animation.clipIndex = idx;
        viewer.seekAnimation(0);
        syncAnimationClipSelection();
        updateAnimationScrub();
    });
    animClipTrigger.addEventListener('click', (event) => {
        event.stopPropagation();
        const open = animClipTrigger.getAttribute('aria-expanded') !== 'true';
        if (open) openAnimationClipMenu();
        else closeAnimationClipMenu();
    });
    animClipMenu.addEventListener('click', (event) => event.stopPropagation());
    animClipTrigger.addEventListener('keydown', handleAnimationClipTriggerKeydown);
    animClipMenu.addEventListener('keydown', handleAnimationClipMenuKeydown);
    // Wrapped: passing the listener directly would hand the MouseEvent in as a
    // truthy `restoreFocus`, so every click in the page focused the trigger.
    document.addEventListener('click', () => closeAnimationClipMenu());
    document.addEventListener('keydown', (event) => {
        if (event.key === 'Escape') closeAnimationClipMenu();
    });
    document.addEventListener('keydown', handlePlaybackShortcut);
    animLoopCheckbox.addEventListener('change', () => {
        if (viewer) viewer.animation.loop = animLoopCheckbox.checked;
    });
    animScrub.addEventListener('input', () => {
        if (!viewer || !viewer.scene?.animations?.length) return;
        const clip = viewer.scene.animations[viewer.animation.clipIndex];
        if (!clip) return;
        viewer.animation.playing = false;
        updateAnimationPlayButton();
        const t = (Number(animScrub.value) / 1000) * clip.duration;
        viewer.seekAnimation(t);
        updateAnimationScrub();
    });
    animSpeed.addEventListener('input', () => {
        const v = Number(animSpeed.value) / 100;
        if (viewer) viewer.animation.speed = v;
        animSpeedValue.textContent = `${v.toFixed(2)}×`;
    });
}

// Handle file selection
async function handleFiles(fileList: FileList) {
    const files = Array.from(fileList);
    const supportedMain = files.filter((file) =>
        ['obj', 'ply', 'gltf', 'glb', 'fbx'].includes(file.name.split('.').pop()!.toLowerCase())
    );
    if (supportedMain.length === 0) {
        log('No supported main 3D file found in the selection', 'error');
        return;
    }
    const gltfMain = supportedMain.filter((file) => /\.(gltf|glb)$/i.test(file.name));
    const file = gltfMain.length === 1 ? gltfMain[0] : supportedMain[0];
    if (supportedMain.length > 1) {
        log('Select exactly one main model file; additional files are only companions for glTF', 'error');
        return;
    }
    await handleFile(file, files.filter((candidate) => candidate !== file));
}

function updateDracoEncoderAvailability() {
    const prototype = modules.gltf.module?.GltfAsset?.prototype;
    const available = typeof prototype?.compressPrimitive === 'function';
    useDraco.disabled = !available;
    useDraco.checked = available;
    useDracoLabel.textContent = available
        ? 'Enable Draco Compression'
        : 'Draco Compression (not included in this build)';
    dracoSettings.style.display = available && useDraco.checked ? 'grid' : 'none';
}

async function handleFile(file: File, companionFiles: File[] = []) {
    const extension = file.name.split('.').pop()!.toLowerCase();
    
    if (!['obj', 'ply', 'gltf', 'glb', 'fbx'].includes(extension)) {
        log(`Unsupported file format: .${extension}`, 'error');
        return;
    }
    
    log(`Loading ${file.name}...`, 'info');
    
    // Show file info
    fileName.textContent = companionFiles.length > 0
        ? `${file.name} (+${companionFiles.length} resources)`
        : file.name;
    const totalSize = companionFiles.reduce((sum, companion) => sum + companion.size, file.size);
    fileSize.textContent = formatFileSize(totalSize);
    fileInfo.style.display = 'flex';
    dropZone.style.display = 'none';
    
    currentFileType = extension;
    
    try {
        const arrayBuffer = await file.arrayBuffer();
        const data = new Uint8Array(arrayBuffer);
        currentSourceData = new Uint8Array(data);
        currentSourceResources = Object.create(null);
        currentSceneDocument = null;
        currentFbxProvenance = null;
        clearWarningPanel();
        for (const companion of companionFiles) {
            if (Object.prototype.hasOwnProperty.call(currentSourceResources, companion.name)) {
                throw new Error(`Duplicate companion resource name: ${companion.name}`);
            }
            currentSourceResources[companion.name] = new Uint8Array(await companion.arrayBuffer());
        }
        
        // Parse file based on extension
        let result;
        switch (extension) {
            case 'obj':
                result = await parseObjFile(data, currentSourceResources);
                break;
            case 'ply':
                result = await parsePlyFile(data);
                break;
            case 'gltf':
            case 'glb':
                result = await parseGltfFile(data, extension, currentSourceResources);
                if (result?.success && result.document) {
                    try {
                        currentSceneDocument = buildSceneDocumentFromGltf(data, currentSourceResources as Record<string, Uint8Array>, modules.gltf.module);
                    } catch (error) {
                        log(`Scene details unavailable: ${errorMessage(error)}`, 'warning');
                    }
                }
                break;
            case 'fbx':
                result = await parseFbxFile(data);
                if (result?.success && result.scene) {
                    const adapted = buildSceneDocumentWithFbxProvenance(result, currentSourceResources);
                    currentSceneDocument = adapted.document;
                    currentFbxProvenance = adapted.provenance;
                }
                break;
        }
        
        if (result && result.success) {
            currentMeshData = result;
            displayMeshInfo(result);
            renderSceneDocumentSummary(currentSceneDocument!);
            previewSection.style.display = 'block';
            exportSection.style.display = 'flex';
            exportSidebar.style.display = 'flex';
            workspace.classList.add('export-loaded');
            setWarningSource('export', []);
            log(`Successfully parsed ${file.name}`, 'success');
            await loadPreview(extension);
        } else {
            log(`Failed to parse file: ${result?.error || 'Unknown error'}`, 'error');
        }
    } catch (error) {
        const message = errorMessage(error);
        const resourceHint = extension === 'gltf' && message.includes('External resource denied:')
            ? ' Select the .gltf together with all referenced .bin and image files.'
            : '';
        log(`Error reading file: ${message}.${resourceHint}`, 'error');
    }
}

// Parse OBJ file
async function parseObjFile(data: Uint8Array, resources = Object.create(null)) {
    if (!modules.obj.loaded) {
        return { success: false, error: 'OBJ module not loaded' };
    }

    const textContent = new TextDecoder().decode(data);
    const result = modules.obj.module.parse_obj(textContent);
    if (result?.success) {
        result.materials = parseObjMaterials(textContent, resources, result.warnings || (result.warnings = []));
    }
    return result;
}

function parseObjMaterials(objText: string, resources: ResourceMap, warnings: string[]) {
    const materials = Object.create(null);
    const libraries = [];
    for (const line of objText.split(/\r\n|[\r\n]/)) {
        const match = line.trim().match(/^mtllib\s+(.+)$/i);
        if (match) libraries.push(match[1].trim());
    }
    for (const library of libraries) {
        const bytes = resources[library] || resources[basename(library)];
        if (!bytes) {
            warnings.push(`OBJ material library not selected: ${library}`);
            continue;
        }
        let current = null;
        const text = new TextDecoder().decode(bytes);
        for (const line of text.split(/\r\n|[\r\n]/)) {
            const trimmed = line.trim();
            if (!trimmed || trimmed.startsWith('#')) continue;
            const [directive, ...values] = trimmed.split(/\s+/);
            if (directive === 'newmtl') {
                current = values.join(' ');
                if (current) materials[current] ||= { diffuse: [1, 1, 1], alpha: 1 };
            } else if (directive === 'Kd' && current && values.length >= 3) {
                const diffuse = values.slice(0, 3).map(Number);
                if (diffuse.every(Number.isFinite)) materials[current].diffuse = diffuse;
            } else if ((directive === 'd' || directive === 'Tr') && current && values.length >= 1) {
                const value = Number(values[0]);
                if (Number.isFinite(value)) materials[current].alpha = directive === 'Tr' ? 1 - value : value;
            } else if (directive.toLowerCase() === 'map_kd' && current) {
                const texture = mtlMapPath(values);
                if (texture) materials[current].baseColorTextureUri = texture;
            }
        }
    }
    return materials;
}

// `map_Kd` accepts optional flags before its filename. Keep the filename (which
// may itself contain spaces) while skipping the standardized flag arguments.
function mtlMapPath(values: string[]) {
    const optionValues = {
        '-blendu': 1, '-blendv': 1, '-cc': 1, '-clamp': 1, '-texres': 1,
        '-bm': 1, '-imfchan': 1, '-type': 1, '-mm': 2, '-o': 3, '-s': 3, '-t': 3,
    };
    let index = 0;
    while (index < values.length && values[index].startsWith('-')) {
        index += 1 + (optionValues[values[index].toLowerCase() as keyof typeof optionValues] ?? 0);
    }
    return values.slice(index).join(' ').trim();
}

// Parse PLY file
async function parsePlyFile(data: Uint8Array) {
    if (!modules.ply.loaded) {
        return { success: false, error: 'PLY module not loaded' };
    }

    const result = modules.ply.module.parse_ply_bytes(data);
    debugLog('PLY parse result:', result);
    if (result.meshes) {
        for (const mesh of result.meshes) {
            debugLog('PLY mesh: positions=', mesh.positions?.length,
                ', indices=', mesh.indices?.length,
                ', normals=', mesh.normals?.length);
        }
    }
    return result;
}

// Parse glTF/GLB file
async function parseGltfFile(data: Uint8Array, extension: string, resources = Object.create(null)) {
    if (!modules.gltf.loaded) {
        return { success: false, error: 'glTF module not loaded' };
    }

    const summary = modules.gltf.module.inspect_gltf(data);
    if (!summary.success) {
        return { ...summary, document: true, format: extension };
    }

    const asset = modules.gltf.module.GltfAsset.withResources(data, resources, '2.1');
    let vertexCount = 0;
    let triangleCount = 0;
    let hasNormals = false;
    let hasUvs = false;

    try {
        for (let mesh = 0; mesh < asset.meshCount(); mesh += 1) {
            const primitiveCount = asset.primitiveCount(mesh);
            for (let primitive = 0; primitive < primitiveCount; primitive += 1) {
                const geometry = asset.readPrimitive(mesh, primitive);
                try {
                    let primitiveVertexCount = 0;
                    for (let attribute = 0; attribute < geometry.attributeCount(); attribute += 1) {
                        const semantic = geometry.attributeSemantic(attribute);
                        if (semantic === 'POSITION') {
                            primitiveVertexCount = geometry.attributeElementCount(attribute);
                        } else if (semantic === 'NORMAL') {
                            hasNormals = true;
                        } else if (semantic.startsWith('TEXCOORD_')) {
                            hasUvs = true;
                        }
                    }

                    vertexCount += primitiveVertexCount;
                    const elementCount = geometry.hasIndices()
                        ? geometry.indexCount()
                        : primitiveVertexCount;
                    triangleCount += triangleCountForMode(geometry.mode(), elementCount);
                } finally {
                    geometry.free();
                }
            }
        }
    } finally {
        asset.free();
    }

    return {
        ...summary,
        document: true,
        format: extension,
        vertexCount,
        triangleCount,
        hasNormals,
        hasUvs,
    };
}

function triangleCountForMode(mode: number, elementCount: number) {
    switch (mode) {
        case 4: // TRIANGLES
            return Math.floor(elementCount / 3);
        case 5: // TRIANGLE_STRIP
        case 6: // TRIANGLE_FAN
            return Math.max(0, elementCount - 2);
        default:
            return 0;
    }
}

// Parse FBX file
async function parseFbxFile(data: Uint8Array) {
    // ASCII and binary both go through the WASM reader now; it produces the
    // same node tree from either container, so the regex fallback that used to
    // scrape ASCII geometry -- without transforms, materials, skins or
    // animation -- is gone.
    if (!modules.fbx.loaded) {
        return { success: false, error: 'FBX module not loaded' };
    }

    return modules.fbx.module.parse_fbx(data);
}

function renderSceneDocumentSummary(sceneDocument: SceneDocument, extraWarnings = []) {
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
            (total: number, mesh: any) => total + mesh.primitives.reduce((count: number, primitive: any) => count + (primitive.targets?.length || 0), 0),
            0,
        );
        sceneStatFields.nodes.textContent = sceneDocument.nodes.length.toLocaleString();
        sceneStatFields.meshes.textContent = sceneDocument.meshes.length.toLocaleString();
        sceneStatFields.materials.textContent = sceneDocument.materials.length.toLocaleString();
        sceneStatFields.skins.textContent = sceneDocument.skins.length.toLocaleString();
        sceneStatFields.morphs.textContent = morphs.toLocaleString();
        sceneStatFields.clips.textContent = sceneDocument.animations.length.toLocaleString();
        renderSceneTree(sceneDocument);
        renderSceneCompanions(sceneDocument);
        sceneSection.style.display = 'flex';
        workspace.classList.add('scene-loaded');
        sceneCapabilitySummary.textContent = describeSceneCapabilities(validation.capabilities);
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

function describeSceneCapabilities(capabilities: any = {}) {
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

const WARNING_PREVIEW_COUNT = 8;

// Renders a numbered list; the overflow row is a real button that reveals the rest.
function setWarningList(list: HTMLElement, warnings: string[], limit = WARNING_PREVIEW_COUNT) {
    list.replaceChildren();
    const unique = uniqueWarnings(warnings);
    const shown = Number.isFinite(limit) ? unique.slice(0, limit) : unique;
    shown.forEach((warning, index) => {
        const item = document.createElement('li');
        item.className = 'scene-warning-item';
        const marker = document.createElement('span');
        marker.className = 'scene-warning-index';
        marker.setAttribute('aria-hidden', 'true');
        marker.textContent = String(index + 1);
        const text = document.createElement('span');
        text.className = 'scene-warning-text';
        text.textContent = warning;
        item.append(marker, text);
        list.appendChild(item);
    });
    const hidden = unique.length - shown.length;
    if (hidden > 0) {
        const item = document.createElement('li');
        item.className = 'scene-warning-item scene-warning-more';
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'scene-warning-more-button';
        button.textContent = `Show ${hidden} more`;
        button.addEventListener('click', () => setWarningList(list, unique, Number.POSITIVE_INFINITY));
        item.appendChild(button);
        list.appendChild(item);
    }
    return unique;
}

function uniqueWarnings(warnings: string[]) {
    return [...new Set(warnings.filter((warning: string) => typeof warning === 'string' && warning.trim()))];
}

// Warnings arrive from independent stages, so each stage owns one slot and the
// panel always shows their union — no stage can wipe another's warnings.
const warningSources: Record<'scene' | 'mesh' | 'preview' | 'export', string[]> =
    { scene: [], mesh: [], preview: [], export: [] };

type WarningSource = keyof typeof warningSources;

function setWarningSource(source: WarningSource, warnings: string[]) {
    warningSources[source] = uniqueWarnings(warnings || []);
    setWarningPanel([
        ...warningSources.scene,
        ...warningSources.mesh,
        ...warningSources.preview,
        ...warningSources.export,
    ]);
}

function clearWarningPanel() {
    for (const source of Object.keys(warningSources) as WarningSource[]) warningSources[source] = [];
    setWarningPanel([]);
}

// The warnings live in their own collapsible panel so the scene tree stays readable.
// Collapsed state still advertises how many warnings are waiting.
function setWarningPanel(warnings: string[]) {
    if (!sceneWarnings) return;
    const unique = setWarningList(sceneWarningList, warnings);
    const total = unique.length;
    sceneWarningCount.textContent = String(total);
    if (sceneWarningsSection) sceneWarningsSection.hidden = total === 0;
    if (total === 0) sceneWarnings.open = false;
}

function renderSceneTree(sceneDocument: SceneDocument) {
    sceneTree.replaceChildren();
    if (sceneDocument.nodes.length === 0) {
        const empty = document.createElement('div');
        empty.className = 'scene-tree-empty';
        empty.textContent = 'This loaded document has no scene nodes.';
        sceneTree.appendChild(empty);
        return;
    }
    const animatedNodes = new Set(sceneDocument.animations.flatMap((clip: any) => clip.channels.map((channel: any) => channel.node)));
    const appendNode = (nodeIndex: number, depth: number, visited: Set<number>, target: any) => {
        if (visited.has(nodeIndex)) return;
        visited.add(nodeIndex);
        const node = sceneDocument.nodes[nodeIndex] || {};
        const children = (node.children || []).filter((child: any) => Number.isInteger(child) && child >= 0 && child < sceneDocument.nodes.length);
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
            children.forEach((child: any, position) => {
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

function renderSceneCompanions(sceneDocument: SceneDocument) {
    const formatNames = (items: any[], fallback: string) => {
        if (!items.length) return fallback;
        const names = items.map((item, index: number) => item.name || `${fallback} ${index + 1}`);
        return names.length > 3 ? `${names.slice(0, 3).join(', ')} +${names.length - 3}` : names.join(', ');
    };
    sceneResourceList.textContent = formatNames(sceneDocument.resources, 'none');
    sceneMaterialList.textContent = formatNames(sceneDocument.materials, 'none');
    sceneClipList.textContent = formatNames(sceneDocument.animations, 'none');
}

// Display mesh information
function displayMeshInfo(result: any) {
    if (result.document) {
        element('mesh-count').textContent = result.meshCount.toLocaleString();
        element('vertex-count').textContent = result.vertexCount.toLocaleString();
        element('triangle-count').textContent = result.triangleCount.toLocaleString();
        element('has-normals').textContent = result.hasNormals ? 'Yes' : 'No';
        element('has-uvs').textContent = result.hasUvs ? 'Yes' : 'No';
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
        if (mesh.normals?.length > 0) hasNormals = true;
        if (mesh.uvs?.length > 0) hasUvs = true;
    }
    
    element('mesh-count').textContent = meshes.length;
    element('vertex-count').textContent = totalVertices.toLocaleString();
    element('triangle-count').textContent = totalTriangles.toLocaleString();
    element('has-normals').textContent = hasNormals ? 'Yes' : 'No';
    element('has-uvs').textContent = hasUvs ? 'Yes' : 'No';

    setWarningSource('mesh', result.warnings || []);
}

// Update export options based on format
function updateExportOptions() {
    const format = exportFormat.value;
    
    // Show/hide Draco options for glTF formats only
    if (format === 'gltf' || format === 'glb') {
        dracoOptions.style.display = 'flex';
    } else {
        dracoOptions.style.display = 'none';
    }
}

// Export file
async function exportFile() {
    if (!currentMeshData) {
        log('No mesh data to export', 'error');
        return;
    }
    
    const format = exportFormat.value;
    log(`Exporting to ${format.toUpperCase()}...`, 'info');
    
    try {
        let result;
        if (format === 'glb' && currentFileType === 'fbx' && currentSceneDocument) {
            result = exportSceneDocumentToGlb(currentSceneDocument);
            for (const warning of result.warnings || []) log(warning, 'warning');
            logSceneDocumentCapabilities(result.capabilities);
            setWarningSource('export', result.warnings || []);
            downloadResult(result, format);
            log(result.message, 'success');
            return;
        }
        if (currentMeshData.document && (format === 'gltf' || format === 'glb')) {
            result = exportGltfDocument(format);
            downloadResult(result, format);
            log(result.message, 'success');
            return;
        }
        const legacyFbx = format === 'fbx-legacy';
        if ((format === 'fbx' || legacyFbx) && currentFileType === 'fbx' && currentSceneDocument) {
            const scene = buildFbxSceneFromDocument(currentSceneDocument, { provenance: currentFbxProvenance });
            result = await exportToFbxScene(scene, legacyFbx);
        } else if ((format === 'fbx' || legacyFbx) && currentMeshData.scene) {
            result = await exportToFbxScene(
                prepareFbxSceneForExport(currentMeshData.scene, legacyFbx),
                legacyFbx,
            );
        } else if (currentMeshData.document && (format === 'fbx' || legacyFbx)) {
            const scene = prepareFbxSceneForExport(buildFbxSceneFromGltf(
                currentSourceData!,
                currentSourceResources,
                modules.gltf.module,
                { legacyCompatibility: legacyFbx },
            ), legacyFbx);
            result = await exportToFbxScene(scene, legacyFbx);
        } else {
        const sourceMeshes = currentMeshData.document
            ? buildFlatMeshesFromGltf(
                currentSourceData!,
                currentSourceResources,
                modules.gltf.module,
            )
            : currentMeshData.meshes;
        const meshes = prepareMeshesForExport(sourceMeshes);
        if (meshes.length === 0) {
            throw new Error('The document contains no triangle geometry to export');
        }
        
        switch (format) {
            case 'obj':
                result = await exportToObj(meshes);
                break;
            case 'ply':
                result = await exportToPly(meshes);
                break;
            case 'gltf':
            case 'glb':
                result = await exportToGltf(meshes, format);
                break;
            case 'fbx':
            case 'fbx-legacy':
                result = await exportToFbx(meshes);
                break;
        }
        }
        
        if (result && result.success) {
            downloadResult(result, format);
            
            // Display compression stats if available
            if (result.draco_stats) {
                displayCompressionStats(result.draco_stats);
            } else {
                // Hide stats if not using Draco
                element('compression-stats').style.display = 'none';
            }
            if (result.compression_report) {
                const compressed = result.compression_report.compressed_primitives?.length || 0;
                const preserved = result.compression_report.preserved_primitives?.length || 0;
                log(`Compression report: ${compressed} compressed, ${preserved} preserved`, preserved > 0 ? 'warning' : 'success');
            }
            
            log(`Export complete!`, 'success');
        } else {
            log(`Export failed: ${result?.error || 'Unknown error'}`, 'error');
        }
    } catch (error) {
        log(`Export error: ${errorMessage(error)}`, 'error');
    }
}

function exportSceneDocumentToGlb(document: SceneDocument) {
    if (!modules.gltf.loaded) throw new Error('glTF module not loaded');
    const output = serializeSceneDocumentToGlb(document, modules.gltf.module);
    return {
        success: true,
        binary_data: output.binary,
        warnings: output.warnings,
        capabilities: output.capabilities,
        message: 'FBX SceneDocument exported as GLB',
    };
}

function logSceneDocumentCapabilities(capabilities: any = {}) {
    const supported = Object.entries(capabilities)
        .filter(([, value]) => value === true)
        .map(([key]) => key);
    if (supported.length > 0) log(`SceneDocument capabilities: ${supported.join(', ')}`, 'info');
}

function exportGltfDocument(format: string) {
    if (!modules.gltf.loaded) {
        throw new Error('glTF module not loaded');
    }

    const asset = modules.gltf.module.GltfAsset.withResources(
        currentSourceData,
        currentSourceResources,
        '2.1',
    );
    try {
        if (useDraco.checked) {
            if (typeof asset.compressPrimitive !== 'function') {
                throw new Error('Draco encoding is not included in this WASM build');
            }
            for (let mesh = 0; mesh < asset.meshCount(); mesh += 1) {
                const primitiveCount = asset.primitiveCount(mesh);
                for (let primitive = 0; primitive < primitiveCount; primitive += 1) {
                    asset.compressPrimitive(mesh, primitive, Number(encodingSpeed.value), 5);
                }
            }
        }

        if (format === 'glb') {
            return {
                success: true,
                binary_data: asset.glb(2),
                message: useDraco.checked
                    ? 'Document compressed with Draco and exported as GLB'
                    : 'Document packaged and exported as GLB',
            };
        }
        if (format === 'gltf' && currentFileType === 'gltf' && !useDraco.checked) {
            return {
                success: true,
                json_data: new TextDecoder().decode(asset.minifiedJson()),
                message: 'Document exported as minified JSON glTF',
            };
        }
        if (format === 'gltf' && useDraco.checked) {
            throw new Error('Compressed JSON glTF requires bundle download; select GLB instead');
        }
        throw new Error(`Document export to ${format.toUpperCase()} is not supported`);
    } finally {
        asset.free();
    }
}

// Prepare meshes for export
function prepareMeshesForExport(meshes: any[]) {
    const includeNormals = element<HTMLInputElement>('include-normals').checked;
    const includeUvs = element<HTMLInputElement>('include-uvs').checked;
    
    debugLog('prepareMeshesForExport called with', meshes.length, 'meshes');
    for (const mesh of meshes) {
        debugLog('Input mesh:',
            'positions:', mesh.positions?.length,
            'indices:', mesh.indices?.length,
            'normals:', mesh.normals?.length,
            'uvs:', mesh.uvs?.length);
    }
    
    const result = meshes.map((mesh: any, idx) => ({
        name: mesh.name || `mesh_${idx}`,
        positions: Array.from(mesh.positions || []),
        indices: Array.from(mesh.indices || []),
        normals: includeNormals ? Array.from(mesh.normals || []) : null,
        uvs: includeUvs ? Array.from(mesh.uvs || []) : null,
        controlPoints: mesh.controlPoints ? Array.from(mesh.controlPoints) : null,
        polygonVertexIndices: mesh.polygonVertexIndices
            ? Array.from(mesh.polygonVertexIndices)
            : null,
        uvSets: (mesh.uvSets || []).map((set: any) => ({
            name: set.name,
            mapping: set.mapping,
            reference: set.reference,
            values: Array.from(set.values || []),
            indices: Array.from(set.indices || []),
        })),
        normalSets: (mesh.normalSets || []).map((set: any) => ({
            name: set.name,
            mapping: set.mapping,
            reference: set.reference,
            values: Array.from(set.values || []),
            indices: Array.from(set.indices || []),
        })),
        colorSets: (mesh.colorSets || []).map((set: any) => ({
            name: set.name,
            mapping: set.mapping,
            reference: set.reference,
            values: Array.from(set.values || []),
            indices: Array.from(set.indices || []),
        })),
    }));
    
    debugLog('Output meshes:');
    for (const mesh of result) {
        debugLog('Output mesh:',
            'positions:', mesh.positions?.length,
            'indices:', mesh.indices?.length,
            'normals:', mesh.normals?.length,
            'uvs:', mesh.uvs?.length);
    }
    
    return result;
}

function prepareFbxSceneForExport(scene: any, legacyCompatibility = false) {
    const prepareNode = (node: any) => ({
        ...node,
        meshes: (node.meshes || []).map((sourceMesh: any) => {
            const [mesh] = prepareMeshesForExport([sourceMesh]);
            return {
                ...mesh,
            // Keep FBX per-polygon assignments: `fbx-wasm` maps these to
            // LayerElementMaterial when serializing the scene again.
                materialIndices: Array.isArray(sourceMesh.materialIndices)
                ? sourceMesh.materialIndices
                : [],
                skin: sourceMesh.skin || null,
                morphTargets: sourceMesh.morphTargets || [],
            };
        }),
        children: (node.children || []).map(prepareNode),
    });
    const prepared = {
        rootNodes: (scene.rootNodes || []).map(prepareNode),
        materials: (scene.materials || []).map(prepareFbxMaterialForExport),
        textures: scene.textures || [],
        animations: (scene.animations || []).map((animation: any) => prepareFbxAnimationForExport(
            animation, legacyCompatibility,
        )),
    };
    return prepared;
}

/** Strip viewer-only fields and keep what fbx-wasm's MaterialInput accepts. */
function prepareFbxMaterialForExport(material: any) {
    if (!material) return material;
    return {
        name: material.name,
        shadingModel: material.shadingModel,
        diffuse: material.diffuse,
        specular: material.specular,
        emissive: material.emissive,
        ambient: material.ambient,
        diffuseFactor: material.diffuseFactor,
        specularFactor: material.specularFactor,
        shininess: material.shininess,
        emissiveFactor: material.emissiveFactor,
        reflectionFactor: material.reflectionFactor,
        transparencyFactor: material.transparencyFactor,
        opacity: material.opacity,
        bumpFactor: material.bumpFactor,
        textures: material.textures || [],
    };
}

/** Pass animation clips through; fbx-wasm's AnimationInput mirrors the reader. */
function prepareFbxAnimationForExport(animation: any, legacyCompatibility = false) {
    if (!animation) return animation;
    return {
        name: animation.name,
        duration: animation.duration,
        channels: (animation.channels || []).map((channel: any) => ({
            nodeName: channel.nodeName,
            nodeId: channel.nodeId,
            path: channel.path,
            sampler: legacyCompatibility ? {
                ...channel.sampler,
                // Legacy's importer has fragile support for cubic tangents.
                // Preserve key values but write robust linear curves.
                interpolation: 'linear',
                inTangents: null,
                outTangents: null,
            } : channel.sampler,
        })),
    };
}

// Export to OBJ
async function exportToObj(meshes: any[]) {
    if (!modules.obj.loaded) {
        return { success: false, error: 'OBJ module not loaded' };
    }

    const options = {
        include_normals: element<HTMLInputElement>('include-normals').checked,
        include_uvs: element<HTMLInputElement>('include-uvs').checked,
        precision: 6,
    };

    if (meshes.length === 1) {
        return modules.obj.module.create_obj(meshes[0], options);
    } else {
        return modules.obj.module.create_obj_multi(meshes, options);
    }
}

// Export to PLY
async function exportToPly(meshes: any[]) {
    if (!modules.ply.loaded) {
        return { success: false, error: 'PLY module not loaded' };
    }

    // PLY only supports single mesh, merge if multiple
    const merged = mergeMeshes(meshes);

    const options = {
        include_normals: element<HTMLInputElement>('include-normals').checked,
        include_colors: true,
        precision: 6,
        format: 'ascii',
    };

    return modules.ply.module.create_ply(merged, options);
}

// Export to glTF/GLB
async function exportToGltf(meshes: any[], format: string) {
    return {
        success: false,
        error: `Creating ${format.toUpperCase()} from flattened meshes is not part of the document API`,
    };
}

// Export to FBX
async function exportToFbx(meshes: any[]) {
    if (!modules.fbx.loaded) {
        return { success: false, error: 'FBX module not loaded' };
    }

    const options = {
        version: 7500,
    };

    return modules.fbx.module.create_fbx(meshes, options);
}

async function exportToFbxScene(scene: any, legacyCompatibility = false) {
    if (!modules.fbx.loaded) {
        return { success: false, error: 'FBX module not loaded' };
    }
    return modules.fbx.module.create_fbx_scene(scene, { version: 7500, legacyCompatibility });
}

// Merge multiple meshes into one
function mergeMeshes(meshes: any[]) {
    if (meshes.length === 1) return meshes[0];
    
    const merged = {
        name: 'merged',
        positions: [] as number[],
        indices: [] as number[],
        normals: [] as number[],
        uvs: [] as number[],
    };
    
    let vertexOffset = 0;
    
    for (const mesh of meshes) {
        merged.positions.push(...mesh.positions);
        
        if (mesh.indices) {
            for (const idx of mesh.indices) {
                merged.indices.push(idx + vertexOffset);
            }
        }
        
        if (mesh.normals) {
            merged.normals.push(...mesh.normals);
        }
        
        if (mesh.uvs) {
            merged.uvs.push(...mesh.uvs);
        }
        
        vertexOffset += mesh.positions.length / 3;
    }
    
    return merged;
}

// Download the export result
function downloadResult(result: any, format: string) {
    let blob;
    const extension = format === 'fbx-legacy' ? 'fbx' : format;
    let filename = `export.${extension}`;
    
    if (result.binary_data) {
        blob = new Blob([new Uint8Array(result.binary_data)], { type: 'application/octet-stream' });
    } else if (result.json_data) {
        blob = new Blob([result.json_data], { type: 'application/json' });
    } else if (result.data) {
        blob = new Blob([result.data], { type: 'text/plain' });
    } else {
        log('No data to download', 'error');
        return;
    }
    
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
}

// === 3D preview integration ===

function ensureViewer() {
    if (viewer) return viewer;
    try {
        viewer = new Viewer(viewerCanvas, {
            onLog: (msg, type) => log(msg, type),
            onSceneLoaded: (scene) => {
                if (scene) updateAnimationUi(scene);
            },
            onAnimationEnded: () => updateAnimationPlayButton(),
            onAutoRotateChange: syncAutoRotateButton,
        });
        syncViewerToolbar();
    } catch (error) {
        log(`Preview unavailable: ${errorMessage(error)}`, 'error');
        viewer = null;
    }
    return viewer;
}

async function loadPreview(extension: string) {
    viewerSection.classList.add('loaded');
    setViewerControlsEnabled(false);

    // Yield to the browser so the section layout settles before measuring the canvas.
    await new Promise((resolve) => requestAnimationFrame(resolve));

    if (!ensureViewer()) {
        log('Preview unavailable', 'error');
        return;
    }

    try {
        let scene: any;
        if (extension === 'gltf' || extension === 'glb') {
            if (!modules.gltf.loaded) throw new Error('glTF module is not loaded');
            scene = await buildSceneFromGltf(
                currentSourceData!,
                currentSourceResources,
                modules.gltf.module,
                { onLog: (msg, type) => log(msg, type) },
            );
        } else if (extension === 'fbx' && currentMeshData?.scene) {
            scene = await buildSceneFromFbx(
                currentMeshData,
                currentSourceResources,
                { onLog: (msg, type) => log(msg, type) },
            );
        } else if (currentMeshData?.meshes) {
            scene = await buildSceneFromMeshes(
                currentMeshData,
                currentSourceResources,
                { onLog: (msg, type) => log(msg, type) },
            );
        } else {
            throw new Error('No geometry available to preview');
        }

        for (const warning of scene.warnings || []) {
            log(warning, 'warning');
        }

        viewer!.setScene(scene);
        renderSceneDocumentSummary(currentSceneDocument!);
        setWarningSource('preview', scene.warnings || []);
        setViewerControlsEnabled(true);
        syncViewerToolbar();
        log('Preview ready', 'success');
    } catch (error) {
        viewer!.clear();
        setViewerControlsEnabled(false);
        log(`Preview failed: ${errorMessage(error)}`, 'error');
    }
}

function updateAnimationUi(scene: any) {
    const clips = currentSceneDocument?.animations?.length
        ? currentSceneDocument.animations
        : (scene.animations || []);
    resetAnimationUi();
    if (clips.length === 0) return;
    viewerAnimation.style.display = 'flex';
    for (let i = 0; i < clips.length; i++) {
        const option = document.createElement('option');
        option.value = String(i);
        option.textContent = clips[i].name || `Clip ${i + 1}`;
        animClipSelect.appendChild(option);
    }
    animClipSelect.value = String(viewer!.animation.clipIndex);
    rebuildAnimationClipMenu();
    updateAnimationPlayButton();
    updateAnimationScrub();
}

function resetAnimationUi() {
    viewerAnimation.style.display = 'none';
    animClipSelect.innerHTML = '';
    animClipMenu.replaceChildren();
    animClipLabel.textContent = 'Animation';
    closeAnimationClipMenu();
    animTimeLabel.textContent = '0.00s';
    animScrub.value = '0';
    animSpeedValue.textContent = '1.00×';
    animSpeed.value = '100';
    animPlayBtn.classList.remove('active');
    animPlayBtn.title = 'Play';
    animPlayBtn.setAttribute('aria-label', 'Play animation');
}

function rebuildAnimationClipMenu() {
    animClipMenu.replaceChildren();
    for (const option of animClipSelect.options) {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'anim-clip-option';
        button.dataset.value = option.value;
        button.id = `anim-clip-option-${option.value}`;
        button.tabIndex = -1;
        button.setAttribute('role', 'option');
        button.textContent = option.textContent;
        button.addEventListener('click', () => {
            animClipSelect.value = option.value;
            animClipSelect.dispatchEvent(new Event('change', { bubbles: true }));
            closeAnimationClipMenu(true);
        });
        animClipMenu.appendChild(button);
    }
    syncAnimationClipSelection();
}

function syncAnimationClipSelection() {
    const selected = animClipSelect.selectedOptions[0];
    animClipLabel.textContent = selected?.textContent || 'Animation';
    for (const option of animClipMenu.querySelectorAll<HTMLElement>('.anim-clip-option')) {
        const active = option.dataset.value === animClipSelect.value;
        option.classList.toggle('selected', active);
        option.setAttribute('aria-selected', String(active));
        if (active) animClipTrigger.setAttribute('aria-activedescendant', option.id);
    }
}

function openAnimationClipMenu() {
    animClipTrigger.setAttribute('aria-expanded', 'true');
    animClipMenu.hidden = false;
    const selected = animClipMenu.querySelector<HTMLElement>('.anim-clip-option.selected')
        || animClipMenu.querySelector<HTMLElement>('.anim-clip-option');
    selected?.focus();
}

function closeAnimationClipMenu(restoreFocus = false) {
    // Hiding the menu while one of its options holds focus would drop focus to
    // the body, so the trigger takes it back — but only then, otherwise closing
    // would steal focus from whatever the user just clicked.
    const hadFocus = animClipMenu.contains(document.activeElement);
    animClipTrigger.setAttribute('aria-expanded', 'false');
    animClipMenu.hidden = true;
    if (restoreFocus || hadFocus) animClipTrigger.focus();
}

function selectAnimationClipAt(index: number) {
    const options = [...animClipSelect.options];
    if (options.length === 0) return;
    const wrapped = (index + options.length) % options.length;
    animClipSelect.value = options[wrapped].value;
    animClipSelect.dispatchEvent(new Event('change', { bubbles: true }));
}

function handleAnimationClipTriggerKeydown(event: KeyboardEvent) {
    const options = [...animClipSelect.options];
    if (options.length === 0) return;
    const current = Math.max(0, options.findIndex(option => option.value === animClipSelect.value));
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault();
        selectAnimationClipAt(current + (event.key === 'ArrowDown' ? 1 : -1));
    } else if (event.key === 'Home' || event.key === 'End') {
        event.preventDefault();
        selectAnimationClipAt(event.key === 'Home' ? 0 : options.length - 1);
    } else if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        openAnimationClipMenu();
    }
}

function handleAnimationClipMenuKeydown(event: KeyboardEvent) {
    const options = [...animClipMenu.querySelectorAll<HTMLElement>('.anim-clip-option')];
    const current = options.indexOf(document.activeElement as HTMLElement);
    if (event.key === 'Escape') {
        event.preventDefault();
        closeAnimationClipMenu(true);
        return;
    }
    let next = current;
    if (event.key === 'ArrowDown') next = (current + 1) % options.length;
    else if (event.key === 'ArrowUp') next = (current - 1 + options.length) % options.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = options.length - 1;
    else return;
    event.preventDefault();
    options[next]?.focus();
    if (options[next]) {
        animClipSelect.value = options[next].dataset.value!;
        animClipSelect.dispatchEvent(new Event('change', { bubbles: true }));
    }
}

function toggleAnimationPlayback() {
    if (!viewer || !viewer.scene?.animations?.length) return false;
    viewer.animation.playing = !viewer.animation.playing;
    if (viewer.animation.playing && viewer.animation.time >= viewer.scene.animations[viewer.animation.clipIndex].duration) {
        viewer.seekAnimation(0);
    }
    updateAnimationPlayButton();
    return true;
}

/** Space plays and pauses, unless it belongs to the focused control. */
function handlePlaybackShortcut(event: KeyboardEvent) {
    if (event.code !== 'Space' || event.repeat) return;
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    const target = event.target;
    if (target instanceof HTMLElement) {
        if (target.isContentEditable) return;
        // Space is the activation key for buttons, checkboxes and text fields,
        // and the clip listbox picks a clip with it.
        if (/^(BUTTON|INPUT|SELECT|TEXTAREA)$/.test(target.tagName)) return;
        if (!animClipMenu.hidden && animClipMenu.contains(target)) return;
    }
    if (toggleAnimationPlayback()) event.preventDefault();
}

function updateAnimationPlayButton() {
    if (!viewer || !viewer.scene?.animations?.length) return;
    const playing = viewer.animation.playing;
    animPlayBtn.classList.toggle('active', playing);
    animPlayBtn.title = playing ? 'Pause' : 'Play';
    animPlayBtn.setAttribute('aria-label', playing ? 'Pause animation' : 'Play animation');
}

// Animation scrub/timeline ticker — bound to the render loop via rAF.
function animationTick() {
    if (viewer && viewer.scene?.animations?.length && viewer.animation.clipIndex >= 0) {
        updateAnimationScrub();
    }
    requestAnimationFrame(animationTick);
}
requestAnimationFrame(animationTick);

function updateAnimationScrub() {
    if (!viewer || !viewer.scene?.animations?.length) return;
    const clip = viewer.scene.animations[viewer.animation.clipIndex];
    if (!clip) return;
    animTimeLabel.textContent = `${viewer.animation.time.toFixed(2)}s / ${clip.duration.toFixed(2)}s`;
    animScrub.value = String(Math.round((viewer.animation.time / Math.max(clip.duration, 0.0001)) * 1000));
}

// Clear loaded file
function clearFile() {
    currentMeshData = null;
    currentFileType = null;
    currentSourceData = null;
    currentSourceResources = Object.create(null);
    currentSceneDocument = null;
    currentFbxProvenance = null;
    
    fileInfo.style.display = 'none';
    dropZone.style.display = 'grid';
    previewSection.style.display = 'none';
    exportSection.style.display = 'none';
    viewerSection.classList.remove('loaded');
    viewer?.clear();
    setViewerControlsEnabled(false);
    resetAnimationUi();
    scenePanel.hidden = true;
    sceneInfo.hidden = true;
    clearWarningPanel();
    sceneSection.style.display = 'none';
    exportSidebar.style.display = 'none';
    workspace.classList.remove('export-loaded');
    workspace.classList.remove('scene-loaded');

    fileInput.value = '';

    log('File cleared', 'info');
}

// Format file size
function formatFileSize(bytes: number) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / (1024 * 1024)).toFixed(2) + ' MB';
}

// Log to console
function log(message: string, type = 'info') {
    const timestamp = new Date().toLocaleTimeString();
    const line = document.createElement('div');
    line.className = `console-line ${type}`;
    const timestampEl = document.createElement('span');
    timestampEl.className = 'timestamp';
    timestampEl.textContent = `[${timestamp}]`;
    line.append(timestampEl, document.createTextNode(` ${String(message)}`));
    consoleEl.appendChild(line);
    consoleEl.scrollTop = consoleEl.scrollHeight;
}
// Display compression statistics
function displayCompressionStats(stats: any) {
    const statsSection = element('compression-stats');
    // Use proper naming: EdgeBreaker (not Edgebreaker)
    const methodDisplay = stats.method === 'edgebreaker' ? 'EdgeBreaker' : 
                          stats.method === 'sequential' ? 'Sequential' : stats.method;
    element('stats-method').textContent = methodDisplay;
    element('stats-speed').textContent = `${stats.speed} (${stats.speed === 0 ? 'best compression' : stats.speed === 10 ? 'fastest' : 'balanced'})`;
    
    // Display prediction scheme with readable formatting
    const predictionSchemeMap = {
        'DIFFERENCE': 'Difference',
        'PARALLELOGRAM': 'Parallelogram',
        'CONSTRAINED_MULTI_PARALLELOGRAM': 'Constrained Multi-Parallelogram',
        'TEXCOORDS_PORTABLE': 'TexCoords Portable'
    };
    const predictionDisplay = predictionSchemeMap[stats.prediction_scheme as keyof typeof predictionSchemeMap]
        || stats.prediction_scheme || 'Unknown';
    element('stats-prediction').textContent = predictionDisplay;
    
    element('stats-size').textContent = formatFileSize(stats.compressed_size);
    statsSection.style.display = 'block';
    
    log(`Compression: ${methodDisplay} method, speed ${stats.speed}, prediction ${predictionDisplay}, ${formatFileSize(stats.compressed_size)}`, 'success');
}
// Initialize on load
init();
