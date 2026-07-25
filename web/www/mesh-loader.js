/**
 * Format-neutral flat mesh adapter for the viewer.
 *
 * Semantic FBX import lives in fbx-import-scene.js. glTF has its own document
 * adapter in gltf-loader.js; both converge only on viewer's shared Scene.
 */

import { buildSceneFromFbx as buildSemanticFbxScene } from './fbx-import-scene.js';
import { basename, mimeFromUri, resolveResource } from './scene-resources.js';

function pushAabb(box, x, y, z) {
    if (x < box.min[0]) box.min[0] = x;
    if (y < box.min[1]) box.min[1] = y;
    if (z < box.min[2]) box.min[2] = z;
    if (x > box.max[0]) box.max[0] = x;
    if (y > box.max[1]) box.max[1] = y;
    if (z > box.max[2]) box.max[2] = z;
}

/** Build a Scene from flat mesh primitives emitted by OBJ/PLY/legacy FBX. */
export async function buildSceneFromMeshes(parsed, resources = Object.create(null), hooks = {}) {
    const meshes = parsed?.meshes || [];
    if (meshes.length === 0) throw new Error('No meshes were decoded from this file');
    const sceneMeshes = [];
    const box = { min: [Infinity, Infinity, Infinity], max: [-Infinity, -Infinity, -Infinity] };

    for (const mesh of meshes) {
        const positions = Float32Array.from(mesh.positions || []);
        const vertexCount = positions.length / 3;
        const indices = mesh.indices ? Uint32Array.from(mesh.indices) : null;
        const normals = mesh.normals?.length === positions.length ? Float32Array.from(mesh.normals) : null;
        const uvs = mesh.uvs?.length === vertexCount * 2 ? Float32Array.from(mesh.uvs) : null;
        const colors = mesh.colors?.length > 0 ? Uint8Array.from(mesh.colors) : null;
        const joints = mesh.joints0?.length === vertexCount * 4 ? Uint16Array.from(mesh.joints0) : null;
        const weights = mesh.weights0?.length === vertexCount * 4 ? Float32Array.from(mesh.weights0) : null;
        if (mesh.joints1?.length === vertexCount * 4 || mesh.weights1?.length === vertexCount * 4) {
            parsed.warnings ||= [];
            parsed.warnings.push(`Mesh ${mesh.name || sceneMeshes.length} has additional skin influences; preview uses the first four while document/export paths retain the extra set`);
        }
        if (vertexCount === 0) continue;

        const localAabb = { min: [Infinity, Infinity, Infinity], max: [-Infinity, -Infinity, -Infinity] };
        for (let index = 0; index < positions.length; index += 3) {
            pushAabb(box, positions[index], positions[index + 1], positions[index + 2]);
            pushAabb(localAabb, positions[index], positions[index + 1], positions[index + 2]);
        }
        const attributes = {
            POSITION: { bytes: positions, componentType: 5126, components: 3, normalized: false, count: vertexCount },
        };
        if (normals) attributes.NORMAL = { bytes: normals, componentType: 5126, components: 3, normalized: false, count: vertexCount };
        if (uvs) attributes.TEXCOORD_0 = { bytes: uvs, componentType: 5126, components: 2, normalized: false, count: vertexCount };
        if (colors) {
            attributes.COLOR_0 = { bytes: colors, componentType: 5121, components: colors.length === vertexCount * 4 ? 4 : 3, normalized: true, count: vertexCount };
        }
        if (joints && weights) {
            attributes.JOINTS_0 = { bytes: joints, componentType: 5123, components: 4, normalized: false, count: vertexCount };
            attributes.WEIGHTS_0 = { bytes: weights, componentType: 5126, components: 4, normalized: false, count: vertexCount };
        }
        const primitive = { attributes, mode: 4, materialIndex: 0 };
        if (indices) primitive.indices = { bytes: indices, componentType: 5125, count: indices.length };

        const sourceMaterial = parsed.materials?.[mesh.material];
        const material = {
            baseColorFactor: colors ? [1, 1, 1, 1] : [...(sourceMaterial?.diffuse || [1, 1, 1]), sourceMaterial?.alpha ?? 1],
            doubleSided: true,
            alphaMode: 'OPAQUE',
            unlit: false,
        };
        if (sourceMaterial?.baseColorTextureUri) {
            if (uvs) material.baseColorTextureUri = sourceMaterial.baseColorTextureUri;
            else {
                parsed.warnings ||= [];
                parsed.warnings.push(`OBJ texture ${sourceMaterial.baseColorTextureUri} ignored for ${mesh.material}: mesh has no texture coordinates`);
            }
        }
        sceneMeshes.push({
            name: mesh.name || `mesh_${sceneMeshes.length}`,
            primitives: [primitive],
            aabb: localAabb,
            // Retain source sparse targets for FBX export; the FBX semantic
            // adapter creates its render-space expansion separately.
            morphTargets: mesh.morphTargets || [],
            _defaultMaterial: material,
        });
    }
    if (!isFinite(box.min[0])) {
        box.min = [-0.5, -0.5, -0.5];
        box.max = [0.5, 0.5, 0.5];
    }

    const nodes = sceneMeshes.map((mesh, index) => ({
        name: mesh.name,
        trs: restTrs(),
        children: [],
        meshIndex: index,
        skinIndex: -1,
        world: new Float32Array(16),
    }));
    const materials = sceneMeshes.map((mesh) => mesh._defaultMaterial);
    sceneMeshes.forEach((mesh, meshIndex) => mesh.primitives.forEach((primitive) => { primitive.materialIndex = meshIndex; }));
    const renderables = nodes.map((node, meshIndex) => ({ node, meshIndex, skinIndex: -1 }));
    const textures = await buildObjTextures(materials, resources, parsed.warnings || (parsed.warnings = []), hooks);
    return {
        nodes,
        rootIndices: nodes.map((_, index) => index),
        meshes: sceneMeshes,
        skins: [],
        materials,
        textures,
        animations: [],
        renderables,
        aabb: box,
        warnings: parsed?.warnings || [],
    };
}

