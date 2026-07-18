/**
 * Convert flat mesh arrays from the OBJ/PLY/FBX WASM parsers into the
 * format-agnostic Scene used by viewer.js.
 *
 * Each parser returns `meshes: [{ positions:f32[], indices:u32[], normals:f32[],
 * uvs:f32[], colors?:u8[] }]`. We wrap every mesh as a primitive under a root
 * node, compute an AABB, and pick a default material based on available
 * attributes.
 */

function pushAabb(box, x, y, z) {
    if (x < box.min[0]) box.min[0] = x;
    if (y < box.min[1]) box.min[1] = y;
    if (z < box.min[2]) box.min[2] = z;
    if (x > box.max[0]) box.max[0] = x;
    if (y > box.max[1]) box.max[1] = y;
    if (z > box.max[2]) box.max[2] = z;
}

export async function buildSceneFromMeshes(parsed, resources = Object.create(null), hooks = {}) {
    const meshes = parsed?.meshes || [];
    if (meshes.length === 0) {
        throw new Error('No meshes were decoded from this file');
    }

    const sceneMeshes = [];
    const box = {
        min: [Infinity, Infinity, Infinity],
        max: [-Infinity, -Infinity, -Infinity],
    };

    for (const mesh of meshes) {
        const positions = Float32Array.from(mesh.positions || []);
        const vertexCount = positions.length / 3;
        const indices = mesh.indices ? Uint32Array.from(mesh.indices) : null;
        const normals = mesh.normals?.length === positions.length ? Float32Array.from(mesh.normals) : null;
        const uvs = mesh.uvs?.length === vertexCount * 2 ? Float32Array.from(mesh.uvs) : null;
        const colors = mesh.colors && mesh.colors.length > 0 ? Uint8Array.from(mesh.colors) : null;

        if (vertexCount === 0) continue;

        const localAabb = {
            min: [Infinity, Infinity, Infinity],
            max: [-Infinity, -Infinity, -Infinity],
        };

        for (let i = 0; i < positions.length; i += 3) {
            pushAabb(box, positions[i], positions[i + 1], positions[i + 2]);
            pushAabb(localAabb, positions[i], positions[i + 1], positions[i + 2]);
        }

        const attributes = {
            POSITION: {
                bytes: positions,
                componentType: 5126,
                components: 3,
                normalized: false,
                count: vertexCount,
            },
        };
        if (normals) {
            attributes.NORMAL = {
                bytes: normals,
                componentType: 5126,
                components: 3,
                normalized: false,
                count: vertexCount,
            };
        }
        if (uvs) {
            attributes.TEXCOORD_0 = {
                bytes: uvs,
                componentType: 5126,
                components: 2,
                normalized: false,
                count: vertexCount,
            };
        }
        if (colors) {
            // PLY ships RGBA as 4 bytes per vertex; treat as normalized.
            const comp = colors.length === vertexCount * 4 ? 4 : 3;
            attributes.COLOR_0 = {
                bytes: colors,
                componentType: 5121,
                components: comp,
                normalized: true,
                count: vertexCount,
            };
        }

        const primitive = {
            attributes,
            mode: 4, // TRIANGLES
            materialIndex: 0,
        };
        if (indices) {
            primitive.indices = {
                bytes: indices,
                componentType: 5125,
                count: indices.length,
            };
        }

        const sourceMaterial = parsed.materials?.[mesh.material];
        const material = {
            baseColorFactor: colors
                ? [1, 1, 1, 1]
                : [...(sourceMaterial?.diffuse || [0.7, 0.78, 0.88]), sourceMaterial?.alpha ?? 1],
            // OBJ/PLY/FBX readers do not carry a material contract. Rendering
            // both sides keeps the diagnostic preview useful for exporters
            // whose triangle winding is opposite to WebGL's default.
            doubleSided: true,
            alphaMode: 'OPAQUE',
            // Without explicit normals the fragment shader derives a face
            // normal from world-space derivatives, so these meshes still have
            // useful diagnostic lighting.
            unlit: false,
        };

        if (sourceMaterial?.baseColorTextureUri) {
            if (uvs) {
                material.baseColorTextureUri = sourceMaterial.baseColorTextureUri;
            } else {
                parsed.warnings ||= [];
                parsed.warnings.push(
                    `OBJ texture ignored for ${mesh.material}: mesh has no texture coordinates`,
                );
            }
        }

        sceneMeshes.push({
            name: mesh.name || `mesh_${sceneMeshes.length}`,
            primitives: [primitive],
            aabb: localAabb,
            _defaultMaterial: material,
        });
    }

    if (!isFinite(box.min[0])) {
        box.min = [-0.5, -0.5, -0.5];
        box.max = [0.5, 0.5, 0.5];
    }

    // Each mesh becomes its own node under a synthetic root.
    const nodes = sceneMeshes.map((mesh, i) => ({
        name: mesh.name,
        trs: restTrs(),
        children: [],
        meshIndex: i,
        skinIndex: -1,
        world: new Float32Array(16),
    }));

    // Collect the first material reference so the viewer can apply it per-primitive.
    const materials = sceneMeshes.map((m) => m._defaultMaterial);
    // Each primitive references its mesh's material index in the global list.
    sceneMeshes.forEach((mesh, meshIdx) => {
        mesh.primitives.forEach((p) => {
            p.materialIndex = meshIdx;
        });
    });

    const renderables = nodes.map((node, i) => ({
        node,
        meshIndex: i,
        skinIndex: -1,
    }));

    const textures = await buildObjTextures(materials, resources, parsed.warnings || (parsed.warnings = []), hooks);
    return {
        nodes,
        rootIndices: nodes.map((_, i) => i),
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
                const bitmap = await decodeImage(bytes, mimeFromUri(uri));
                if (!bitmap) throw new Error('browser could not decode the image');
                index = textures.length;
                textures.push({
                    name: resourceBasename(uri), image: bitmap, flipY: true,
                    wrapS: WebGL2RenderingContext.REPEAT,
                    wrapT: WebGL2RenderingContext.REPEAT,
                    minFilter: WebGL2RenderingContext.LINEAR_MIPMAP_LINEAR,
                    magFilter: WebGL2RenderingContext.LINEAR,
                });
                byUri.set(uri, index);
            } catch (error) {
                warnings.push(`Failed to decode OBJ texture ${uri}: ${error.message}`);
                hooks.onLog?.(`Failed to decode OBJ texture ${uri}: ${error.message}`, 'warning');
                continue;
            }
        }
        material.baseColorTexture = index;
        delete material.baseColorTextureUri;
    }
    return textures;
}

function resolveResource(uri, resources) {
    return resources?.[uri] || resources?.[resourceBasename(uri)] || null;
}

function resourceBasename(path) {
    const slash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    return slash >= 0 ? path.substring(slash + 1) : path;
}

function mimeFromUri(uri) {
    const extension = resourceBasename(uri).split('.').pop()?.toLowerCase();
    return { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', webp: 'image/webp' }[extension]
        || 'application/octet-stream';
}

async function decodeImage(bytes, mime) {
    return createImageBitmap(new Blob([bytes], { type: mime }));
}

function restTrs() {
    return {
        translation: [0, 0, 0],
        rotation: [0, 0, 0, 1],
        scale: [1, 1, 1],
    };
}
