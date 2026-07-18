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

export function buildSceneFromMeshes(parsed) {
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
        const indices = mesh.indices ? Uint32Array.from(mesh.indices) : null;
        const normals = mesh.normals && mesh.normals.length > 0 ? Float32Array.from(mesh.normals) : null;
        const uvs = mesh.uvs && mesh.uvs.length > 0 ? Float32Array.from(mesh.uvs) : null;
        const colors = mesh.colors && mesh.colors.length > 0 ? Uint8Array.from(mesh.colors) : null;

        const vertexCount = positions.length / 3;
        if (vertexCount === 0) continue;

        for (let i = 0; i < positions.length; i += 3) {
            pushAabb(box, positions[i], positions[i + 1], positions[i + 2]);
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

        const material = {
            baseColorFactor: colors ? [1, 1, 1, 1] : [0.7, 0.78, 0.88, 1],
            doubleSided: false,
            alphaMode: 'OPAQUE',
            unlit: !normals,
        };

        sceneMeshes.push({
            name: mesh.name || `mesh_${sceneMeshes.length}`,
            primitives: [primitive],
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

    return {
        nodes,
        rootIndices: nodes.map((_, i) => i),
        meshes: sceneMeshes,
        skins: [],
        materials,
        textures: [],
        animations: [],
        renderables,
        aabb: box,
        warnings: parsed?.warnings || [],
    };
}

function restTrs() {
    return {
        translation: [0, 0, 0],
        rotation: [0, 0, 0, 1],
        scale: [1, 1, 1],
    };
}
