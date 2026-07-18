/**
 * glTF / GLB → Scene loader for the WebGL2 viewer.
 *
 * Uses the gltf-wasm API:
 *   - GltfAsset.withResources(bytes, resources, profile)
 *   - asset.readPrimitive(mesh, primitive) -> PackedGeometry (decoded, incl. Draco)
 *   - asset.readAccessor(index)            -> PackedAccessor (sparse + stride-resolved)
 *   - asset.bufferViewBytes(index)         -> Uint8Array (raw layout, for embedded images)
 *   - asset.json()                          -> lossless JSON document bytes
 *
 * Scene-structure data (nodes, scenes, skins, animations, materials, accessors,
 * images, textures, samplers) is parsed from the lossless JSON client-side.
 * Companion files (.bin / images) and data-URIs come from the supplied
 * `resources` map / inline URIs.
 */

const GL = WebGL2RenderingContext;

const SUPPORTED_PBR_EXTENSIONS = new Set([
    'KHR_materials_unlit',
]);

/**
 * Build a Scene from a parsed glTF document.
 *
 * @param {Uint8Array} sourceData     The .gltf or .glb file bytes.
 * @param {Object} resources          Map of companion filename -> Uint8Array.
 * @param {Object} gltfModule         Imported gltf.js module.
 * @param {Object} hooks              { onLog(msg, type), loadImage(bytes, mime) }
 */
export async function buildSceneFromGltf(sourceData, resources, gltfModule, hooks = {}) {
    const log = (msg, type = 'info') => hooks.onLog?.(msg, type);

    const asset = gltfModule.GltfAsset.withResources(sourceData, resources, '2.1');
    try {
        const json = JSON.parse(new TextDecoder().decode(asset.json()));
        const warnings = [];

        const extensionsUsed = json.extensionsUsed || [];
        const unsupported = extensionsUsed.filter((ext) => !SUPPORTED_PBR_EXTENSIONS.has(ext));
        if (unsupported.length) {
            warnings.push(`Unsupported glTF extensions ignored: ${unsupported.join(', ')}`);
            log(`Unsupported glTF extensions ignored: ${unsupported.join(', ')}`, 'warning');
        }
        if (json.extensionsRequired?.some((ext) => !SUPPORTED_PBR_EXTENSIONS.has(ext))) {
            warnings.push('Model requires extensions that this viewer ignores; rendering may be incomplete');
        }

        const nodes = buildNodes(json, warnings);
        const meshes = buildMeshes(asset, json, warnings);
        const skins = buildSkins(asset, json, nodes, warnings);
        const materials = buildMaterials(json, warnings);
        const textures = await buildTextures(asset, json, resources, hooks);
        const animations = buildAnimations(asset, json, nodes, warnings);

        // Resolve meshes with morph targets — we surface them as a warning.
        for (const meshDef of json.meshes || []) {
            for (const prim of meshDef.primitives || []) {
                if (prim.targets) {
                    warnings.push('Morph target animation is not supported by the preview; targets are ignored');
                    break;
                }
            }
        }

        const scenes = json.scenes || [];
        const sceneIndex = typeof json.scene === 'number' ? json.scene : 0;
        const rootIndices = scenes[sceneIndex]?.nodes || (scenes[0]?.nodes) || [];

        const { renderables, aabb } = computeRenderables(
            nodes,
            meshes,
            skins,
            rootIndices,
        );

        if (animations.length > 0) {
            log(`Loaded ${animations.length} animation clip${animations.length === 1 ? '' : 's'}`, 'info');
        }

        return {
            nodes,
            rootIndices,
            meshes,
            skins,
            materials,
            textures,
            animations,
            renderables,
            aabb,
            warnings,
        };
    } finally {
        asset.free();
    }
}

