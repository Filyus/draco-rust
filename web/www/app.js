/**
 * Draco 3D Format Converter - Main Application
 * 
 * This application loads separate WASM modules for each reader/writer format
 * and provides a unified interface for 3D file format conversion.
 */

// Module state
const modules = {
    objReader: { loaded: false, module: null },
    objWriter: { loaded: false, module: null },
    plyReader: { loaded: false, module: null },
    plyWriter: { loaded: false, module: null },
    gltfReader: { loaded: false, module: null },
    gltfWriter: { loaded: false, module: null },
    fbxReader: { loaded: false, module: null },
    fbxWriter: { loaded: false, module: null },
};

// Current loaded mesh data
let currentMeshData = null;
let currentFileType = null;

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
const dracoOptions = document.getElementById('draco-options');
const dracoSettings = document.getElementById('draco-settings');
const encodingSpeed = document.getElementById('encoding-speed');
const encodingMethod = document.getElementById('encoding-method');
const positionBits = document.getElementById('position-bits');
const normalBits = document.getElementById('normal-bits');
const texcoordBits = document.getElementById('texcoord-bits');
const exportBtn = document.getElementById('export-btn');
const consoleEl = document.getElementById('console');

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
        { key: 'objReader', path: `./pkg/obj_reader.js${CACHE_BUST}`, statusId: 'obj-reader-status' },
        { key: 'objWriter', path: `./pkg/obj_writer.js${CACHE_BUST}`, statusId: 'obj-writer-status' },
        { key: 'plyReader', path: `./pkg/ply_reader.js${CACHE_BUST}`, statusId: 'ply-reader-status' },
        { key: 'plyWriter', path: `./pkg/ply_writer.js${CACHE_BUST}`, statusId: 'ply-writer-status' },
        { key: 'gltfReader', path: `./pkg/gltf_reader.js${CACHE_BUST}`, statusId: 'gltf-reader-status' },
        { key: 'gltfWriter', path: `./pkg/gltf_writer.js${CACHE_BUST}`, statusId: 'gltf-writer-status' },
        { key: 'fbxReader', path: `./pkg/fbx_reader.js${CACHE_BUST}`, statusId: 'fbx-reader-status' },
        { key: 'fbxWriter', path: `./pkg/fbx_writer.js${CACHE_BUST}`, statusId: 'fbx-writer-status' },
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
        await module.default();
        
        modules[key].module = module;
        modules[key].loaded = true;
        
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
        log(`${module.module_name ? module.module_name() : key} v${version} loaded`, 'success');
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
        log(`Failed to load ${key}: ${error.message}`, 'error');
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
            handleFile(files[0]);
        }
    });
    
    // File input
    fileInput.addEventListener('change', (e) => {
        if (e.target.files.length > 0) {
            handleFile(e.target.files[0]);
        }
    });
    
    // Clear file
    clearFileBtn.addEventListener('click', clearFile);
    
    // Export format change
    exportFormat.addEventListener('change', updateExportOptions);
    
    // Draco checkbox
    useDraco.addEventListener('change', () => {
        dracoSettings.style.display = useDraco.checked ? 'block' : 'none';
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
}

// Handle file selection
async function handleFile(file) {
    const extension = file.name.split('.').pop().toLowerCase();
    
    if (!['obj', 'ply', 'gltf', 'glb', 'fbx'].includes(extension)) {
        log(`Unsupported file format: .${extension}`, 'error');
        return;
    }
    
    log(`Loading ${file.name}...`, 'info');
    
    // Show file info
    fileName.textContent = file.name;
    fileSize.textContent = formatFileSize(file.size);
    fileInfo.style.display = 'flex';
    dropZone.style.display = 'none';
    
    currentFileType = extension;
    
    try {
        const arrayBuffer = await file.arrayBuffer();
        const data = new Uint8Array(arrayBuffer);
        
        // Parse file based on extension
        let result;
        switch (extension) {
            case 'obj':
                result = await parseObjFile(data);
                break;
            case 'ply':
                result = await parsePlyFile(data);
                break;
            case 'gltf':
            case 'glb':
                result = await parseGltfFile(data, extension);
                break;
            case 'fbx':
                result = await parseFbxFile(data);
                break;
        }
        
        if (result && result.success) {
            currentMeshData = result;
            displayMeshInfo(result);
            previewSection.style.display = 'block';
            exportSection.style.display = 'block';
            log(`Successfully parsed ${file.name}`, 'success');
        } else {
            log(`Failed to parse file: ${result?.error || 'Unknown error'}`, 'error');
        }
    } catch (error) {
        log(`Error reading file: ${error.message}`, 'error');
    }
}

// Parse OBJ file
async function parseObjFile(data) {
    if (!modules.objReader.loaded) {
        return { success: false, error: 'OBJ Reader module not loaded' };
    }
    
    const textContent = new TextDecoder().decode(data);
    return modules.objReader.module.parse_obj(textContent);
}

// Parse PLY file
async function parsePlyFile(data) {
    if (!modules.plyReader.loaded) {
        return { success: false, error: 'PLY Reader module not loaded' };
    }
    
    const result = modules.plyReader.module.parse_ply_bytes(data);
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
async function parseGltfFile(data, extension) {
    if (!modules.gltfReader.loaded) {
        return { success: false, error: 'glTF Reader module not loaded' };
    }
    
    if (extension === 'glb') {
        return modules.gltfReader.module.parse_glb(data);
    } else {
        const textContent = new TextDecoder().decode(data);
        return modules.gltfReader.module.parse_gltf(textContent);
    }
}

// Parse FBX file
async function parseFbxFile(data) {
    if (!modules.fbxReader.loaded) {
        return { success: false, error: 'FBX Reader module not loaded' };
    }
    
    return modules.fbxReader.module.parse_fbx(data);
}

// Display mesh information
function displayMeshInfo(result) {
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
        dracoOptions.style.display = 'block';
    } else {
        dracoOptions.style.display = 'none';
    }
}

// Export file
async function exportFile() {
    if (!currentMeshData || !currentMeshData.meshes || currentMeshData.meshes.length === 0) {
        log('No mesh data to export', 'error');
        return;
    }
    
    const format = exportFormat.value;
    log(`Exporting to ${format.toUpperCase()}...`, 'info');
    
    try {
        let result;
        const meshes = prepareMeshesForExport(currentMeshData.meshes);
        
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
                result = await exportToFbx(meshes);
                break;
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
            
            log(`Export complete!`, 'success');
        } else {
            log(`Export failed: ${result?.error || 'Unknown error'}`, 'error');
        }
    } catch (error) {
        log(`Export error: ${error.message}`, 'error');
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

// Export to OBJ
async function exportToObj(meshes) {
    if (!modules.objWriter.loaded) {
        return { success: false, error: 'OBJ Writer module not loaded' };
    }
    
    const options = {
        include_normals: document.getElementById('include-normals').checked,
        include_uvs: document.getElementById('include-uvs').checked,
        precision: 6,
    };
    
    if (meshes.length === 1) {
        return modules.objWriter.module.create_obj(meshes[0], options);
    } else {
        return modules.objWriter.module.create_obj_multi(meshes, options);
    }
}

// Export to PLY
async function exportToPly(meshes) {
    if (!modules.plyWriter.loaded) {
        return { success: false, error: 'PLY Writer module not loaded' };
    }
    
    // PLY only supports single mesh, merge if multiple
    const merged = mergeMeshes(meshes);
    
    const options = {
        include_normals: document.getElementById('include-normals').checked,
        include_colors: true,
        precision: 6,
        format: 'ascii',
    };
    
    return modules.plyWriter.module.create_ply(merged, options);
}

// Export to glTF/GLB
async function exportToGltf(meshes, format) {
    if (!modules.gltfWriter.loaded) {
        return { success: false, error: 'glTF Writer module not loaded' };
    }
    
    const options = {
        use_draco: useDraco.checked,
        encoding_speed: parseInt(encodingSpeed.value),
        encoding_method: parseInt(encodingMethod.value),
        position_quantization: parseInt(positionBits.value),
        normal_quantization: parseInt(normalBits.value),
        texcoord_quantization: parseInt(texcoordBits.value),
        format: format,
    };
    
    return modules.gltfWriter.module.create_gltf(meshes, options);
}

// Export to FBX
async function exportToFbx(meshes) {
    if (!modules.fbxWriter.loaded) {
        return { success: false, error: 'FBX Writer module not loaded' };
    }
    
    const options = {
        version: 7500,
    };
    
    return modules.fbxWriter.module.create_fbx(meshes, options);
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
    let filename = `export.${format}`;
    
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

// Clear loaded file
function clearFile() {
    currentMeshData = null;
    currentFileType = null;
    
    fileInfo.style.display = 'none';
    dropZone.style.display = 'block';
    previewSection.style.display = 'none';
    exportSection.style.display = 'none';
    
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
    line.innerHTML = `<span class="timestamp">[${timestamp}]</span> ${message}`;
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