/** Facade preserving the public FBX import entry point. */
export async function buildSceneFromFbx(parsed, resources = Object.create(null), hooks = {}) {
    return buildSemanticFbxScene(parsed, resources, hooks, buildSceneFromMeshes);
}

async function buildObjTextures(materials, resources, warnings, hooks) {
    const textures = [];
    const byUri = new Map();
    for (const material of materials) {
        const uri = material.baseColorTextureUri;
        if (!uri) continue;
        let index = byUri.get(uri);
        if (index === undefined) {
            const bytes = resolveResource(uri, resources);
            if (!bytes) {
                warnings.push(`OBJ texture not selected: ${uri}`);
                continue;
            }
            try {
                const bitmap = await createImageBitmap(new Blob([bytes], { type: mimeFromUri(uri) || 'application/octet-stream' }));
                if (!bitmap) throw new Error('browser could not decode the image');
                index = textures.length;
                textures.push({
                    name: basename(uri), image: bitmap, flipY: true,
                    wrapS: WebGL2RenderingContext.REPEAT,
                    wrapT: WebGL2RenderingContext.REPEAT,
                    minFilter: WebGL2RenderingContext.LINEAR_MIPMAP_LINEAR,
                    magFilter: WebGL2RenderingContext.LINEAR,
                });
                byUri.set(uri, index);
            } catch (error) {
                const message = `Failed to decode OBJ texture ${uri}: ${error.message}`;
                warnings.push(message);
                hooks.onLog?.(message, 'warning');
                continue;
            }
        }
        material.baseColorTexture = index;
        delete material.baseColorTextureUri;
    }
    return textures;
}

function restTrs() {
    return { translation: [0, 0, 0], rotation: [0, 0, 0, 1], scale: [1, 1, 1] };
}
