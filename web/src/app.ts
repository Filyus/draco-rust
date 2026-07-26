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
import {
    animClipLabel,
    animClipMenu,
    animClipSelect,
    animClipTrigger,
    animLoopCheckbox,
    animPlayBtn,
    animScrub,
    animSpeed,
    animSpeedValue,
    animTimeLabel,
    clearFileBtn,
    dracoOptions,
    dracoSettings,
    dropZone,
    element,
    encodingMethod,
    encodingSpeed,
    exportBtn,
    exportFormat,
    exportSection,
    exportSidebar,
    fileInfo,
    fileInput,
    fileName,
    fileSize,
    normalBits,
    positionBits,
    previewSection,
    sceneInfo,
    scenePanel,
    sceneSection,
    texcoordBits,
    useDraco,
    viewerAnimation,
    viewerAutoRotateBtn,
    viewerBaseColorBtn,
    viewerCanvas,
    viewerControls,
    viewerGridBtn,
    viewerResetBtn,
    viewerSection,
    viewerSmoothNormalsBtn,
    viewerWireframeBtn,
    workspace,
} from './app/dom.ts';
import { debugLog, errorMessage, log } from './app/log.ts';
import { modules, state } from './app/state.ts';
import {
    clearWarningPanel,
    setWarningSource,
} from './app/warnings.ts';
import { loadAllModules, updateDracoEncoderAvailability } from './app/modules.ts';
import { parseFbxFile, parseGltfFile, parseObjFile, parsePlyFile } from './app/parsers.ts';
import {
    describeSceneCapabilities,
    displayMeshInfo,
    renderSceneDocumentSummary,
} from './app/scene-report.ts';

function setViewerControlsEnabled(enabled: boolean) {
    for (const control of viewerControls) control.disabled = !enabled;
}

