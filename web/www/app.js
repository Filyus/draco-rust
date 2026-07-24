/**
 * Draco 3D Format Converter - Main Application
 *
 * This application loads separate WASM modules for each reader/writer format
 * and provides a unified interface for 3D file format conversion.
 */

import { Viewer } from './viewer.js';
import { buildFbxSceneFromGltf, buildFlatMeshesFromGltf, buildSceneFromGltf } from './gltf-loader.js';
import { buildSceneFromFbx, buildSceneFromMeshes } from './mesh-loader.js';
import { isAsciiFbx, parseAsciiFbx } from './ascii-fbx-loader.js';

// Module state
const modules = {
    obj: { loaded: false, module: null },
    ply: { loaded: false, module: null },
    gltf: { loaded: false, module: null },
    fbx: { loaded: false, module: null },
};

// Current loaded mesh data
let currentMeshData = null;
let currentFileType = null;
let currentSourceData = null;
let currentSourceResources = Object.create(null);

// 3D preview viewer (lazily created on first use)
let viewer = null;

function errorMessage(error) {
    if (error && typeof error.message === 'string') {
        return error.message;
    }
    return String(error);
}

// DOM Elements
const dropZone = document.getElementById('drop-zone');
const fileInput = document.getElementById('file-input');
const fileInfo = document.getElementById('file-info');
const fileName = document.getElementById('file-name');
const fileSize = document.getElementById('file-size');
const clearFileBtn = document.getElementById('clear-file');
const previewSection = document.getElementById('preview-section');
const exportSection = document.getElementById('export-section');
const exportFormat = document.getElementById('export-format');
const useDraco = document.getElementById('use-draco');
const useDracoLabel = document.getElementById('use-draco-label');
const dracoOptions = document.getElementById('draco-options');
const dracoSettings = document.getElementById('draco-settings');
const encodingSpeed = document.getElementById('encoding-speed');
const encodingMethod = document.getElementById('encoding-method');
const positionBits = document.getElementById('position-bits');
const normalBits = document.getElementById('normal-bits');
const texcoordBits = document.getElementById('texcoord-bits');
const exportBtn = document.getElementById('export-btn');
const consoleEl = document.getElementById('console');

// 3D preview DOM references
const viewerSection = document.getElementById('viewer-section');
const viewerCanvas = document.getElementById('viewer-canvas');
const viewerResetBtn = document.getElementById('viewer-reset');
const viewerAutoRotateBtn = document.getElementById('viewer-autorotate');
const viewerWireframeBtn = document.getElementById('viewer-wireframe');
const viewerBaseColorBtn = document.getElementById('viewer-base-color');
const viewerSmoothNormalsBtn = document.getElementById('viewer-smooth-normals');
const viewerGridBtn = document.getElementById('viewer-grid');
const viewerAnimation = document.getElementById('viewer-animation');
const animPlayBtn = document.getElementById('anim-play');
const animClipSelect = document.getElementById('anim-clip');
const animClipTrigger = document.getElementById('anim-clip-trigger');
const animClipLabel = document.getElementById('anim-clip-label');
const animClipMenu = document.getElementById('anim-clip-menu');
const animLoopCheckbox = document.getElementById('anim-loop');
const animTimeLabel = document.getElementById('anim-time');
const animScrub = document.getElementById('anim-scrub');
const animSpeed = document.getElementById('anim-speed');
const animSpeedValue = document.getElementById('anim-speed-value');
const viewerControls = [
    viewerResetBtn,
    viewerAutoRotateBtn,
    viewerWireframeBtn,
    viewerBaseColorBtn,
    viewerSmoothNormalsBtn,
    viewerGridBtn,
];

function setViewerControlsEnabled(enabled) {
    for (const control of viewerControls) control.disabled = !enabled;
}

function syncViewerToolbar() {
    if (!viewer) return;
    syncAutoRotateButton(viewer.autoRotate);
    for (const [button, enabled] of [
        [viewerWireframeBtn, viewer.wireframe],
        [viewerBaseColorBtn, viewer.baseColorOnly],
        [viewerSmoothNormalsBtn, viewer.smoothNormals],
        [viewerGridBtn, viewer.showGrid],
    ]) {
        button.classList.toggle('active', enabled);
        button.setAttribute('aria-pressed', String(enabled));
    }
}