function buildNodes(json, warnings) {
    const defs = json.nodes || [];
    return defs.map((def, index) => {
        const trs = {
            translation: def.translation ? Array.from(def.translation) : [0, 0, 0],
            rotation: def.rotation ? Array.from(def.rotation) : [0, 0, 0, 1],
            scale: def.scale ? Array.from(def.scale) : [1, 1, 1],
        };
        return {
            name: def.name || `node_${index}`,
            trs,
            // A node may use a static matrix instead of TRS. Keep it intact
            // rather than silently rendering the node at the origin.
            localMatrix: Array.isArray(def.matrix) && def.matrix.length === 16
                ? Float32Array.from(def.matrix)
                : null,
            children: (def.children || []).slice(),
            meshIndex: typeof def.mesh === 'number' ? def.mesh : -1,
            skinIndex: typeof def.skin === 'number' ? def.skin : -1,
            world: new Float32Array(16),
            index,
        };
    });
}

function buildMeshes(asset, json, warnings) {
    const defs = json.meshes || [];
    return defs.map((def, meshIndex) => {
        const primitives = [];
        for (let p = 0; p < def.primitives.length; p++) {
            const packed = asset.readPrimitive(meshIndex, p);
            try {
                const attributes = {};
                for (let i = 0; i < packed.attributeCount(); i++) {
                    const semantic = packed.attributeSemantic(i);
                    attributes[semantic] = {
                        bytes: new Uint8Array(packed.attributeBytes(i)),
                        componentType: packed.attributeComponentType(i),
                        components: packed.attributeComponents(i),
                        normalized: packed.attributeNormalized(i),
                        count: packed.attributeElementCount(i),
                    };
                }
                const primitive = {
                    attributes,
                    mode: packed.mode(),
                    materialIndex: typeof def.primitives[p].material === 'number'
                        ? def.primitives[p].material
                        : -1,
                };
                if (packed.hasIndices()) {
                    primitive.indices = {
                        bytes: new Uint8Array(packed.indexBytes()),
                        componentType: packed.indexComponentType(),
                        count: packed.indexCount(),
                    };
                }
                primitives.push(primitive);
            } finally {
                packed.free();
            }
        }
        return { name: def.name || `mesh_${meshIndex}`, primitives };
    });
}

function readAccessorAsTyped(asset, index) {
    const packed = asset.readAccessor(index);
    try {
        const componentType = packed.componentType();
        const components = packed.components();
        const count = packed.count();
        const bytes = new Uint8Array(packed.bytes());
        const typedView = bytesAsTyped(componentType, bytes);
        return {
            componentType,
            components,
            count,
            normalized: packed.normalized(),
            data: typedView,
        };
    } finally {
        packed.free();
    }
}

function bytesAsTyped(componentType, bytes) {
    switch (componentType) {
        case 5120: return new Int8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        case 5121: return new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        case 5122: return new Int16Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 2);
        case 5123: return new Uint16Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 2);
        case 5125: return new Uint32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 4);
        case 5126: return new Float32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 4);
        default: return new Float32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 4);
    }
}

function buildSkins(asset, json, nodes, warnings) {
    const defs = json.skins || [];
    return defs.map((def, skinIndex) => {
        const joints = (def.joints || []).map((jointNodeIndex) => ({
            node: nodes[jointNodeIndex],
            inverseBind: identityMat4(),
        }));
        if (typeof def.inverseBindMatrices === 'number') {
            try {
                const accessor = readAccessorAsTyped(asset, def.inverseBindMatrices);
                if (accessor.componentType === 5126 && accessor.components === 16) {
                    for (let i = 0; i < joints.length && i < accessor.count; i++) {
                        const src = accessor.data.subarray(i * 16, (i + 1) * 16);
                        joints[i].inverseBind = Float32Array.from(src);
                    }
                }
            } catch (error) {
                warnings.push(`Failed to read skin inverse bind matrices: ${error.message}`);
            }
        }
        return {
            name: def.name || `skin_${skinIndex}`,
            joints,
        };
    });
}

function identityMat4() {
    const m = new Float32Array(16);
    m[0] = m[5] = m[10] = m[15] = 1;
    return m;
}