function syncViewerToolbar() {
    if (!state.viewer) return;
    syncAutoRotateButton(state.viewer.autoRotate);
    const toggles: [HTMLButtonElement, boolean][] = [
        [viewerWireframeBtn, state.viewer.wireframe],
        [viewerBaseColorBtn, state.viewer.baseColorOnly],
        [viewerSmoothNormalsBtn, state.viewer.smoothNormals],
        [viewerGridBtn, state.viewer.showGrid],
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
        if (!state.viewer) return;
        state.viewer.resetView();
    });
    viewerAutoRotateBtn.addEventListener('click', () => {
        if (!state.viewer) return;
        state.viewer.setAutoRotate(!state.viewer.autoRotate);
    });
    viewerWireframeBtn.addEventListener('click', () => {
        if (!state.viewer) return;
        state.viewer.wireframe = !state.viewer.wireframe;
        viewerWireframeBtn.classList.toggle('active', state.viewer.wireframe);
        viewerWireframeBtn.setAttribute('aria-pressed', String(state.viewer.wireframe));
    });
    viewerBaseColorBtn.addEventListener('click', () => {
        if (!state.viewer) return;
        state.viewer.baseColorOnly = !state.viewer.baseColorOnly;
        viewerBaseColorBtn.classList.toggle('active', state.viewer.baseColorOnly);
        viewerBaseColorBtn.setAttribute('aria-pressed', String(state.viewer.baseColorOnly));
    });
    viewerSmoothNormalsBtn.addEventListener('click', () => {
        if (!state.viewer) return;
        state.viewer.smoothNormals = !state.viewer.smoothNormals;
        viewerSmoothNormalsBtn.classList.toggle('active', state.viewer.smoothNormals);
        viewerSmoothNormalsBtn.setAttribute('aria-pressed', String(state.viewer.smoothNormals));
    });
    viewerGridBtn.addEventListener('click', () => {
        if (!state.viewer) return;
        state.viewer.showGrid = !state.viewer.showGrid;
        viewerGridBtn.classList.toggle('active', state.viewer.showGrid);
        viewerGridBtn.setAttribute('aria-pressed', String(state.viewer.showGrid));
    });

    // Animation controls
    animPlayBtn.addEventListener('click', toggleAnimationPlayback);
    animClipSelect.addEventListener('change', () => {
        if (!state.viewer) return;
        const idx = Number(animClipSelect.value);
        state.viewer.animation.clipIndex = idx;
        state.viewer.seekAnimation(0);
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
        if (state.viewer) state.viewer.animation.loop = animLoopCheckbox.checked;
    });
    animScrub.addEventListener('input', () => {
        if (!state.viewer || !state.viewer.scene?.animations?.length) return;
        const clip = state.viewer.scene.animations[state.viewer.animation.clipIndex];
        if (!clip) return;
        state.viewer.animation.playing = false;
        updateAnimationPlayButton();
        const t = (Number(animScrub.value) / 1000) * clip.duration;
        state.viewer.seekAnimation(t);
        updateAnimationScrub();
    });
    animSpeed.addEventListener('input', () => {
        const v = Number(animSpeed.value) / 100;
        if (state.viewer) state.viewer.animation.speed = v;
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
    
    state.currentFileType = extension;
    
    try {
        const arrayBuffer = await file.arrayBuffer();
        const data = new Uint8Array(arrayBuffer);
        state.currentSourceData = new Uint8Array(data);
        state.currentSourceResources = Object.create(null);
        state.currentSceneDocument = null;
        state.currentFbxProvenance = null;
        clearWarningPanel();
        for (const companion of companionFiles) {
            if (Object.prototype.hasOwnProperty.call(state.currentSourceResources, companion.name)) {
                throw new Error(`Duplicate companion resource name: ${companion.name}`);
            }
            state.currentSourceResources[companion.name] = new Uint8Array(await companion.arrayBuffer());
        }
        
        // Parse file based on extension
        let result;
        switch (extension) {
            case 'obj':
                result = await parseObjFile(data, state.currentSourceResources);
                break;
            case 'ply':
                result = await parsePlyFile(data);
                break;
            case 'gltf':
            case 'glb':
                result = await parseGltfFile(data, extension, state.currentSourceResources);
                if (result?.success && result.document) {
                    try {
                        state.currentSceneDocument = buildSceneDocumentFromGltf(data, state.currentSourceResources as Record<string, Uint8Array>, modules.gltf.module);
                    } catch (error) {
                        log(`Scene details unavailable: ${errorMessage(error)}`, 'warning');
                    }
                }
                break;
            case 'fbx':
                result = await parseFbxFile(data);
                if (result?.success && result.scene) {
                    const adapted = buildSceneDocumentWithFbxProvenance(result, state.currentSourceResources);
                    state.currentSceneDocument = adapted.document;
                    state.currentFbxProvenance = adapted.provenance;
                }
                break;
        }
        
        if (result && result.success) {
            state.currentMeshData = result;
            displayMeshInfo(result);
            renderSceneDocumentSummary(state.currentSceneDocument!);
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
    if (!state.currentMeshData) {
        log('No mesh data to export', 'error');
        return;
    }
    
    const format = exportFormat.value;
    log(`Exporting to ${format.toUpperCase()}...`, 'info');
    
    try {
        let result;
        if (format === 'glb' && state.currentFileType === 'fbx' && state.currentSceneDocument) {
            result = exportSceneDocumentToGlb(state.currentSceneDocument);
            for (const warning of result.warnings || []) log(warning, 'warning');
            logSceneDocumentCapabilities(result.capabilities);
            setWarningSource('export', result.warnings || []);
            downloadResult(result, format);
            log(result.message, 'success');
            return;
        }
        if (state.currentMeshData.document && (format === 'gltf' || format === 'glb')) {
            result = exportGltfDocument(format);
            downloadResult(result, format);
            log(result.message, 'success');
            return;
        }
        const legacyFbx = format === 'fbx-legacy';
        if ((format === 'fbx' || legacyFbx) && state.currentFileType === 'fbx' && state.currentSceneDocument) {
            const scene = buildFbxSceneFromDocument(state.currentSceneDocument, { provenance: state.currentFbxProvenance });
            result = await exportToFbxScene(scene, legacyFbx);
        } else if ((format === 'fbx' || legacyFbx) && state.currentMeshData.scene) {
            result = await exportToFbxScene(
                prepareFbxSceneForExport(state.currentMeshData.scene, legacyFbx),
                legacyFbx,
            );
        } else if (state.currentMeshData.document && (format === 'fbx' || legacyFbx)) {
            const scene = prepareFbxSceneForExport(buildFbxSceneFromGltf(
                state.currentSourceData!,
                state.currentSourceResources,
                modules.gltf.module,
                { legacyCompatibility: legacyFbx },
            ), legacyFbx);
            result = await exportToFbxScene(scene, legacyFbx);
        } else {
        const sourceMeshes = state.currentMeshData.document
            ? buildFlatMeshesFromGltf(
                state.currentSourceData!,
                state.currentSourceResources,
                modules.gltf.module,
            )
            : state.currentMeshData.meshes;
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
        state.currentSourceData,
        state.currentSourceResources,
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
        if (format === 'gltf' && state.currentFileType === 'gltf' && !useDraco.checked) {
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

/** Strip state.viewer-only fields and keep what fbx-wasm's MaterialInput accepts. */
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
    if (state.viewer) return state.viewer;
    try {
        state.viewer = new Viewer(viewerCanvas, {
            onLog: (msg: string, type: string) => log(msg, type),
            onSceneLoaded: (scene: any) => {
                if (scene) updateAnimationUi(scene);
            },
            onAnimationEnded: () => updateAnimationPlayButton(),
            onAutoRotateChange: syncAutoRotateButton,
        });
        syncViewerToolbar();
    } catch (error) {
        log(`Preview unavailable: ${errorMessage(error)}`, 'error');
        state.viewer = null;
    }
    return state.viewer;
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
                state.currentSourceData!,
                state.currentSourceResources,
                modules.gltf.module,
                { onLog: (msg: string, type: string) => log(msg, type) },
            );
        } else if (extension === 'fbx' && state.currentMeshData?.scene) {
            scene = await buildSceneFromFbx(
                state.currentMeshData,
                state.currentSourceResources,
                { onLog: (msg: string, type: string) => log(msg, type) },
            );
        } else if (state.currentMeshData?.meshes) {
            scene = await buildSceneFromMeshes(
                state.currentMeshData,
                state.currentSourceResources,
                { onLog: (msg: string, type: string) => log(msg, type) },
            );
        } else {
            throw new Error('No geometry available to preview');
        }

        for (const warning of scene.warnings || []) {
            log(warning, 'warning');
        }

        state.viewer!.setScene(scene);
        renderSceneDocumentSummary(state.currentSceneDocument!);
        setWarningSource('preview', scene.warnings || []);
        setViewerControlsEnabled(true);
        syncViewerToolbar();
        log('Preview ready', 'success');
    } catch (error) {
        state.viewer!.clear();
        setViewerControlsEnabled(false);
        log(`Preview failed: ${errorMessage(error)}`, 'error');
    }
}

function updateAnimationUi(scene: any) {
    const clips = state.currentSceneDocument?.animations?.length
        ? state.currentSceneDocument.animations
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
    animClipSelect.value = String(state.viewer!.animation.clipIndex);
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
    if (!state.viewer || !state.viewer.scene?.animations?.length) return false;
    state.viewer.animation.playing = !state.viewer.animation.playing;
    if (state.viewer.animation.playing && state.viewer.animation.time >= state.viewer.scene.animations[state.viewer.animation.clipIndex].duration) {
        state.viewer.seekAnimation(0);
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
    if (!state.viewer || !state.viewer.scene?.animations?.length) return;
    const playing = state.viewer.animation.playing;
    animPlayBtn.classList.toggle('active', playing);
    animPlayBtn.title = playing ? 'Pause' : 'Play';
    animPlayBtn.setAttribute('aria-label', playing ? 'Pause animation' : 'Play animation');
}

// Animation scrub/timeline ticker — bound to the render loop via rAF.
function animationTick() {
    if (state.viewer && state.viewer.scene?.animations?.length && state.viewer.animation.clipIndex >= 0) {
        updateAnimationScrub();
    }
    requestAnimationFrame(animationTick);
}
requestAnimationFrame(animationTick);

function updateAnimationScrub() {
    if (!state.viewer || !state.viewer.scene?.animations?.length) return;
    const clip = state.viewer.scene.animations[state.viewer.animation.clipIndex];
    if (!clip) return;
    animTimeLabel.textContent = `${state.viewer.animation.time.toFixed(2)}s / ${clip.duration.toFixed(2)}s`;
    animScrub.value = String(Math.round((state.viewer.animation.time / Math.max(clip.duration, 0.0001)) * 1000));
}

// Clear loaded file
function clearFile() {
    state.currentMeshData = null;
    state.currentFileType = null;
    state.currentSourceData = null;
    state.currentSourceResources = Object.create(null);
    state.currentSceneDocument = null;
    state.currentFbxProvenance = null;
    
    fileInfo.style.display = 'none';
    dropZone.style.display = 'grid';
    previewSection.style.display = 'none';
    exportSection.style.display = 'none';
    viewerSection.classList.remove('loaded');
    state.viewer?.clear();
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
