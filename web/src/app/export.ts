import { formatFileSize } from './format.ts';
import type { SceneDocument } from '../scene-document.ts';
import { buildFbxSceneFromDocument } from '../fbx-scene-document-writer.ts';
import { buildFbxSceneFromGltf, buildFlatMeshesFromGltf } from '../gltf-loader.ts';
import { debugLog, errorMessage, log } from './log.ts';
import { compressionStatFields, compressionStats, dracoOptions, element, encodingSpeed, exportFormat, useDraco } from './dom.ts';
import { modules, state } from './state.ts';
import { serializeSceneDocumentToGlb } from '../scene-document-gltf.ts';
import { setWarningSource } from './warnings.ts';

/**
 * Turning what is loaded into a downloadable file.
 *
 * Two routes reach the writers: a source document keeps its own lossless path,
 * while flattened meshes are prepared, optionally merged, and handed to the
 * format's encoder. Draco settings apply only where the target supports them.
 */

// Update export options based on format
export function updateExportOptions() {
    const format = exportFormat.value;
    
    // Show/hide Draco options for glTF formats only
    if (format === 'gltf' || format === 'glb') {
        dracoOptions.style.display = 'flex';
    } else {
        dracoOptions.style.display = 'none';
    }
}

// Export file
export async function exportFile() {
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
                compressionStats.style.display = 'none';
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

export function exportSceneDocumentToGlb(document: SceneDocument) {
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

export function logSceneDocumentCapabilities(capabilities: any = {}) {
    const supported = Object.entries(capabilities)
        .filter(([, value]) => value === true)
        .map(([key]) => key);
    if (supported.length > 0) log(`SceneDocument capabilities: ${supported.join(', ')}`, 'info');
}

export function exportGltfDocument(format: string) {
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
export function prepareMeshesForExport(meshes: any[]) {
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

export function prepareFbxSceneForExport(scene: any, legacyCompatibility = false) {
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
export function prepareFbxMaterialForExport(material: any) {
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
export function prepareFbxAnimationForExport(animation: any, legacyCompatibility = false) {
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
export async function exportToObj(meshes: any[]) {
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
export async function exportToPly(meshes: any[]) {
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
export async function exportToGltf(meshes: any[], format: string) {
    return {
        success: false,
        error: `Creating ${format.toUpperCase()} from flattened meshes is not part of the document API`,
    };
}

// Export to FBX
export async function exportToFbx(meshes: any[]) {
    if (!modules.fbx.loaded) {
        return { success: false, error: 'FBX module not loaded' };
    }

    const options = {
        version: 7500,
    };

    return modules.fbx.module.create_fbx(meshes, options);
}

export async function exportToFbxScene(scene: any, legacyCompatibility = false) {
    if (!modules.fbx.loaded) {
        return { success: false, error: 'FBX module not loaded' };
    }
    return modules.fbx.module.create_fbx_scene(scene, { version: 7500, legacyCompatibility });
}

// Merge multiple meshes into one
export function mergeMeshes(meshes: any[]) {
    if (meshes.length === 1) return meshes[0];
    
    const merged = {
        name: 'merged',
        positions: [] as number[],
        indices: [] as number[],
        normals: [] as number[],
        uvs: [] as number[],
    };
    
    let vertexOffset = 0;
    
    // Appended one element at a time on purpose. `push(...values)` passes the
    // whole array as arguments and blows the call stack somewhere past a
    // hundred thousand of them, which a mesh reaches at roughly 40k vertices.
    const append = (into: number[], values: ArrayLike<number>) => {
        for (let index = 0; index < values.length; index += 1) into.push(values[index]);
    };

    for (const mesh of meshes) {
        append(merged.positions, mesh.positions);
        
        if (mesh.indices) {
            for (const idx of mesh.indices) {
                merged.indices.push(idx + vertexOffset);
            }
        }
        
        if (mesh.normals) {
            append(merged.normals, mesh.normals);
        }
        
        if (mesh.uvs) {
            append(merged.uvs, mesh.uvs);
        }
        
        vertexOffset += mesh.positions.length / 3;
    }
    
    return merged;
}

// Download the export result
export function downloadResult(result: any, format: string) {
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

// Log to console
// Display compression statistics
export function displayCompressionStats(stats: any) {
    const statsSection = compressionStats;
    // Use proper naming: EdgeBreaker (not Edgebreaker)
    const methodDisplay = stats.method === 'edgebreaker' ? 'EdgeBreaker' : 
                          stats.method === 'sequential' ? 'Sequential' : stats.method;
    compressionStatFields.method.textContent = methodDisplay;
    compressionStatFields.speed.textContent = `${stats.speed} (${stats.speed === 0 ? 'best compression' : stats.speed === 10 ? 'fastest' : 'balanced'})`;
    
    // Display prediction scheme with readable formatting
    const predictionSchemeMap = {
        'DIFFERENCE': 'Difference',
        'PARALLELOGRAM': 'Parallelogram',
        'CONSTRAINED_MULTI_PARALLELOGRAM': 'Constrained Multi-Parallelogram',
        'TEXCOORDS_PORTABLE': 'TexCoords Portable'
    };
    const predictionDisplay = predictionSchemeMap[stats.prediction_scheme as keyof typeof predictionSchemeMap]
        || stats.prediction_scheme || 'Unknown';
    compressionStatFields.prediction.textContent = predictionDisplay;
    
    compressionStatFields.size.textContent = formatFileSize(stats.compressed_size);
    statsSection.style.display = 'block';
    
    log(`Compression: ${methodDisplay} method, speed ${stats.speed}, prediction ${predictionDisplay}, ${formatFileSize(stats.compressed_size)}`, 'success');
}
