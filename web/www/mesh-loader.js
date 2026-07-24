/**
 * Convert flat mesh arrays from the OBJ/PLY/FBX WASM parsers into the
 * format-agnostic Scene used by viewer.js.
 *
 * Each parser returns `meshes: [{ positions:f32[], indices:u32[], normals:f32[],
 * uvs:f32[], colors?:u8[] }]`. We wrap every mesh as a primitive under a root
 * node, compute an AABB, and pick a default material based on available
 * attributes.
 */

import { identityMat4, invertMat4, multiplyMat4 } from './mat4.js';

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
        const joints = mesh.joints0?.length === vertexCount * 4 ? Uint16Array.from(mesh.joints0) : null;
        const weights = mesh.weights0?.length === vertexCount * 4 ? Float32Array.from(mesh.weights0) : null;

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
        if (joints && weights) {
            attributes.JOINTS_0 = {
                bytes: joints,
                componentType: 5123,
                components: 4,
                normalized: false,
                count: vertexCount,
            };
            attributes.WEIGHTS_0 = {
                bytes: weights,
                componentType: 5126,
                components: 4,
                normalized: false,
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
                : [...(sourceMaterial?.diffuse || [1, 1, 1]), sourceMaterial?.alpha ?? 1],
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
                    `OBJ texture ${sourceMaterial.baseColorTextureUri} ignored for ${mesh.material}: mesh has no texture coordinates`,
                );
            }
        }

        sceneMeshes.push({
            name: mesh.name || `mesh_${sceneMeshes.length}`,
            primitives: [primitive],
            aabb: localAabb,
            // Keep the sparse source targets alongside the render primitive.
            // FBX export uses these original control-point deltas, while the
            // FBX scene path below creates render-space attributes for WebGL.
            morphTargets: mesh.morphTargets || [],
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

/**
 * Build a viewer scene directly from the FBX model tree returned by fbx-wasm.
 * Geometry remains a shared flat resource list internally, while model nodes,
 * local transforms, materials, textures, animation, and parent-child
 * connections stay intact for rendering and a later FBX export.
 */
export async function buildSceneFromFbx(parsed, resources = Object.create(null), hooks = {}) {
    const roots = parsed?.scene?.rootNodes;
    if (!Array.isArray(roots) || roots.length === 0) {
        return buildSceneFromMeshes(parsed, resources, hooks);
    }

    const flatMeshes = [];
    const collectMeshes = (node) => {
        flatMeshes.push(...(node.meshes || []));
        for (const child of node.children || []) collectMeshes(child);
    };
    for (const root of roots) collectMeshes(root);
    if (flatMeshes.length === 0) {
        throw new Error('No meshes were decoded from this FBX scene');
    }

    const scene = await buildSceneFromMeshes(
        { ...parsed, meshes: flatMeshes },
        resources,
        hooks,
    );

    // Apply FBX materials on top of the fallback materials produced by
    // buildSceneFromMeshes, when the FBX scene carries them.
    const fbxMaterials = parsed?.scene?.materials || parsed?.materials || [];
    if (fbxMaterials.length > 0) {
        const converted = fbxMaterials.map(fbxMaterialToViewer);
        // Replace the per-mesh fallback materials with FBX materials. Each
        // flat mesh's `material` field is an index into fbxMaterials; the
        // fallback path used the mesh index as the material slot, so we map
        // by that index where no explicit material is set.
        scene.materials = scene.meshes.map((mesh, meshIdx) => {
            const meshData = flatMeshes[meshIdx];
            const matIdx = typeof meshData?.material === 'number' ? meshData.material : 0;
            return converted[matIdx] || converted[0] || mesh._defaultMaterial;
        });
        scene.meshes.forEach((mesh, meshIdx) => {
            mesh.primitives.forEach((p) => {
                p.materialIndex = meshIdx;
            });
        });
    }

    // The FBX reader retains morphs in control-point space.  UV/normal seams
    // may duplicate one control point into several render vertices, so build
    // dense GPU attributes from the explicit render expansion emitted by the
    // WASM adapter.  The sparse source remains on `mesh.morphTargets` for a
    // lossless FBX write.
    for (let meshIndex = 0; meshIndex < scene.meshes.length; meshIndex++) {
        const sourceMesh = flatMeshes[meshIndex];
        const primitive = scene.meshes[meshIndex]?.primitives?.[0];
        if (!primitive || !sourceMesh?.morphTargets?.length) continue;
        primitive.morphPositions = [];
        primitive.morphNormals = [];
        const vertexCount = (sourceMesh.positions?.length || 0) / 3;
        for (const target of sourceMesh.morphTargets) {
            const position = new Float32Array(vertexCount * 3);
            const renderIndices = target.renderPointIndices || [];
            const renderDeltas = target.renderPositionDeltas || [];
            for (let entry = 0; entry < renderIndices.length; entry++) {
                const render = renderIndices[entry] * 3;
                const delta = entry * 3;
                if (render + 2 >= position.length || delta + 2 >= renderDeltas.length) continue;
                position[render] = renderDeltas[delta] || 0;
                position[render + 1] = renderDeltas[delta + 1] || 0;
                position[render + 2] = renderDeltas[delta + 2] || 0;
            }
            primitive.morphPositions.push({
                bytes: position,
                componentType: 5126,
                components: 3,
                normalized: false,
                count: vertexCount,
            });
            const normalDeltas = target.renderNormalDeltas;
            if (normalDeltas?.length) {
                const normal = new Float32Array(vertexCount * 3);
                for (let entry = 0; entry < renderIndices.length; entry++) {
                    const render = renderIndices[entry] * 3;
                    const delta = entry * 3;
                    if (render + 2 >= normal.length || delta + 2 >= normalDeltas.length) continue;
                    normal[render] = normalDeltas[delta] || 0;
                    normal[render + 1] = normalDeltas[delta + 1] || 0;
                    normal[render + 2] = normalDeltas[delta + 2] || 0;
                }
                primitive.morphNormals.push({
                    bytes: normal,
                    componentType: 5126,
                    components: 3,
                    normalized: false,
                    count: vertexCount,
                });
            } else {
                primitive.morphNormals.push(null);
            }
        }
    }

    // Decode FBX textures into ImageBitmap-backed viewer textures.
    const fbxTextures = parsed?.scene?.textures || parsed?.textures || [];
    if (fbxTextures.length > 0) {
        const warnings = parsed.warnings || (parsed.warnings = []);
        const textures = await buildFbxTextures(fbxTextures, resources, warnings, hooks);
        if (textures.length > 0) {
            scene.textures = textures;
            // Wire each material's slot binding to its viewer texture index.
            for (let matIdx = 0; matIdx < scene.materials.length && matIdx < fbxMaterials.length; matIdx++) {
                const source = fbxMaterials[matIdx];
                const target = scene.materials[matIdx];
                if (!source?.textures) continue;
                for (const binding of source.textures) {
                    const texIdx = binding.textureIndex;
                    if (!(texIdx in textures)) continue;
                    if (binding.slot === 'diffuse') target.baseColorTexture = texIdx;
                    else if (binding.slot === 'normal') target.normalTexture = { index: texIdx };
                    else if (binding.slot === 'emissive') target.emissiveTexture = { index: texIdx };
                    else if (binding.slot === 'roughness' || binding.slot === 'metallic') {
                        target.metallicRoughnessTexture = { index: texIdx };
                    }
                }
            }
        }
    }

    const nodes = [];
    const renderables = [];
    let meshIndex = 0;

    const nodeByName = new Map();
    const nodeById = new Map();
    // A BindPose stores model *world* matrices. For a skinned FBX it is the
    // authoritative rest pose, including the pre/post rotation evaluation
    // that cannot be recovered from Lcl TRS alone. Reconstructing local
    // matrices from it keeps every joint world at its cluster bind matrix at
    // frame zero; otherwise the shoulder's rotation is inherited as a large
    // positional error by the arm and the torso appears independently hinged.
    const bindPoseByNodeId = new Map();
    const collectBindPoses = (source) => {
        for (const mesh of source.meshes || []) {
            for (const entry of mesh.skin?.bindPose || []) {
                if (typeof entry?.nodeId === 'number'
                    && Array.isArray(entry.matrix)
                    && entry.matrix.length === 16
                    && !bindPoseByNodeId.has(entry.nodeId)) {
                    bindPoseByNodeId.set(entry.nodeId, entry.matrix);
                }
            }
        }
        for (const child of source.children || []) collectBindPoses(child);
    };
    for (const root of roots) collectBindPoses(root);

    const appendNode = (source, parentBindMatrix = null) => {
        const nodeId = typeof source.id === 'number' ? source.id : null;
        const bindMatrix = nodeId === null ? null : bindPoseByNodeId.get(nodeId);
        const sourceMatrix = Array.isArray(source.matrix) && source.matrix.length === 16
            ? source.matrix
            : null;
        const localMatrix = bindMatrix
            ? Float32Array.from(
                parentBindMatrix
                    ? (multiplyMat4(invertMat4(parentBindMatrix), bindMatrix) || bindMatrix)
                    : bindMatrix,
            )
            : sourceMatrix
                ? Float32Array.from(sourceMatrix)
            : null;
        const node = {
            id: nodeId,
            name: source.name || `node_${nodes.length}`,
            // The matrix remains the exact rest transform until animation is
            // evaluated.  Keep a decomposed counterpart too: FBX animation
            // drives Lcl properties, while its static Pre/Post rotations must
            // not disappear when the viewer switches to TRS animation.
            trs: localMatrix ? decomposeFbxMatrix(localMatrix) : restTrs(),
            restTrs: null,
            localMatrix,
            children: [],
            weights: Float32Array.from(
                (source.meshes?.[0]?.morphTargets || []).map((target) =>
                    (Number(target.defaultWeight) || 0) / 100,
                ),
            ),
            meshIndex: -1,
            skinIndex: -1,
            world: new Float32Array(16),
        };
        node.restTrs = cloneTrs(node.trs);
        const nodeIndex = nodes.length;
        nodes.push(node);
        if (source.name) nodeByName.set(source.name, node);
        if (typeof source.id === 'number') nodeById.set(source.id, node);
        for (const mesh of source.meshes || []) {
            // `scene.meshes` was assembled in the same depth-first order.
            renderables.push({ node, meshIndex, skinIndex: -1 });
            if (node.meshIndex < 0) node.meshIndex = meshIndex;
            meshIndex += 1;
        }
        node.children = (source.children || []).map((child) =>
            appendNode(child, bindMatrix || parentBindMatrix),
        );
        return nodeIndex;
    };

    scene.nodes = nodes;
    scene.rootIndices = roots.map(appendNode);
    scene.renderables = renderables;

    // Rebuild the viewer's existing GPU-skin contract from FBX clusters.
    // The WASM output retains every influence; its `joints0`/`weights0`
    // preview attributes are intentionally only the first four.
    const fbxSkins = [];
    let flatMeshIndex = 0;
    const attachSkins = (source, ownerNode) => {
        for (const sourceMesh of source.meshes || []) {
            if (sourceMesh.skin?.clusters?.length) {
                const bindPose = new Map(
                    (sourceMesh.skin.bindPose || []).map((entry) => [entry.nodeId, entry.matrix]),
                );
                const joints = sourceMesh.skin.clusters.map((cluster) => {
                    // Blender/ufbx priority: explicit BindPose matrices are
                    // authoritative; Transform/TransformLink are the FBX
                    // cluster fallback when a pose entry is absent.
                    const meshBind = bindPose.get(ownerNode?.id)
                        || cluster.meshBindTransform
                        || identityMat4();
                    const jointBind = bindPose.get(cluster.jointNodeId)
                        || cluster.jointBindTransform
                        || identityMat4();
                    const inverseJointBind = invertMat4(jointBind) || identityMat4();
                    const inverseBind = multiplyMat4(inverseJointBind, meshBind)
                        || inverseJointBind;
                    return {
                        node: nodeById.get(cluster.jointNodeId),
                        inverseBind: Float32Array.from(inverseBind),
                    };
                });
                if (joints.every((joint) => joint.node)) {
                    const skinIndex = fbxSkins.length;
                    fbxSkins.push({ name: `${sourceMesh.name || 'mesh'}_skin`, joints });
                    renderables[flatMeshIndex].skinIndex = skinIndex;
                }
            }
            flatMeshIndex += 1;
        }
        for (const child of source.children || []) {
            const childNode = typeof child.id === 'number' ? nodeById.get(child.id) : null;
            attachSkins(child, childNode);
        }
    };
    for (const root of roots) {
        const rootNode = typeof root.id === 'number' ? nodeById.get(root.id) : null;
        attachSkins(root, rootNode);
    }
    scene.skins = fbxSkins;

    // Convert FBX animation takes into the glTF-shaped clips the viewer
    // already plays. FBX Euler rotations are emitted in radians (XYZ order,
    // applied as Rz·Ry·Rx); convert to quaternions up-front so the runtime
    // path stays identical to glTF.
    const fbxAnimations = parsed?.scene?.animations || parsed?.animations || [];
    if (fbxAnimations.length > 0) {
        scene.animations = fbxAnimations
            .map((clip) => fbxAnimationToViewer(clip, nodeById, nodeByName))
            .filter(Boolean);
    }

    return scene;
}


/**
 * Convert an FBX Phong/Lambert material into the viewer's glTF-shaped material.
 */
function fbxMaterialToViewer(material, index) {
    const diffuse = material.diffuse || [1, 1, 1];
    const diffuseFactor = typeof material.diffuseFactor === 'number' ? material.diffuseFactor : 1;
    const emissive = material.emissive || [0, 0, 0];
    const emissiveFactor = typeof material.emissiveFactor === 'number' ? material.emissiveFactor : 1;
    // Resolve alpha: FBX uses TransparencyFactor (1 = transparent) or Opacity
    // (1 = opaque). Prefer Opacity when present.
    let alpha = 1;
    if (typeof material.opacity === 'number') {
        alpha = material.opacity;
    } else if (typeof material.transparencyFactor === 'number') {
        alpha = 1 - material.transparencyFactor;
    }
    const alphaMode = alpha < 1 ? 'BLEND' : 'OPAQUE';
    const [dr, dg, db] = diffuse.map((c) => c * diffuseFactor);
    const [er, eg, eb] = emissive.map((c) => c * emissiveFactor);
    // Roughness heuristic mirrored from Blender's io_scene_fbx importer.
    const shininess = typeof material.shininess === 'number' ? Math.max(material.shininess, 0) : 20;
    const roughness = Math.min(1, Math.max(0, 1 - Math.sqrt(shininess) / 10));
    const metallic = typeof material.reflectionFactor === 'number' ? material.reflectionFactor : 0;
    return {
        name: material.name || `material_${index}`,
        baseColorFactor: [dr, dg, db, alpha],
        baseColorTexture: null,
        baseColorTexCoord: 0,
        baseColorTextureTransform: { offset: [0, 0], scale: [1, 1], rotation: 0 },
        metallic,
        roughness,
        metallicRoughnessTexture: null,
        emissiveFactor: [er, eg, eb],
        emissiveTexture: null,
        normalTexture: null,
        occlusionTexture: null,
        doubleSided: false,
        alphaMode,
        alphaCutoff: 0.5,
        unlit: false,
    };
}

/** Decode FBX texture objects (embedded bytes or external filenames). */
async function buildFbxTextures(fbxTextures, resources, warnings, hooks) {
    const textures = [];
    for (let index = 0; index < fbxTextures.length; index++) {
        const texture = fbxTextures[index];
        let bytes = null;
        if (texture.content && texture.content.length > 0) {
            bytes = texture.content;
        } else if (texture.filename) {
            bytes = resolveResource(texture.filename, resources);
            if (!bytes) {
                warnings.push(`FBX texture not selected: ${texture.filename}`);
                continue;
            }
        }
        if (!bytes) continue;
        try {
            const bitmap = await decodeImage(bytes, mimeFromUri(texture.filename || ''));
            textures[index] = {
                name: texture.name || resourceBasename(texture.filename || `texture_${index}`),
                image: bitmap,
                flipY: true,
                wrapS: WebGL2RenderingContext.REPEAT,
                wrapT: WebGL2RenderingContext.REPEAT,
                minFilter: WebGL2RenderingContext.LINEAR_MIPMAP_LINEAR,
                magFilter: WebGL2RenderingContext.LINEAR,
            };
        } catch (error) {
            warnings.push(`Failed to decode FBX texture ${texture.filename}: ${error.message}`);
            hooks.onLog?.(`Failed to decode FBX texture ${texture.filename}: ${error.message}`, 'warning');
        }
    }
    return textures;
}

/** Convert one FBX animation take into a viewer clip. */
function fbxAnimationToViewer(clip, nodeById, nodeByName) {
    const channels = [];
    for (const channel of clip.channels || []) {
        // Names are legal duplicates in FBX; use the object id emitted by
        // WASM first and retain names only for older parser results.
        const node = nodeById.get(channel.nodeId) || nodeByName.get(channel.nodeName);
        if (!node) continue;
        const sampler = channel.sampler || {};
        const input = Float32Array.from(sampler.input || []);
        let output = Float32Array.from(sampler.output || []);
        let interpolation = (sampler.interpolation || 'linear').toUpperCase() === 'CUBIC'
            ? 'CUBICSPLINE'
            : (sampler.interpolation || 'linear').toUpperCase();
        if (channel.path === 'morphweight') {
            const targetIndex = Number.isInteger(channel.morphTargetIndex)
                ? channel.morphTargetIndex
                : 0;
            const targetCount = node.weights?.length || targetIndex + 1;
            const values = [];
            const scalar = output;
            if (interpolation === 'CUBICSPLINE') {
                for (let frame = 0; frame < input.length; frame++) {
                    const inTangent = sampler.inTangents?.[frame] || 0;
                    const value = scalar[frame] || 0;
                    const outTangent = sampler.outTangents?.[frame] || 0;
                    for (const component of [inTangent, value, outTangent]) {
                        for (let target = 0; target < targetCount; target++) {
                            values.push(target === targetIndex ? component / 100 : 0);
                        }
                    }
                }
            } else {
                for (let frame = 0; frame < input.length; frame++) {
                    for (let target = 0; target < targetCount; target++) {
                        values.push(target === targetIndex ? (scalar[frame] || 0) / 100 : 0);
                    }
                }
            }
            channels.push({
                node,
                path: 'weights',
                targetCount,
                sampler: { input, output: Float32Array.from(values), interpolation },
            });
            continue;
        }
        // FBX emits rotation as Euler radians (XYZ order, Rz·Ry·Rx).
        // Convert to quaternions (4 values per frame) so the runtime rotation
        // path matches glTF.
        if (channel.path === 'rotation') {
            const frames = input.length;
            const quatOut = new Float32Array(frames * 4);
            // FBX cubic samplers expose values and tangents as separate
            // arrays.  The viewer's cubic layout is different and Euler
            // tangents cannot be converted component-wise to quaternion
            // tangents.  Keep the authored key values and use quaternion
            // linear interpolation; this is stable for the dense FBX keys
            // used by skeletal clips and avoids feeding tangent data into the
            // Euler value conversion.
            const values = output;
            // `node.restTrs.rotation` is the static FBX rotation basis (the
            // Mixamo rig's PreRotation for this source).  Lcl Rotation keys
            // are absolute authored values in that basis, not deltas from
            // the skin BindPose.  Compose the basis with every raw key;
            // normalizing against the first key silently replaced the
            // animation's opening pose with the T-pose and changed the dance.
            const correction = node.restTrs?.rotation || [0, 0, 0, 1];
            for (let i = 0; i < frames; i++) {
                const rx = values[i * 3] || 0;
                const ry = values[i * 3 + 1] || 0;
                const rz = values[i * 3 + 2] || 0;
                const q = quatMultiply(correction, eulerXyzToQuat(rx, ry, rz));
                quatOut[i * 4] = q[0];
                quatOut[i * 4 + 1] = q[1];
                quatOut[i * 4 + 2] = q[2];
                quatOut[i * 4 + 3] = q[3];
            }
            output = quatOut;
            interpolation = 'LINEAR';
        } else if (channel.path === 'translation') {
            // FBX curves carry the raw Lcl Translation values. The node's
            // rest transform may also include pivots and pre/post rotation,
            // so assigning those raw values directly moves the skeleton away
            // from its BindPose even at the first key. Preserve the authored
            // root-motion delta while anchoring that first key to the same
            // rest translation used for the bind matrix.
            const rest = node.restTrs?.translation || [0, 0, 0];
            const rawRest = [output[0] || 0, output[1] || 0, output[2] || 0];
            const translated = new Float32Array(output.length);
            for (let i = 0; i < input.length; i++) {
                const offset = i * 3;
                translated[offset] = rest[0] + (output[offset] - rawRest[0]);
                translated[offset + 1] = rest[1] + (output[offset + 1] - rawRest[1]);
                translated[offset + 2] = rest[2] + (output[offset + 2] - rawRest[2]);
            }
            output = translated;
            // The semantic FBX decoder exposes cubic tangents separately,
            // whereas viewer.js expects glTF's interleaved cubic layout.
            // Translation values above are deliberately rebased, so passing
            // the value-only FBX array through as CUBICSPLINE makes the
            // viewer read past every third key and turns the root transform
            // into NaN midway through this Mixamo clip.  The project's FBX
            // interoperability profile uses a target-equivalent linear bake
            // for transform curves; retain the dense authored values and
            // sample them linearly just as the rotation adapter does.
            if (interpolation === 'CUBICSPLINE') interpolation = 'LINEAR';
        } else if (interpolation === 'CUBICSPLINE') {
            // `FbxAnimSampler.output` contains only key values.  Interleave
            // FBX's separate in/out tangent arrays into the glTF-shaped
            // [inTangent, value, outTangent] layout consumed by viewer.js.
            const components = channel.path === 'weights'
                ? (channel.targetCount || 1)
                : 3;
            const inTangents = sampler.inTangents || [];
            const outTangents = sampler.outTangents || [];
            const interleaved = new Float32Array(input.length * components * 3);
            for (let frame = 0; frame < input.length; frame++) {
                const source = frame * components;
                const target = frame * components * 3;
                for (let component = 0; component < components; component++) {
                    interleaved[target + component] = inTangents[source + component] || 0;
                    interleaved[target + components + component] = output[source + component] || 0;
                    interleaved[target + components * 2 + component] = outTangents[source + component] || 0;
                }
            }
            output = interleaved;
        }
        channels.push({
            node,
            path: channel.path,
            sampler: { input, output, interpolation },
            targetCount: 3,
        });
    }
    if (channels.length === 0) return null;
    return {
        name: clip.name || `animation_${channels.length}`,
        duration: clip.duration || 0,
        channels,
    };
}

/** Euler XYZ (radians) -> quaternion [x,y,z,w], matching FBX's Rz·Ry·Rx. */
function eulerXyzToQuat(rx, ry, rz) {
    const cx = Math.cos(rx * 0.5), sx = Math.sin(rx * 0.5);
    const cy = Math.cos(ry * 0.5), sy = Math.sin(ry * 0.5);
    const cz = Math.cos(rz * 0.5), sz = Math.sin(rz * 0.5);
    // Rz · Ry · Rx composition. The former qx·qy·qz expansion reverses the
    // authored FBX rotation order, which is most apparent in shoulders and
    // the chained spine.
    return [
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
        cx * cy * cz + sx * sy * sz,
    ];
}

function quatMultiply(a, b) {
    return [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ];
}

function quatInverse(q) {
    const lengthSquared = q[0] ** 2 + q[1] ** 2 + q[2] ** 2 + q[3] ** 2 || 1;
    return [-q[0] / lengthSquared, -q[1] / lengthSquared, -q[2] / lengthSquared, q[3] / lengthSquared];
}

/** Decompose a column-major affine matrix into the preview's TRS shape. */
function decomposeFbxMatrix(matrix) {
    const scale = [
        Math.hypot(matrix[0], matrix[1], matrix[2]) || 1,
        Math.hypot(matrix[4], matrix[5], matrix[6]) || 1,
        Math.hypot(matrix[8], matrix[9], matrix[10]) || 1,
    ];
    const m00 = matrix[0] / scale[0], m01 = matrix[4] / scale[1], m02 = matrix[8] / scale[2];
    const m10 = matrix[1] / scale[0], m11 = matrix[5] / scale[1], m12 = matrix[9] / scale[2];
    const m20 = matrix[2] / scale[0], m21 = matrix[6] / scale[1], m22 = matrix[10] / scale[2];
    const trace = m00 + m11 + m22;
    let rotation;
    if (trace > 0) {
        const s = Math.sqrt(trace + 1) * 2;
        rotation = [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s];
    } else if (m00 > m11 && m00 > m22) {
        const s = Math.sqrt(1 + m00 - m11 - m22) * 2;
        rotation = [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s];
    } else if (m11 > m22) {
        const s = Math.sqrt(1 + m11 - m00 - m22) * 2;
        rotation = [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s];
    } else {
        const s = Math.sqrt(1 + m22 - m00 - m11) * 2;
        rotation = [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s];
    }
    const length = Math.hypot(...rotation) || 1;
    rotation = rotation.map((value) => value / length);
    return {
        translation: [matrix[12], matrix[13], matrix[14]],
        rotation,
        scale,
    };
}

function cloneTrs(trs) {
    return {
        translation: [...trs.translation],
        rotation: [...trs.rotation],
        scale: [...trs.scale],
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