function buildMaterials(json, warnings) {
    const defs = json.materials || [];
    const fallback = {
        baseColorFactor: [0.8, 0.82, 0.86, 1],
        doubleSided: false,
        alphaMode: 'OPAQUE',
        unlit: false,
    };
    const list = defs.map((def, idx) => {
        const pbr = def.pbrMetallicRoughness || {};
        const unlit = !!(def.extensions && def.extensions.KHR_materials_unlit);
        const texInfo = pbr.baseColorTexture;
        return {
            name: def.name || `material_${idx}`,
            baseColorFactor: pbr.baseColorFactor ? Array.from(pbr.baseColorFactor) : [1, 1, 1, 1],
            baseColorTexture: typeof texInfo?.index === 'number' ? texInfo.index : null,
            baseColorTexCoord: texInfo?.texCoord || 0,
            metallic: pbr.metallicFactor ?? 1,
            roughness: pbr.roughnessFactor ?? 1,
            doubleSided: !!def.doubleSided,
            alphaMode: def.alphaMode || 'OPAQUE',
            alphaCutoff: def.alphaCutoff ?? 0.5,
            unlit,
        };
    });
    list.push(fallback);
    return list;
}

async function buildTextures(asset, json, resources, hooks) {
    const images = await decodeImages(asset, json, resources, hooks);
    const samplers = (json.samplers || []).map((s) => ({
        wrapS: s.wrapS ?? GL.REPEAT,
        wrapT: s.wrapT ?? GL.REPEAT,
        minFilter: s.minFilter ?? GL.LINEAR_MIPMAP_LINEAR,
        magFilter: s.magFilter ?? GL.LINEAR,
    }));
    const defaultSampler = {
        wrapS: GL.REPEAT,
        wrapT: GL.REPEAT,
        minFilter: GL.LINEAR_MIPMAP_LINEAR,
        magFilter: GL.LINEAR,
    };

    return (json.textures || []).map((tex, idx) => {
        const samplerIndex = typeof tex.sampler === 'number' ? tex.sampler : -1;
        const sampler = samplerIndex >= 0 ? samplers[samplerIndex] : defaultSampler;
        const sourceIndex = tex.source;
        const image = typeof sourceIndex === 'number' ? images[sourceIndex] : null;
        return {
            name: tex.name || `texture_${idx}`,
            image: image?.bitmap || null,
            flipY: false,
            wrapS: sampler.wrapS,
            wrapT: sampler.wrapT,
            minFilter: sampler.minFilter,
            magFilter: sampler.magFilter,
        };
    });
}

async function decodeImages(asset, json, resources, hooks) {
    const defs = json.images || [];
    return Promise.all(
        defs.map(async (def) => {
            try {
                if (typeof def.bufferView === 'number') {
                    const bytes = new Uint8Array(asset.bufferViewBytes(def.bufferView));
                    const mime = def.mimeType || sniffMime(bytes);
                    const bitmap = await loadImageBytes(bytes, mime, hooks);
                    return { bitmap, mime };
                }
                if (def.uri) {
                    const bytes = resolveUriBytes(def.uri, resources);
                    if (!bytes) {
                        hooks.onLog?.(`Image not found: ${def.uri}`, 'warning');
                        return { bitmap: null, mime: null };
                    }
                    const mime = def.mimeType || sniffMime(bytes) || mimeFromUri(def.uri);
                    const bitmap = await loadImageBytes(bytes, mime, hooks);
                    return { bitmap, mime };
                }
                return { bitmap: null, mime: null };
            } catch (error) {
                hooks.onLog?.(`Failed to decode image: ${error.message}`, 'warning');
                return { bitmap: null, mime: null };
            }
        }),
    );
}