function syncAutoRotateButton(enabled) {
    viewerAutoRotateBtn.classList.toggle('active', enabled);
    viewerAutoRotateBtn.setAttribute('aria-pressed', String(enabled));
}

function setupChoiceControl(select) {
    const control = document.querySelector(`[data-choice-for="${select.id}"]`);
    if (!control) return;
    const buttons = Array.from(control.querySelectorAll('button[data-value]'));
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
            select.value = button.dataset.value;
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
async function loadModule({ key, path, statusId }) {
    const statusEl = document.getElementById(statusId);
    const indicator = statusEl.querySelector('.status-indicator');
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
        
        const files = e.dataTransfer.files;
        if (files.length > 0) {
            handleFiles(files);
        }
    });
    
    // File input
    fileInput.addEventListener('change', (e) => {
        if (e.target.files.length > 0) {
            handleFiles(e.target.files);
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
    encodingSpeed.addEventListener('input', (e) => {
        document.getElementById('speed-value').textContent = e.target.value;
    });
    positionBits.addEventListener('input', (e) => {
        document.getElementById('position-bits-value').textContent = e.target.value;
    });
    normalBits.addEventListener('input', (e) => {
        document.getElementById('normal-bits-value').textContent = e.target.value;
    });
    texcoordBits.addEventListener('input', (e) => {
        document.getElementById('texcoord-bits-value').textContent = e.target.value;
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
    animPlayBtn.addEventListener('click', () => {
        if (!viewer || !viewer.scene?.animations?.length) return;
        viewer.animation.playing = !viewer.animation.playing;
        if (viewer.animation.playing && viewer.animation.time >= viewer.scene.animations[viewer.animation.clipIndex].duration) {
            viewer.seekAnimation(0);
        }
        updateAnimationPlayButton();
    });
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
    document.addEventListener('click', closeAnimationClipMenu);
    document.addEventListener('keydown', (event) => {
        if (event.key === 'Escape') closeAnimationClipMenu();
    });
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
async function handleFiles(fileList) {
    const files = Array.from(fileList);
    const supportedMain = files.filter((file) =>
        ['obj', 'ply', 'gltf', 'glb', 'fbx'].includes(file.name.split('.').pop().toLowerCase())
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

async function handleFile(file, companionFiles = []) {
    const extension = file.name.split('.').pop().toLowerCase();
    
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
                break;
            case 'fbx':
                result = await parseFbxFile(data);
                break;
        }
        
        if (result && result.success) {
            currentMeshData = result;
            displayMeshInfo(result);
            previewSection.style.display = 'block';
            exportSection.style.display = 'flex';
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
async function parseObjFile(data, resources = Object.create(null)) {
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

function parseObjMaterials(objText, resources, warnings) {
    const materials = Object.create(null);
    const libraries = [];
    for (const line of objText.split(/\r\n|[\r\n]/)) {
        const match = line.trim().match(/^mtllib\s+(.+)$/i);
        if (match) libraries.push(match[1].trim());
    }
    for (const library of libraries) {
        const bytes = resources[library] || resources[resourceBasename(library)];
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
function mtlMapPath(values) {
    const optionValues = {
        '-blendu': 1, '-blendv': 1, '-cc': 1, '-clamp': 1, '-texres': 1,
        '-bm': 1, '-imfchan': 1, '-type': 1, '-mm': 2, '-o': 3, '-s': 3, '-t': 3,
    };
    let index = 0;
    while (index < values.length && values[index].startsWith('-')) {
        index += 1 + (optionValues[values[index].toLowerCase()] ?? 0);
    }
    return values.slice(index).join(' ').trim();
}

function resourceBasename(path) {
    const slash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    return slash >= 0 ? path.substring(slash + 1) : path;
}

// Parse PLY file
async function parsePlyFile(data) {
    if (!modules.ply.loaded) {
        return { success: false, error: 'PLY module not loaded' };
    }

    const result = modules.ply.module.parse_ply_bytes(data);
    console.log('[JS] PLY parse result:', result);
    if (result.meshes) {
        for (const mesh of result.meshes) {
            console.log('[JS] PLY mesh: positions=', mesh.positions?.length, 
                ', indices=', mesh.indices?.length,
                ', normals=', mesh.normals?.length);
        }
    }
    return result;
}

// Parse glTF/GLB file
async function parseGltfFile(data, extension, resources = Object.create(null)) {
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

function triangleCountForMode(mode, elementCount) {
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
async function parseFbxFile(data) {
    if (isAsciiFbx(data)) return parseAsciiFbx(data);
    if (!modules.fbx.loaded) {
        return { success: false, error: 'FBX module not loaded' };
    }

    return modules.fbx.module.parse_fbx(data);
}

// Display mesh information
function displayMeshInfo(result) {
    if (result.document) {
        document.getElementById('mesh-count').textContent = result.meshCount.toLocaleString();
        document.getElementById('vertex-count').textContent = result.vertexCount.toLocaleString();
        document.getElementById('triangle-count').textContent = result.triangleCount.toLocaleString();
        document.getElementById('has-normals').textContent = result.hasNormals ? 'Yes' : 'No';
        document.getElementById('has-uvs').textContent = result.hasUvs ? 'Yes' : 'No';
        document.getElementById('warnings-container').style.display = 'none';
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
    
    document.getElementById('mesh-count').textContent = meshes.length;
    document.getElementById('vertex-count').textContent = totalVertices.toLocaleString();
    document.getElementById('triangle-count').textContent = totalTriangles.toLocaleString();
    document.getElementById('has-normals').textContent = hasNormals ? 'Yes' : 'No';
    document.getElementById('has-uvs').textContent = hasUvs ? 'Yes' : 'No';
    
    // Show warnings
    const warningsContainer = document.getElementById('warnings-container');
    const warningsList = document.getElementById('warnings-list');
    warningsList.innerHTML = '';
    
    if (result.warnings?.length > 0) {
        for (const warning of result.warnings) {
            const li = document.createElement('li');
            li.textContent = warning;
            warningsList.appendChild(li);
        }
        warningsContainer.style.display = 'block';
    } else {
        warningsContainer.style.display = 'none';
    }
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
        if (currentMeshData.document && (format === 'gltf' || format === 'glb')) {
            result = exportGltfDocument(format);
            downloadResult(result, format);
            log(result.message, 'success');
            return;
        }
        const legacyFbx = format === 'fbx-legacy';
        if ((format === 'fbx' || legacyFbx) && currentMeshData.scene) {
            result = await exportToFbxScene(
                prepareFbxSceneForExport(currentMeshData.scene, legacyFbx),
                legacyFbx,
            );
        } else if (currentMeshData.document && (format === 'fbx' || legacyFbx)) {
            const scene = prepareFbxSceneForExport(buildFbxSceneFromGltf(
                currentSourceData,
                currentSourceResources,
                modules.gltf.module,
                { legacyCompatibility: legacyFbx },
            ), legacyFbx);
            result = await exportToFbxScene(scene, legacyFbx);
        } else {
        const sourceMeshes = currentMeshData.document
            ? buildFlatMeshesFromGltf(
                currentSourceData,
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
                document.getElementById('compression-stats').style.display = 'none';
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

function exportGltfDocument(format) {
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
function prepareMeshesForExport(meshes) {
    const includeNormals = document.getElementById('include-normals').checked;
    const includeUvs = document.getElementById('include-uvs').checked;
    
    console.log('[JS] prepareMeshesForExport called with', meshes.length, 'meshes');
    for (const mesh of meshes) {
        console.log('[JS] Input mesh:', 
            'positions:', mesh.positions?.length,
            'indices:', mesh.indices?.length,
            'normals:', mesh.normals?.length,
            'uvs:', mesh.uvs?.length);
    }
    
    const result = meshes.map((mesh, idx) => ({
        name: mesh.name || `mesh_${idx}`,
        positions: Array.from(mesh.positions || []),
        indices: Array.from(mesh.indices || []),
        normals: includeNormals ? Array.from(mesh.normals || []) : null,
        uvs: includeUvs ? Array.from(mesh.uvs || []) : null,
        controlPoints: mesh.controlPoints ? Array.from(mesh.controlPoints) : null,
        polygonVertexIndices: mesh.polygonVertexIndices
            ? Array.from(mesh.polygonVertexIndices)
            : null,
        uvSets: (mesh.uvSets || []).map((set) => ({
            name: set.name,
            mapping: set.mapping,
            reference: set.reference,
            values: Array.from(set.values || []),
            indices: Array.from(set.indices || []),
        })),
        normalSets: (mesh.normalSets || []).map((set) => ({
            name: set.name,
            mapping: set.mapping,
            reference: set.reference,
            values: Array.from(set.values || []),
            indices: Array.from(set.indices || []),
        })),
    }));
    
    console.log('[JS] Output meshes:');
    for (const mesh of result) {
        console.log('[JS] Output mesh:', 
            'positions:', mesh.positions?.length,
            'indices:', mesh.indices?.length,
            'normals:', mesh.normals?.length,
            'uvs:', mesh.uvs?.length);
    }
    
    return result;
}

function prepareFbxSceneForExport(scene, legacyCompatibility = false) {
    const prepareNode = (node) => ({
        ...node,
        meshes: (node.meshes || []).map((sourceMesh) => {
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
        animations: (scene.animations || []).map((animation) => prepareFbxAnimationForExport(
            animation, legacyCompatibility,
        )),
    };
    return prepared;
}

/** Strip viewer-only fields and keep what fbx-wasm's MaterialInput accepts. */
function prepareFbxMaterialForExport(material) {
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
function prepareFbxAnimationForExport(animation, legacyCompatibility = false) {
    if (!animation) return animation;
    return {
        name: animation.name,
        duration: animation.duration,
        channels: (animation.channels || []).map((channel) => ({
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
async function exportToObj(meshes) {
    if (!modules.obj.loaded) {
        return { success: false, error: 'OBJ module not loaded' };
    }

    const options = {
        include_normals: document.getElementById('include-normals').checked,
        include_uvs: document.getElementById('include-uvs').checked,
        precision: 6,
    };

    if (meshes.length === 1) {
        return modules.obj.module.create_obj(meshes[0], options);
    } else {
        return modules.obj.module.create_obj_multi(meshes, options);
    }
}

// Export to PLY
async function exportToPly(meshes) {
    if (!modules.ply.loaded) {
        return { success: false, error: 'PLY module not loaded' };
    }

    // PLY only supports single mesh, merge if multiple
    const merged = mergeMeshes(meshes);

    const options = {
        include_normals: document.getElementById('include-normals').checked,
        include_colors: true,
        precision: 6,
        format: 'ascii',
    };

    return modules.ply.module.create_ply(merged, options);
}

// Export to glTF/GLB
async function exportToGltf(meshes, format) {
    return {
        success: false,
        error: `Creating ${format.toUpperCase()} from flattened meshes is not part of the document API`,
    };
}

// Export to FBX
async function exportToFbx(meshes) {
    if (!modules.fbx.loaded) {
        return { success: false, error: 'FBX module not loaded' };
    }

    const options = {
        version: 7500,
    };

    return modules.fbx.module.create_fbx(meshes, options);
}

async function exportToFbxScene(scene, legacyCompatibility = false) {
    if (!modules.fbx.loaded) {
        return { success: false, error: 'FBX module not loaded' };
    }
    return modules.fbx.module.create_fbx_scene(scene, { version: 7500, legacyCompatibility });
}

// Merge multiple meshes into one
function mergeMeshes(meshes) {
    if (meshes.length === 1) return meshes[0];
    
    const merged = {
        name: 'merged',
        positions: [],
        indices: [],
        normals: [],
        uvs: [],
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
function downloadResult(result, format) {
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

async function loadPreview(extension) {
    viewerSection.classList.add('loaded');
    setViewerControlsEnabled(false);

    // Yield to the browser so the section layout settles before measuring the canvas.
    await new Promise((resolve) => requestAnimationFrame(resolve));

    if (!ensureViewer()) {
        log('Preview unavailable', 'error');
        return;
    }

    try {
        let scene;
        if (extension === 'gltf' || extension === 'glb') {
            if (!modules.gltf.loaded) throw new Error('glTF module is not loaded');
            scene = await buildSceneFromGltf(
                currentSourceData,
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

        viewer.setScene(scene);
        setViewerControlsEnabled(true);
        syncViewerToolbar();
        log('Preview ready', 'success');
    } catch (error) {
        viewer.clear();
        setViewerControlsEnabled(false);
        log(`Preview failed: ${errorMessage(error)}`, 'error');
    }
}

function updateAnimationUi(scene) {
    const clips = scene.animations || [];
    resetAnimationUi();
    if (clips.length === 0) return;
    viewerAnimation.style.display = 'flex';
    for (let i = 0; i < clips.length; i++) {
        const option = document.createElement('option');
        option.value = i;
        option.textContent = clips[i].name || `Clip ${i + 1}`;
        animClipSelect.appendChild(option);
    }
    animClipSelect.value = String(viewer.animation.clipIndex);
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
    animScrub.value = 0;
    animSpeedValue.textContent = '1.00×';
    animSpeed.value = 100;
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
    for (const option of animClipMenu.querySelectorAll('.anim-clip-option')) {
        const active = option.dataset.value === animClipSelect.value;
        option.classList.toggle('selected', active);
        option.setAttribute('aria-selected', String(active));
        if (active) animClipTrigger.setAttribute('aria-activedescendant', option.id);
    }
}

function openAnimationClipMenu() {
    animClipTrigger.setAttribute('aria-expanded', 'true');
    animClipMenu.hidden = false;
    const selected = animClipMenu.querySelector('.anim-clip-option.selected')
        || animClipMenu.querySelector('.anim-clip-option');
    selected?.focus();
}

function closeAnimationClipMenu(restoreFocus = false) {
    animClipTrigger.setAttribute('aria-expanded', 'false');
    animClipMenu.hidden = true;
    if (restoreFocus) animClipTrigger.focus();
}

function selectAnimationClipAt(index) {
    const options = [...animClipSelect.options];
    if (options.length === 0) return;
    const wrapped = (index + options.length) % options.length;
    animClipSelect.value = options[wrapped].value;
    animClipSelect.dispatchEvent(new Event('change', { bubbles: true }));
}

function handleAnimationClipTriggerKeydown(event) {
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

function handleAnimationClipMenuKeydown(event) {
    const options = [...animClipMenu.querySelectorAll('.anim-clip-option')];
    const current = options.indexOf(document.activeElement);
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
        animClipSelect.value = options[next].dataset.value;
        animClipSelect.dispatchEvent(new Event('change', { bubbles: true }));
    }
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
    
    fileInfo.style.display = 'none';
    dropZone.style.display = 'grid';
    previewSection.style.display = 'none';
    exportSection.style.display = 'none';
    viewerSection.classList.remove('loaded');
    viewer?.clear();
    setViewerControlsEnabled(false);
    resetAnimationUi();

    fileInput.value = '';

    log('File cleared', 'info');
}

// Format file size
function formatFileSize(bytes) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    return (bytes / (1024 * 1024)).toFixed(2) + ' MB';
}

// Log to console
function log(message, type = 'info') {
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
function displayCompressionStats(stats) {
    const statsSection = document.getElementById('compression-stats');
    // Use proper naming: EdgeBreaker (not Edgebreaker)
    const methodDisplay = stats.method === 'edgebreaker' ? 'EdgeBreaker' : 
                          stats.method === 'sequential' ? 'Sequential' : stats.method;
    document.getElementById('stats-method').textContent = methodDisplay;
    document.getElementById('stats-speed').textContent = `${stats.speed} (${stats.speed === 0 ? 'best compression' : stats.speed === 10 ? 'fastest' : 'balanced'})`;
    
    // Display prediction scheme with readable formatting
    const predictionSchemeMap = {
        'DIFFERENCE': 'Difference',
        'PARALLELOGRAM': 'Parallelogram',
        'CONSTRAINED_MULTI_PARALLELOGRAM': 'Constrained Multi-Parallelogram',
        'TEXCOORDS_PORTABLE': 'TexCoords Portable'
    };
    const predictionDisplay = predictionSchemeMap[stats.prediction_scheme] || stats.prediction_scheme || 'Unknown';
    document.getElementById('stats-prediction').textContent = predictionDisplay;
    
    document.getElementById('stats-size').textContent = formatFileSize(stats.compressed_size);
    statsSection.style.display = 'block';
    
    log(`Compression: ${methodDisplay} method, speed ${stats.speed}, prediction ${predictionDisplay}, ${formatFileSize(stats.compressed_size)}`, 'success');
}
// Initialize on load
init();