function sniffMime(bytes) {
    if (bytes.length < 4) return null;
    if (bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return 'image/png';
    if (bytes[0] === 0xff && bytes[1] === 0xd8) return 'image/jpeg';
    if (bytes[0] === 0x52 && bytes[1] === 0x49 && bytes[2] === 0x46 && bytes[3] === 0x46) return 'image/webp';
    if (bytes[0] === 0xab && bytes[1] === 0x4b && bytes[2] === 0x54 && bytes[3] === 0x58) return 'image/ktx2';
    return null;
}

function mimeFromUri(uri) {
    const ext = uri.split('.').pop()?.toLowerCase();
    return { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', webp: 'image/webp', ktx2: 'image/ktx2' }[ext];
}

async function loadImageBytes(bytes, mime, hooks) {
    if (mime === 'image/ktx2') {
        hooks.onLog?.('KTX2 textures require a transcoder; skipping image', 'warning');
        return null;
    }
    const blob = new Blob([bytes], { type: mime || 'application/octet-stream' });
    try {
        return await createImageBitmap(blob);
    } catch (error) {
        hooks.onLog?.(`createImageBitmap failed: ${error.message}`, 'warning');
        return null;
    }
}

function resolveUriBytes(uri, resources) {
    if (uri.startsWith('data:')) {
        const comma = uri.indexOf(',');
        const meta = uri.substring(0, comma);
        const payload = uri.substring(comma + 1);
        if (meta.includes(';base64')) {
            try {
                const bin = atob(payload);
                const bytes = new Uint8Array(bin.length);
                for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
                return bytes;
            } catch (error) {
                return null;
            }
        }
        return new TextEncoder().encode(decodeURIComponent(payload));
    }
    return resources?.[uri] || resources?.[basename(uri)] || null;
}

function basename(path) {
    const slash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    return slash >= 0 ? path.substring(slash + 1) : path;
}

function buildAnimations(asset, json, nodes, warnings) {
    const defs = json.animations || [];
    return defs.map((def, animIndex) => {
        const samplers = (def.samplers || []).map((s) => {
            const input = readAccessorAsTyped(asset, s.input);
            const output = readAccessorAsTyped(asset, s.output);
            return {
                input: input.data,
                output: output.data,
                interpolation: s.interpolation || 'LINEAR',
            };
        });

        const channels = (def.channels || []).map((ch) => {
            const node = nodes[ch.target.node];
            if (!node) return null;
            return {
                node,
                path: ch.target.path,
                sampler: samplers[ch.sampler],
            };
        }).filter(Boolean);

        let duration = 0;
        for (const s of samplers) {
            if (s.input.length > 0) duration = Math.max(duration, s.input[s.input.length - 1]);
        }

        return {
            name: def.name || `animation_${animIndex}`,
            duration,
            channels,
        };
    });
}

function computeRenderables(nodes, meshes, skins, rootIndices) {
    const aabb = {
        min: [Infinity, Infinity, Infinity],
        max: [-Infinity, -Infinity, -Infinity],
    };
    const renderables = [];

    const visited = new Set();
    function walk(nodeIndex) {
        if (visited.has(nodeIndex)) return;
        visited.add(nodeIndex);
        const node = nodes[nodeIndex];
        if (!node) return;
        if (node.meshIndex >= 0 && meshes[node.meshIndex]) {
            const skinIndex = node.skinIndex;
            renderables.push({ node, meshIndex: node.meshIndex, skinIndex });
            accumulateAabb(aabb, meshes[node.meshIndex]);
        }
        for (const child of node.children) walk(child);
    }
    for (const root of rootIndices) walk(root);

    if (!isFinite(aabb.min[0])) {
        aabb.min = [-0.5, -0.5, -0.5];
        aabb.max = [0.5, 0.5, 0.5];
    }

    return { renderables, aabb };
}

function accumulateAabb(box, mesh) {
    for (const prim of mesh.primitives) {
        const pos = prim.attributes.POSITION;
        if (!pos) continue;
        const view = bytesAsTyped(pos.componentType, pos.bytes);
        const components = pos.components;
        for (let i = 0; i < pos.count; i++) {
            const x = view[i * components];
            const y = view[i * components + 1];
            const z = view[i * components + 2];
            if (x < box.min[0]) box.min[0] = x;
            if (y < box.min[1]) box.min[1] = y;
            if (z < box.min[2]) box.min[2] = z;
            if (x > box.max[0]) box.max[0] = x;
            if (y > box.max[1]) box.max[1] = y;
            if (z > box.max[2]) box.max[2] = z;
        }
    }
}
