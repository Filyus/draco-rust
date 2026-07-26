/**
 * Vanilla WebGL2 3D preview viewer.
 *
 * Renders the format-agnostic Scene produced by gltf-loader.js / mesh-loader.js.
 * Supports TRS + skinned animation, base color materials (texture + factor +
 * vertex colors), and orbit/touch camera controls. No external dependencies.
 */

import { mat4, vec3, quat, composeMatrix } from './math.js';
import { createEnvironmentIbl } from './environment-ibl.js';

const MAX_JOINTS = 256;
// Morph targets blended in one draw. Deltas are sampled from an array texture,
// so this is a shader loop bound rather than a hardware limit; a mesh may
// declare any number of targets and the strongest-weighted ones are blended.
const MAX_ACTIVE_MORPH_TARGETS = 32;
// Texture unit for the morph delta array. Units 0..4 are material maps and
// 5..8 belong to the environment IBL.
const MORPH_TEXTURE_UNIT = 9;
const DEFAULT_CAMERA_AZIMUTH = Math.PI * 0.25;
const DEFAULT_CAMERA_ELEVATION = Math.PI * 0.09;
const ORBIT_RAD_PER_PIXEL = 0.01;
// Movement keys cross this fraction of the orbit distance per second, so the
// same tap feels alike on a small prop and on a whole level.
const FLY_DISTANCE_PER_SECOND = 0.4;
const ORBIT_RAD_PER_SECOND = 1.2;
// Keys the viewport claims while focused, so they never scroll the page.
const NAV_KEYS = new Set([
    'KeyW', 'KeyA', 'KeyS', 'KeyD', 'KeyQ', 'KeyE',
    'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight',
]);

const VERT_SRC = `#version 300 es
precision highp float;

layout(location=0) in vec3 aPosition;
layout(location=1) in vec3 aNormal;
layout(location=2) in vec2 aTexCoord;
layout(location=6) in vec2 aTexCoord1;
layout(location=3) in vec4 aColor;
layout(location=4) in vec4 aJoints;
layout(location=5) in vec4 aWeights;
layout(location=15) in vec3 aSmoothNormal;

uniform mat4 uProjection;
uniform mat4 uView;
uniform mat4 uModel;
uniform mat4 uNormalMatrix;
uniform int uUseSkin;
uniform int uJointCount;
uniform mat4 uJointMatrix[${MAX_JOINTS}];
// Morph deltas live in an array texture: one layer per target, uMorphStride
// texels per vertex (position, plus a normal when the asset ships one).
uniform highp sampler2DArray uMorphDeltas;
uniform int uMorphCount;
uniform int uMorphStride;
uniform int uMorphWidth;
uniform float uMorphWeights[${MAX_ACTIVE_MORPH_TARGETS}];
uniform int uMorphLayers[${MAX_ACTIVE_MORPH_TARGETS}];
uniform int uUseSmoothNormals;

out vec3 vNormal;
out vec2 vTexCoord;
out vec2 vTexCoord1;
out vec4 vColor;
out vec3 vWorldPos;

vec3 morphDelta(int texel, int layer) {
    return texelFetch(uMorphDeltas, ivec3(texel % uMorphWidth, texel / uMorphWidth, layer), 0).xyz;
}

void main() {
    vec3 morphedPosition = aPosition;
    vec3 morphedNormal = uUseSmoothNormals == 1 ? aSmoothNormal : aNormal;
    for (int i = 0; i < uMorphCount; i++) {
        int texel = gl_VertexID * uMorphStride;
        float weight = uMorphWeights[i];
        int layer = uMorphLayers[i];
        morphedPosition += morphDelta(texel, layer) * weight;
        if (uMorphStride > 1) morphedNormal += morphDelta(texel + 1, layer) * weight;
    }
    vec4 skinned = vec4(0.0);
    if (uUseSkin == 1 && uJointCount > 0) {
        vec4 pos = vec4(morphedPosition, 1.0);
        skinned +=
            (uJointMatrix[int(aJoints.x)] * pos) * aWeights.x +
            (uJointMatrix[int(aJoints.y)] * pos) * aWeights.y +
            (uJointMatrix[int(aJoints.z)] * pos) * aWeights.z +
            (uJointMatrix[int(aJoints.w)] * pos) * aWeights.w;

        vec4 nrm = vec4(morphedNormal, 0.0);
        vec3 skinnedNormal =
            (uJointMatrix[int(aJoints.x)] * nrm).xyz * aWeights.x +
            (uJointMatrix[int(aJoints.y)] * nrm).xyz * aWeights.y +
            (uJointMatrix[int(aJoints.z)] * nrm).xyz * aWeights.z +
            (uJointMatrix[int(aJoints.w)] * nrm).xyz * aWeights.w;
        vNormal = normalize((uNormalMatrix * vec4(skinnedNormal, 0.0)).xyz);
    } else {
        skinned = vec4(morphedPosition, 1.0);
        vNormal = normalize((uNormalMatrix * vec4(morphedNormal, 0.0)).xyz);
    }

    vec4 worldPos = uModel * skinned;
    vWorldPos = worldPos.xyz;
    vTexCoord = aTexCoord;
    vTexCoord1 = aTexCoord1;
    vColor = aColor;
    gl_Position = uProjection * uView * worldPos;
}
`;

const FRAG_SRC = `#version 300 es
precision highp float;

in vec3 vNormal;
in vec2 vTexCoord;
in vec2 vTexCoord1;
in vec4 vColor;
in vec3 vWorldPos;

uniform int uHasTexture;
uniform int uHasNormals;
uniform int uHasVertexColors;
uniform int uUnlit;
uniform int uBaseColorOnly;
uniform sampler2D uBaseColor;
uniform vec4 uBaseColorFactor;
uniform int uBaseColorTexCoord;
uniform vec2 uBaseColorTexOffset;
uniform vec2 uBaseColorTexScale;
uniform float uBaseColorTexRotation;
uniform int uHasMetallicRoughnessTexture;
uniform sampler2D uMetallicRoughness;
uniform int uMetallicRoughnessTexCoord;
uniform float uMetallic;
uniform float uRoughness;
uniform int uHasEmissiveTexture;
uniform sampler2D uEmissive;
uniform int uEmissiveTexCoord;
uniform vec3 uEmissiveFactor;
uniform int uHasNormalTexture;
uniform sampler2D uNormalTexture;
uniform int uNormalTexCoord;
uniform float uNormalScale;
uniform int uHasOcclusionTexture;
uniform sampler2D uOcclusionTexture;
uniform int uOcclusionTexCoord;
uniform float uOcclusionStrength;
uniform samplerCube uIrradianceMap;
uniform samplerCube uPrefilteredMap;
uniform sampler2D uBrdfLut;
uniform float uEnvironmentMaxLod;
uniform vec3 uCameraPos;

out vec4 outColor;

const float PI = 3.14159265359;

vec2 selectUv(int texCoord) {
    return texCoord == 1 ? vTexCoord1 : vTexCoord;
}

vec3 fresnelSchlickRoughness(float cosTheta, vec3 f0, float roughness) {
    return f0 + (max(vec3(1.0 - roughness), f0) - f0) * pow(1.0 - cosTheta, 5.0);
}

vec3 acesToneMap(vec3 color) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((color * (a * color + b)) / (color * (c * color + d) + e), 0.0, 1.0);
}

void main() {
    vec4 base = uBaseColorFactor;
    if (uHasVertexColors == 1) base *= vColor;
    vec4 baseSample = vec4(1.0);
    if (uHasTexture == 1) {
        vec2 uv = selectUv(uBaseColorTexCoord);
        uv *= uBaseColorTexScale;
        float c = cos(uBaseColorTexRotation);
        float s = sin(uBaseColorTexRotation);
        uv = vec2(c * uv.x - s * uv.y, s * uv.x + c * uv.y) + uBaseColorTexOffset;
        baseSample = texture(uBaseColor, uv);
    }
    base *= baseSample;

    // Hard unlit materials (KHR_materials_unlit) keep flat shading.
    if (uUnlit == 1 || uBaseColorOnly == 1) {
        outColor = vec4(base.rgb, base.a);
        return;
    }

    // Pick a surface normal. When the geometry supplies normals, use them
    // (smooth shading). Otherwise derive a per-face normal from screen-space
    // derivatives of the world position so even normal-less meshes show form.
    vec3 N;
    if (uHasNormals == 1) {
        N = normalize(vNormal);
    } else {
        vec3 dx = dFdx(vWorldPos);
        vec3 dy = dFdy(vWorldPos);
        N = normalize(cross(dx, dy));
    }
    // OBJ/PLY/FBX preview meshes are deliberately two-sided because their
    // source winding is not a rendering contract. Match glTF's double-sided
    // material rule so the visible side receives the same lighting either way.
    if (!gl_FrontFacing) N = -N;

    if (uHasNormalTexture == 1) {
        vec2 uv = selectUv(uNormalTexCoord);
        vec3 tangentNormal = texture(uNormalTexture, uv).xyz * 2.0 - 1.0;
        tangentNormal.xy *= uNormalScale;
        vec3 dpdx = dFdx(vWorldPos);
        vec3 dpdy = dFdy(vWorldPos);
        vec2 duvdx = dFdx(uv);
        vec2 duvdy = dFdy(uv);
        vec3 T = dpdx * duvdy.y - dpdy * duvdx.y;
        vec3 B = -dpdx * duvdy.x + dpdy * duvdx.x;
        if (dot(T, T) > 0.000001 && dot(B, B) > 0.000001) {
            T = normalize(T);
            B = normalize(B);
            N = normalize(mat3(T, B, N) * tangentNormal);
        }
    }

    vec3 V = normalize(uCameraPos - vWorldPos);
    vec3 baseColor = uBaseColorFactor.rgb;
    if (uHasVertexColors == 1) baseColor *= vColor.rgb;
    if (uHasTexture == 1) baseColor *= pow(baseSample.rgb, vec3(2.2));

    float metallic = uMetallic;
    float roughness = clamp(uRoughness, 0.045, 1.0);
    if (uHasMetallicRoughnessTexture == 1) {
        vec4 packed = texture(uMetallicRoughness, selectUv(uMetallicRoughnessTexCoord));
        roughness = clamp(roughness * packed.g, 0.045, 1.0);
        metallic *= packed.b;
    }

    float occlusion = 1.0;
    if (uHasOcclusionTexture == 1) {
        occlusion = mix(1.0, texture(uOcclusionTexture, selectUv(uOcclusionTexCoord)).r, uOcclusionStrength);
    }
    vec3 f0 = mix(vec3(0.04), baseColor, metallic);
    float nDotV = max(dot(N, V), 0.0);
    vec3 iblFresnel = fresnelSchlickRoughness(nDotV, f0, roughness);
    vec3 diffuseWeight = (1.0 - iblFresnel) * (1.0 - metallic);
    vec3 irradiance = texture(uIrradianceMap, N).rgb;
    vec3 diffuseIbl = irradiance * baseColor / PI;
    vec3 reflected = reflect(-V, N);
    vec3 prefiltered = textureLod(uPrefilteredMap, reflected, roughness * uEnvironmentMaxLod).rgb;
    vec2 brdf = texture(uBrdfLut, vec2(nDotV, roughness)).rg;
    vec3 specularIbl = prefiltered * (f0 * brdf.x + brdf.y);
    vec3 color = (diffuseWeight * diffuseIbl + specularIbl) * occlusion;
    vec3 emissive = uEmissiveFactor;
    if (uHasEmissiveTexture == 1) {
        emissive *= pow(texture(uEmissive, selectUv(uEmissiveTexCoord)).rgb, vec3(2.2));
    }
    color += emissive;
    color = acesToneMap(color);
    outColor = vec4(pow(color, vec3(1.0 / 2.2)), base.a);
}
`;

const LINE_VERT_SRC = `#version 300 es
precision highp float;
layout(location=0) in vec3 aPosition;
uniform mat4 uProjectionView;
void main() {
    gl_Position = uProjectionView * vec4(aPosition, 1.0);
}
`;

const LINE_FRAG_SRC = `#version 300 es
precision highp float;
uniform vec3 uColor;
out vec4 outColor;
void main() {
    outColor = vec4(uColor, 1.0);
}
`;

const BACKGROUND_VERT_SRC = `#version 300 es
precision highp float;
out vec2 vNdc;
void main() {
    vec2 positions[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    vNdc = positions[gl_VertexID];
    gl_Position = vec4(vNdc, 0.0, 1.0);
}
`;

const BACKGROUND_FRAG_SRC = `#version 300 es
precision highp float;
in vec2 vNdc;
uniform mat4 uInverseProjection;
uniform mat4 uInverseView;
uniform samplerCube uEnvironment;
out vec4 outColor;

vec3 acesToneMap(vec3 color) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((color * (a * color + b)) / (color * (c * color + d) + e), 0.0, 1.0);
}

void main() {
    vec4 view = uInverseProjection * vec4(vNdc, 1.0, 1.0);
    view /= view.w;
    vec3 direction = normalize((uInverseView * vec4(view.xyz, 0.0)).xyz);
    vec3 color = acesToneMap(textureLod(uEnvironment, direction, 0.0).rgb);
    outColor = vec4(pow(color, vec3(1.0 / 2.2)), 1.0);
}
`;

const GL = WebGL2RenderingContext;

function compileShader(gl, type, source) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        const log = gl.getShaderInfoLog(shader);
        gl.deleteShader(shader);
        throw new Error(`Shader compile error: ${log}`);
    }
    return shader;
}

function linkProgram(gl, vert, frag) {
    const program = gl.createProgram();
    gl.attachShader(program, compileShader(gl, gl.VERTEX_SHADER, vert));
    gl.attachShader(program, compileShader(gl, gl.FRAGMENT_SHADER, frag));
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
        const log = gl.getProgramInfoLog(program);
        gl.deleteProgram(program);
        throw new Error(`Program link error: ${log}`);
    }
    return program;
}

/** Return the original byte layout of an ArrayBuffer or typed-array view. */
function byteView(data) {
    if (data instanceof Uint8Array) return data;
    if (ArrayBuffer.isView(data)) {
        return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    }
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    throw new Error('attribute payload is not binary data');
}

export function buildSmoothNormalAttribute(primitive) {
    const positions = primitive.attributes.POSITION;
    const normals = primitive.attributes.NORMAL;
    if (primitive.mode !== 4 || !positions
        || positions.componentType !== 5126 || positions.components !== 3
        || (normals && (normals.componentType !== 5126 || normals.components !== 3
            || positions.count !== normals.count))
        || positions.count === 0) {
        return null;
    }

    const count = positions.count;
    const positionBytes = byteView(positions.bytes);
    const normalBytes = normals ? byteView(normals.bytes) : null;
    if (positionBytes.byteLength !== count * 12
        || (normalBytes && normalBytes.byteLength !== count * 12)) return null;
    const positionView = new DataView(positionBytes.buffer, positionBytes.byteOffset, positionBytes.byteLength);
    const normalView = normalBytes
        ? new DataView(normalBytes.buffer, normalBytes.byteOffset, normalBytes.byteLength)
        : null;
    const position = (index, axis) => positionView.getFloat32((index * 3 + axis) * 4, true);
    const sourceNormal = (index, axis) => normalView
        ? normalView.getFloat32((index * 3 + axis) * 4, true)
        : (axis === 1 ? 1 : 0);

    // Join exactly coincident vertices for preview smoothing. When the asset
    // supplies normals, retain authored creases of 60 degrees or more instead
    // of rounding deliberately split cube edges and other hard surfaces.
    const groupIds = new Uint32Array(count);
    const groups = new Map();
    const contributions = [];
    for (let i = 0; i < count; i++) {
        const key = `${position(i, 0)},${position(i, 1)},${position(i, 2)}`;
        let group = groups.get(key);
        if (group === undefined) {
            group = contributions.length;
            groups.set(key, group);
            contributions.push([]);
        }
        groupIds[i] = group;
    }

    const indices = primitive.indices;
    let indexCount = count;
    let indexAt = (index) => index;
    if (indices) {
        const bytes = byteView(indices.bytes);
        const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        indexCount = indices.count;
        if (indices.componentType === 5121 && bytes.byteLength === indexCount) {
            indexAt = (index) => view.getUint8(index);
        } else if (indices.componentType === 5123 && bytes.byteLength === indexCount * 2) {
            indexAt = (index) => view.getUint16(index * 2, true);
        } else if (indices.componentType === 5125 && bytes.byteLength === indexCount * 4) {
            indexAt = (index) => view.getUint32(index * 4, true);
        } else {
            return null;
        }
    }
    if (indexCount % 3 !== 0) return null;

    const cornerAngle = (ax, ay, az, bx, by, bz) => {
        const divisor = Math.hypot(ax, ay, az) * Math.hypot(bx, by, bz);
        if (divisor <= 1e-12) return 0;
        return Math.acos(Math.max(-1, Math.min(1, (ax * bx + ay * by + az * bz) / divisor)));
    };
    for (let offset = 0; offset < indexCount; offset += 3) {
        const vertices = [indexAt(offset), indexAt(offset + 1), indexAt(offset + 2)];
        if (vertices.some((index) => index >= count)) return null;
        const points = vertices.map((index) => [position(index, 0), position(index, 1), position(index, 2)]);
        const edge1 = points[1].map((value, axis) => value - points[0][axis]);
        const edge2 = points[2].map((value, axis) => value - points[0][axis]);
        let face = [
            edge1[1] * edge2[2] - edge1[2] * edge2[1],
            edge1[2] * edge2[0] - edge1[0] * edge2[2],
            edge1[0] * edge2[1] - edge1[1] * edge2[0],
        ];
        const faceLength = Math.hypot(...face);
        if (faceLength <= 1e-12) continue;
        face = face.map((value) => value / faceLength);
        for (let corner = 0; corner < 3; corner++) {
            const point = points[corner];
            const a = points[(corner + 1) % 3].map((value, axis) => value - point[axis]);
            const b = points[(corner + 2) % 3].map((value, axis) => value - point[axis]);
            const weight = cornerAngle(...a, ...b);
            contributions[groupIds[vertices[corner]]].push([
                face[0],
                face[1],
                face[2],
                weight,
            ]);
        }
    }

    const output = new Float32Array(count * 3);
    const creaseCosine = Math.cos(Math.PI / 3);
    for (let i = 0; i < count; i++) {
        let reference = null;
        if (normalView) {
            const length = Math.hypot(sourceNormal(i, 0), sourceNormal(i, 1), sourceNormal(i, 2));
            if (length > 1e-12) {
                reference = [
                    sourceNormal(i, 0) / length,
                    sourceNormal(i, 1) / length,
                    sourceNormal(i, 2) / length,
                ];
            }
        }
        const sum = [0, 0, 0];
        for (const [x, y, z, weight] of contributions[groupIds[i]]) {
            if (reference && x * reference[0] + y * reference[1] + z * reference[2]
                < creaseCosine - 1e-6) {
                continue;
            }
            sum[0] += x * weight;
            sum[1] += y * weight;
            sum[2] += z * weight;
        }
        const length = Math.hypot(...sum);
        for (let axis = 0; axis < 3; axis++) {
            output[i * 3 + axis] = length > 1e-12 ? sum[axis] / length : sourceNormal(i, axis);
        }
        const dot = normalView ? output[i * 3] * sourceNormal(i, 0)
            + output[i * 3 + 1] * sourceNormal(i, 1)
            + output[i * 3 + 2] * sourceNormal(i, 2) : 1;
        if (dot < 0) {
            output[i * 3] *= -1;
            output[i * 3 + 1] *= -1;
            output[i * 3 + 2] *= -1;
        }
    }
    return { bytes: output, componentType: 5126, components: 3, normalized: false, count };
}

/**
 * Copy one morph accessor into its texel slot of a packed layer. Both loaders
 * reject targets that are not float vec3, and accessor bytes can start at an
 * unaligned offset, so the payload is read through a DataView.
 */
function fillMorphLayer(layer, attr, vertexCount, stride, slot) {
    if (!attr || attr.componentType !== 5126 || attr.components !== 3) return false;
    const bytes = byteView(attr.bytes);
    const count = Math.min(vertexCount, attr.count);
    if (bytes.byteLength < count * 12) return false;
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    for (let i = 0; i < count; i++) {
        const texel = (i * stride + slot) * 4;
        layer[texel] = view.getFloat32(i * 12, true);
        layer[texel + 1] = view.getFloat32(i * 12 + 4, true);
        layer[texel + 2] = view.getFloat32(i * 12 + 8, true);
    }
    return true;
}

/**
 * Pack every morph target of a primitive into one RGBA32F array texture: a
 * layer per target, `stride` texels per vertex, addressed by gl_VertexID.
 *
 * Vertex attributes are capped at 16 by WebGL2 and the preview already spends
 * them all, so attribute-fed deltas could never exceed four targets. A texture
 * has no such budget, which is what lets a mesh declare any number of targets.
 */
function uploadMorphTexture(gl, primitive, vertexCount) {
    const positions = primitive.morphPositions || [];
    const normals = primitive.morphNormals || [];
    const targetCount = Math.max(positions.length, normals.length);
    if (targetCount === 0 || vertexCount === 0) return null;

    const layerCount = Math.min(targetCount, gl.getParameter(gl.MAX_ARRAY_TEXTURE_LAYERS));
    const stride = normals.some(Boolean) ? 2 : 1;
    const texels = vertexCount * stride;
    const maxSize = gl.getParameter(gl.MAX_TEXTURE_SIZE);
    const width = Math.max(1, Math.min(maxSize, Math.ceil(Math.sqrt(texels))));
    const height = Math.ceil(texels / width);
    if (height > maxSize) return null;

    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, texture);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texStorage3D(gl.TEXTURE_2D_ARRAY, 1, gl.RGBA32F, width, height, layerCount);

    const layer = new Float32Array(width * height * 4);
    // A target whose accessor was rejected upstream stays an all-zero layer, so
    // its weight simply contributes nothing.
    const filled = new Array(layerCount).fill(false);
    for (let target = 0; target < layerCount; target++) {
        layer.fill(0);
        filled[target] = fillMorphLayer(layer, positions[target], vertexCount, stride, 0);
        if (stride > 1) {
            filled[target] = fillMorphLayer(layer, normals[target], vertexCount, stride, 1)
                || filled[target];
        }
        gl.texSubImage3D(
            gl.TEXTURE_2D_ARRAY, 0, 0, 0, target, width, height, 1, gl.RGBA, gl.FLOAT, layer,
        );
    }
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, null);
    return { texture, width, stride, layerCount, filled, dropped: targetCount - layerCount };
}

/**
 * Build GPU buffers for one Mesh primitive.
 * Returns an object describing attribute locations, VAO, index/element counts.
 */
function uploadPrimitive(gl, primitive, locationMap) {
    const vao = gl.createVertexArray();
    gl.bindVertexArray(vao);

    const buffers = [];
    const positions = primitive.attributes.POSITION;
    if (!positions) throw new Error('primitive is missing POSITION attribute');

    function bindAccessor(attr, semantic, location, desiredComponents) {
        if (!attr || location < 0) return false;
        if (desiredComponents && attr.components !== desiredComponents) {
            gl.disableVertexAttribArray(location);
            return false;
        }
        const buf = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, buf);
        gl.bufferData(gl.ARRAY_BUFFER, byteView(attr.bytes), gl.STATIC_DRAW);
        buffers.push(buf);
        const normalized = attr.normalized
            || semantic.startsWith('COLOR_') || semantic.startsWith('WEIGHTS_');
        gl.enableVertexAttribArray(location);
        gl.vertexAttribPointer(location, attr.components, attr.componentType, normalized, 0, 0);
        return true;
    }

    function bindAttribute(semantic, location, desiredComponents) {
        return bindAccessor(primitive.attributes[semantic], semantic, location, desiredComponents);
    }

    const layout = {
        position: locationMap.position,
        normal: locationMap.normal,
        texCoord: locationMap.texCoord,
        texCoord1: locationMap.texCoord1,
        color: locationMap.color,
        joints: locationMap.joints,
        weights: locationMap.weights,
        smoothNormal: locationMap.smoothNormal,
    };

    const info = {
        vao,
        buffers,
        hasNormals: !!bindAttribute('NORMAL', layout.normal),
        hasSmoothNormals: !!bindAccessor(
            buildSmoothNormalAttribute(primitive),
            'SMOOTH_NORMAL',
            layout.smoothNormal,
            3,
        ),
        hasTexCoords0: !!bindAttribute('TEXCOORD_0', layout.texCoord),
        hasTexCoords1: !!bindAttribute('TEXCOORD_1', layout.texCoord1),
        hasColors: !!bindAttribute('COLOR_0', layout.color),
        hasJoints: !!bindAttribute('JOINTS_0', layout.joints),
        hasWeights: !!bindAttribute('WEIGHTS_0', layout.weights),
        mode: primitive.mode,
        elementCount: 0,
        indexed: false,
    };

    bindAttribute('POSITION', layout.position);
    // Layers are indexed exactly like the mesh weights, so picking targets at
    // draw time is a plain lookup by target index.
    info.morph = uploadMorphTexture(gl, primitive, positions.count);
    info.morphTargetCount = info.morph ? info.morph.layerCount : 0;

    let indexBuffer = null;
    if (primitive.indices) {
        const idx = primitive.indices;
        const bytes = byteView(idx.bytes);
        indexBuffer = gl.createBuffer();
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, indexBuffer);
        gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, bytes, gl.STATIC_DRAW);
        info.indexed = true;
        info.elementCount = idx.count;
        info.indexType = idx.componentType;
        buffers.push(indexBuffer);
    } else {
        info.indexed = false;
        info.elementCount = positions.count;
    }

    gl.bindVertexArray(null);
    return info;
}

export class Viewer {
    constructor(canvas, hooks = {}) {
        this.canvas = canvas;
        this.hooks = hooks; // { onSceneLoaded(scene), onError(msg), onLog(msg, type), onAutoRotateChange(enabled) }
        this.gl = canvas.getContext('webgl2', {
            antialias: true,
            alpha: true,
            premultipliedAlpha: false,
            preserveDrawingBuffer: false,
        });
        if (!this.gl) throw new Error('WebGL2 is not supported in this browser');

        this._setupGl();
        this._buildPrograms();

        this.scene = null;
        this.glResources = null;

        // Camera state (orbit)
        this.camera = {
            target: vec3.set(vec3.create(), 0, 0, 0),
            distance: 3,
            azimuth: DEFAULT_CAMERA_AZIMUTH,
            elevation: DEFAULT_CAMERA_ELEVATION,
            fov: Math.PI / 4,
            near: 0.05,
            far: 1000,
            // Dolly limits track the scene size; see `_fitCameraToScene`.
            minDistance: 0.05,
            maxDistance: 1000,
        };
        // Scratch vectors for the camera basis used by pan and keyboard flight.
        this._basisRight = vec3.create();
        this._basisUp = vec3.create();
        this._basisForward = vec3.create();
        this._pivotScratch = vec3.create();
        // Navigation keys currently held down, by KeyboardEvent.code.
        this._navKeys = new Set();
        this._navFast = false;
        this._navSlow = false;

        // Animation
        this.animation = {
            playing: false,
            clipIndex: -1,
            time: 0,
            speed: 1.0,
            loop: true,
        };

        // Display options
        this.wireframe = false;
        this.showGrid = true;
        // Diagnostic mode: display base color data without preview lighting.
        this.baseColorOnly = false;
        // Preview-friendly angle-weighted normals can be disabled to inspect
        // the exact normals authored in the source asset.
        this.smoothNormals = false;
        this.autoRotate = false;

        // Matrices
        this._projection = mat4.create();
        this._view = mat4.create();
        this._projectionView = mat4.create();
        this._inverseProjection = mat4.create();
        this._inverseView = mat4.create();
        this._scratch = mat4.create();
        this._normalMatrix = mat4.create();
        this._model = mat4.create();

        // Controls
        this._setupControls();
        this._setupResize();

        this._running = true;
        this._lastTime = performance.now();
        this._loop = this._loop.bind(this);
        requestAnimationFrame(this._loop);
    }

    _setupGl() {
        const gl = this.gl;
        gl.enable(gl.DEPTH_TEST);
        gl.depthFunc(gl.LEQUAL);
        gl.enable(gl.CULL_FACE);
        gl.cullFace(gl.BACK);
        // The background pass writes an opaque, tone-mapped environment.
        gl.clearColor(0, 0, 0, 0);
    }

    _buildPrograms() {
        this.program = linkProgram(this.gl, VERT_SRC, FRAG_SRC);
        this.lineProgram = linkProgram(this.gl, LINE_VERT_SRC, LINE_FRAG_SRC);
        this.backgroundProgram = linkProgram(this.gl, BACKGROUND_VERT_SRC, BACKGROUND_FRAG_SRC);

        const gl = this.gl;
        const p = this.program;
        this.uniforms = {
            uProjection: gl.getUniformLocation(p, 'uProjection'),
            uView: gl.getUniformLocation(p, 'uView'),
            uModel: gl.getUniformLocation(p, 'uModel'),
            uNormalMatrix: gl.getUniformLocation(p, 'uNormalMatrix'),
            uUseSkin: gl.getUniformLocation(p, 'uUseSkin'),
            uJointCount: gl.getUniformLocation(p, 'uJointCount'),
            uJointMatrix: gl.getUniformLocation(p, `uJointMatrix[0]`),
            uMorphDeltas: gl.getUniformLocation(p, 'uMorphDeltas'),
            uMorphCount: gl.getUniformLocation(p, 'uMorphCount'),
            uMorphStride: gl.getUniformLocation(p, 'uMorphStride'),
            uMorphWidth: gl.getUniformLocation(p, 'uMorphWidth'),
            uMorphWeights: gl.getUniformLocation(p, 'uMorphWeights[0]'),
            uMorphLayers: gl.getUniformLocation(p, 'uMorphLayers[0]'),
            uUseSmoothNormals: gl.getUniformLocation(p, 'uUseSmoothNormals'),
            uHasTexture: gl.getUniformLocation(p, 'uHasTexture'),
            uHasNormals: gl.getUniformLocation(p, 'uHasNormals'),
            uHasVertexColors: gl.getUniformLocation(p, 'uHasVertexColors'),
            uUnlit: gl.getUniformLocation(p, 'uUnlit'),
            uBaseColorOnly: gl.getUniformLocation(p, 'uBaseColorOnly'),
            uBaseColor: gl.getUniformLocation(p, 'uBaseColor'),
            uBaseColorFactor: gl.getUniformLocation(p, 'uBaseColorFactor'),
            uBaseColorTexCoord: gl.getUniformLocation(p, 'uBaseColorTexCoord'),
            uBaseColorTexOffset: gl.getUniformLocation(p, 'uBaseColorTexOffset'),
            uBaseColorTexScale: gl.getUniformLocation(p, 'uBaseColorTexScale'),
            uBaseColorTexRotation: gl.getUniformLocation(p, 'uBaseColorTexRotation'),
            uHasMetallicRoughnessTexture: gl.getUniformLocation(p, 'uHasMetallicRoughnessTexture'),
            uMetallicRoughness: gl.getUniformLocation(p, 'uMetallicRoughness'),
            uMetallicRoughnessTexCoord: gl.getUniformLocation(p, 'uMetallicRoughnessTexCoord'),
            uMetallic: gl.getUniformLocation(p, 'uMetallic'),
            uRoughness: gl.getUniformLocation(p, 'uRoughness'),
            uHasEmissiveTexture: gl.getUniformLocation(p, 'uHasEmissiveTexture'),
            uEmissive: gl.getUniformLocation(p, 'uEmissive'),
            uEmissiveTexCoord: gl.getUniformLocation(p, 'uEmissiveTexCoord'),
            uEmissiveFactor: gl.getUniformLocation(p, 'uEmissiveFactor'),
            uHasNormalTexture: gl.getUniformLocation(p, 'uHasNormalTexture'),
            uNormalTexture: gl.getUniformLocation(p, 'uNormalTexture'),
            uNormalTexCoord: gl.getUniformLocation(p, 'uNormalTexCoord'),
            uNormalScale: gl.getUniformLocation(p, 'uNormalScale'),
            uHasOcclusionTexture: gl.getUniformLocation(p, 'uHasOcclusionTexture'),
            uOcclusionTexture: gl.getUniformLocation(p, 'uOcclusionTexture'),
            uOcclusionTexCoord: gl.getUniformLocation(p, 'uOcclusionTexCoord'),
            uOcclusionStrength: gl.getUniformLocation(p, 'uOcclusionStrength'),
            uIrradianceMap: gl.getUniformLocation(p, 'uIrradianceMap'),
            uPrefilteredMap: gl.getUniformLocation(p, 'uPrefilteredMap'),
            uBrdfLut: gl.getUniformLocation(p, 'uBrdfLut'),
            uEnvironmentMaxLod: gl.getUniformLocation(p, 'uEnvironmentMaxLod'),
            uCameraPos: gl.getUniformLocation(p, 'uCameraPos'),
        };
        this.locations = {
            position: 0,
            normal: 1,
            texCoord: 2,
            texCoord1: 6,
            color: 3,
            joints: 4,
            weights: 5,
            smoothNormal: 15,
        };
        this.lineUniforms = {
            uProjectionView: gl.getUniformLocation(this.lineProgram, 'uProjectionView'),
            uColor: gl.getUniformLocation(this.lineProgram, 'uColor'),
        };
        this.backgroundUniforms = {
            uInverseProjection: gl.getUniformLocation(this.backgroundProgram, 'uInverseProjection'),
            uInverseView: gl.getUniformLocation(this.backgroundProgram, 'uInverseView'),
            uEnvironment: gl.getUniformLocation(this.backgroundProgram, 'uEnvironment'),
        };
        // WebGL2 requires a VAO even for a shader driven solely by gl_VertexID.
        this.backgroundVao = gl.createVertexArray();
        this.environmentIbl = createEnvironmentIbl(gl, (message, type) => this._log(message, type));
    }

    _setupResize() {
        const resize = () => this._resize();
        this._resizeObserver = new ResizeObserver(resize);
        this._resizeObserver.observe(this.canvas);
        resize();
    }

    _resize() {
        const dpr = Math.min(window.devicePixelRatio || 1, 2);
        const rect = this.canvas.getBoundingClientRect();
        const w = Math.max(1, Math.floor(rect.width * dpr));
        const h = Math.max(1, Math.floor(rect.height * dpr));
        if (this.canvas.width !== w || this.canvas.height !== h) {
            this.canvas.width = w;
            this.canvas.height = h;
        }
        this.gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    }

    _setupControls() {
        const el = this.canvas;
        let lastX = 0, lastY = 0;
        // Drag mode picked once on pointerdown: 'orbit' | 'pan' | 'zoom'.
        let mode = null;
        const pointers = new Map();

        const updateFromPointers = () => {
            if (pointers.size === 1) {
                const [ptr] = pointers.values();
                this._orbitBy((ptr.x - lastX) * ORBIT_RAD_PER_PIXEL, (ptr.y - lastY) * ORBIT_RAD_PER_PIXEL);
                lastX = ptr.x;
                lastY = ptr.y;
            } else if (pointers.size === 2) {
                const pts = [...pointers.values()];
                const dx = pts[0].x - pts[1].x;
                const dy = pts[0].y - pts[1].y;
                const dist = Math.hypot(dx, dy);
                const midX = (pts[0].x + pts[1].x) * 0.5;
                const midY = (pts[0].y + pts[1].y) * 0.5;
                if (this._lastPinch) {
                    this._zoomBy(this._lastPinch.dist / (dist || 1), midX, midY);
                    this._panBy(midX - this._lastPinch.midX, midY - this._lastPinch.midY);
                }
                this._lastPinch = { dist, midX, midY };
            }
        };

        el.addEventListener('pointerdown', (e) => {
            el.setPointerCapture(e.pointerId);
            pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
            lastX = e.clientX;
            lastY = e.clientY;
            if (pointers.size === 1) {
                if (e.button === 1 || e.button === 2 || e.shiftKey) mode = 'pan';
                else if (e.ctrlKey || e.altKey || e.metaKey) mode = 'zoom';
                else mode = 'orbit';
            }
            // Keyboard navigation follows viewport focus.
            el.focus({ preventScroll: true });
            this.setAutoRotate(false);
            this._lastPinch = null;
            e.preventDefault();
        });
        el.addEventListener('pointermove', (e) => {
            if (!pointers.has(e.pointerId)) return;
            pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
            if (pointers.size === 1 && mode !== 'orbit') {
                const dx = e.clientX - lastX;
                const dy = e.clientY - lastY;
                if (mode === 'pan') this._panBy(dx, dy);
                else if (mode === 'zoom') this._zoomBy(Math.exp(dy * 0.005));
                lastX = e.clientX;
                lastY = e.clientY;
            } else {
                updateFromPointers();
                if (pointers.size === 1) {
                    lastX = e.clientX;
                    lastY = e.clientY;
                }
            }
            e.preventDefault();
        });
        const endPointer = (e) => {
            pointers.delete(e.pointerId);
            if (pointers.size < 2) this._lastPinch = null;
            if (pointers.size === 0) mode = null;
            try { el.releasePointerCapture(e.pointerId); } catch (_) { /* ignore */ }
        };
        el.addEventListener('pointerup', endPointer);
        el.addEventListener('pointercancel', endPointer);
        el.addEventListener('pointerleave', endPointer);

        el.addEventListener('contextmenu', (e) => e.preventDefault());
        // Keep the middle button from starting the browser's autoscroll mode.
        el.addEventListener('auxclick', (e) => e.preventDefault());

        el.addEventListener(
            'wheel',
            (e) => {
                e.preventDefault();
                // Firefox reports lines (and pages) rather than pixels; without
                // this the same notch would barely move the camera there.
                let dy = e.deltaY;
                if (e.deltaMode === 1) dy *= 16;
                else if (e.deltaMode === 2) dy *= this.canvas.clientHeight || 400;
                this._zoomBy(Math.exp(dy * 0.001), e.clientX, e.clientY);
            },
            { passive: false },
        );

        el.addEventListener('keydown', (e) => {
            this._navFast = e.shiftKey;
            this._navSlow = e.altKey;
            if (!NAV_KEYS.has(e.code)) return;
            e.preventDefault();
            this.setAutoRotate(false);
            this._navKeys.add(e.code);
        });
        el.addEventListener('keyup', (e) => {
            this._navFast = e.shiftKey;
            this._navSlow = e.altKey;
            this._navKeys.delete(e.code);
        });
        el.addEventListener('blur', () => this._navKeys.clear());
    }

    /**
     * Orbits by radians; positive `dAz`/`dEl` match dragging right/down.
     *
     * The whole rig turns around the scene centre rather than around the look-at
     * point, the way Blender orbits around the selection: once panning or the
     * movement keys have carried the target away, the model still stays put
     * instead of swinging around an empty pivot.
     */
    _orbitBy(dAz, dEl) {
        const right = this._basisRight;
        const up = this._basisUp;
        const forward = this._basisForward;
        const pivot = this._orbitPivot(this._pivotScratch);

        let a = 0, b = 0, c = 0;
        if (pivot) {
            this._cameraBasis(right, up, forward);
            for (let i = 0; i < 3; i++) {
                const d = this.camera.target[i] - pivot[i];
                a += d * right[i];
                b += d * up[i];
                c += d * forward[i];
            }
        }

        this.camera.azimuth -= dAz;
        this.camera.elevation += dEl;
        this.camera.elevation = Math.max(
            -Math.PI * 0.495,
            Math.min(Math.PI * 0.495, this.camera.elevation),
        );

        if (!pivot) return;
        // Rebuilding the same camera-space offset in the turned basis rotates
        // the target around the pivot exactly as far as the eye turned.
        this._cameraBasis(right, up, forward);
        for (let i = 0; i < 3; i++) {
            this.camera.target[i] = pivot[i] + right[i] * a + up[i] * b + forward[i] * c;
        }
    }

    /** World-space centre of the loaded scene, or null when nothing is loaded. */
    _orbitPivot(out) {
        const box = this.scene?.aabb;
        if (!box) return null;
        for (let i = 0; i < 3; i++) out[i] = (box.min[i] + box.max[i]) * 0.5;
        return out;
    }

    /**
     * Dollies the camera by `factor`. With client coordinates, the orbit target
     * also slides toward the cursor so the point under it keeps its screen
     * position — without that the target stays at the model centre and zooming
     * in just buries the camera inside the geometry.
     */
    _zoomBy(factor, clientX, clientY) {
        const before = this.camera.distance;
        this.camera.distance = Math.max(
            this.camera.minDistance,
            Math.min(this.camera.maxDistance, before * factor),
        );
        if (clientX === undefined) return;

        // The clamp may have swallowed part of the requested dolly.
        const applied = this.camera.distance / before;
        const rect = this.canvas.getBoundingClientRect();
        if (!rect.width || !rect.height) return;
        const right = this._basisRight;
        const up = this._basisUp;
        this._cameraBasis(right, up);
        // Offset of the cursor from the view centre, in world units on the
        // plane through the target.
        const k = (2 * before * Math.tan(this.camera.fov * 0.5)) / rect.height;
        const ax = (clientX - (rect.left + rect.width * 0.5)) * k;
        const ay = -(clientY - (rect.top + rect.height * 0.5)) * k;
        const shift = 1 - applied;
        for (let i = 0; i < 3; i++) {
            this.camera.target[i] += (right[i] * ax + up[i] * ay) * shift;
        }
    }

    /**
     * Slides the orbit target inside the camera plane so the point under the
     * cursor stays under the cursor, for any orbit angle and field of view.
     */
    _panBy(dx, dy) {
        const right = this._basisRight;
        const up = this._basisUp;
        this._cameraBasis(right, up);
        const height = this.canvas.clientHeight || this.canvas.height;
        const k = (2 * this.camera.distance * Math.tan(this.camera.fov * 0.5))
            / Math.max(1, height);
        for (let i = 0; i < 3; i++) {
            this.camera.target[i] += (up[i] * dy - right[i] * dx) * k;
        }
    }

    /**
     * Applies the held navigation keys for one frame: WASD and Q/E move the
     * orbit target along the camera axes, arrows orbit.
     */
    _applyKeyboardNavigation(dt) {
        const keys = this._navKeys;
        if (keys.size === 0) return;

        let scale = 1;
        if (this._navFast) scale *= 4;
        if (this._navSlow) scale *= 0.25;

        const orbitStep = ORBIT_RAD_PER_SECOND * dt * scale;
        let dAz = 0, dEl = 0;
        if (keys.has('ArrowLeft')) dAz -= 1;
        if (keys.has('ArrowRight')) dAz += 1;
        if (keys.has('ArrowUp')) dEl += 1;
        if (keys.has('ArrowDown')) dEl -= 1;
        if (dAz || dEl) this._orbitBy(dAz * orbitStep, dEl * orbitStep);

        let fwd = 0, side = 0, lift = 0;
        if (keys.has('KeyW')) fwd += 1;
        if (keys.has('KeyS')) fwd -= 1;
        if (keys.has('KeyD')) side += 1;
        if (keys.has('KeyA')) side -= 1;
        if (keys.has('KeyE')) lift += 1;
        if (keys.has('KeyQ')) lift -= 1;
        if (!fwd && !side && !lift) return;

        const speed = FLY_DISTANCE_PER_SECOND * this.camera.distance * dt * scale;
        const right = this._basisRight;
        const up = this._basisUp;
        const forward = this._basisForward;
        this._cameraBasis(right, up, forward);
        for (let i = 0; i < 3; i++) {
            this.camera.target[i] +=
                (forward[i] * fwd + right[i] * side + up[i] * lift) * speed;
        }
    }

    _log(msg, type = 'info') {
        this.hooks.onLog?.(msg, type);
    }

    setAutoRotate(enabled) {
        const next = Boolean(enabled);
        if (this.autoRotate === next) return;
        this.autoRotate = next;
        this.hooks.onAutoRotateChange?.(next);
    }

    setScene(scene) {
        this._disposeGlResources();
        this._disposeGrid();
        this.scene = scene;
        this.glResources = null;
        if (!scene) {
            this.hooks.onSceneLoaded?.(null);
            return;
        }

        const gl = this.gl;
        const resources = {
            primitives: [],
            textures: [],
            jointMatrices: null,
        };

        // Upload meshes
        for (const mesh of scene.meshes) {
            const primitives = [];
            for (const primitive of mesh.primitives) {
                try {
                    const uploaded = uploadPrimitive(gl, primitive, this.locations);
                    if (uploaded.morph?.dropped > 0) {
                        this._log(
                            `Mesh ${mesh.name}: ${uploaded.morph.dropped} morph targets exceed this GPU's array texture layers and were ignored`,
                            'warning',
                        );
                    }
                    primitives.push({
                        uploaded,
                        materialIndex: primitive.materialIndex,
                    });
                } catch (error) {
                    this._log(`Skipped primitive: ${error.message}`, 'warning');
                }
            }
            resources.primitives.push(primitives);
        }

        // Upload textures
        for (const tex of scene.textures) {
            let glTexture = null;
            if (tex && tex.image) {
                glTexture = this._uploadImage(tex);
            }
            resources.textures.push(glTexture);
        }

        // Every skin needs its own palette. Sharing one array makes the last
        // skin rendered win for all earlier skinned meshes.
        resources.jointMatrices = scene.skins.map((skin) => {
            const count = Math.min(skin.joints.length, MAX_JOINTS);
            if (skin.joints.length > MAX_JOINTS) {
                this._log(
                    `Skin ${skin.name} has ${skin.joints.length} joints; preview uses the first ${MAX_JOINTS}`,
                    'warning',
                );
            }
            return new Float32Array(count * 16);
        });

        this.glResources = resources;
        this.camera.azimuth = DEFAULT_CAMERA_AZIMUTH;
        this.camera.elevation = DEFAULT_CAMERA_ELEVATION;
        this._updateWorldMatrices();
        this._updateSceneBounds();
        this._fitCameraToScene();

        // Reset animation playback
        this.animation.clipIndex = scene.animations.length > 0 ? 0 : -1;
        this.animation.time = 0;
        this.animation.playing = scene.animations.length > 0;
        this.animation.speed = 1;
        this.animation.loop = true;

        this.hooks.onSceneLoaded?.(scene);
    }

    _uploadImage(tex) {
        const gl = this.gl;
        const glTexture = gl.createTexture();
        gl.bindTexture(gl.TEXTURE_2D, glTexture);
        gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, !!tex.flipY);
        // Placeholder color while the bitmap is decoding.
        gl.texImage2D(
            gl.TEXTURE_2D,
            0,
            gl.RGBA,
            1,
            1,
            0,
            gl.RGBA,
            gl.UNSIGNED_BYTE,
            new Uint8Array([255, 255, 255, 255]),
        );
        if (tex.image instanceof ImageBitmap || tex.image instanceof HTMLImageElement) {
            gl.bindTexture(gl.TEXTURE_2D, glTexture);
            gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, tex.image);
            this._setSampler(gl, tex);
        }
        return glTexture;
    }

    _setSampler(gl, tex) {
        const wrapS = tex.wrapS || GL.REPEAT;
        const wrapT = tex.wrapT || GL.REPEAT;
        const minFilter = tex.minFilter || GL.LINEAR_MIPMAP_LINEAR;
        const magFilter = tex.magFilter || GL.LINEAR;
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, wrapS);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, wrapT);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, minFilter);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, magFilter);
        gl.generateMipmap(gl.TEXTURE_2D);
    }

    clear() {
        this._disposeGlResources();
        this._disposeGrid();
        this.scene = null;
        this.glResources = null;
        this.animation.clipIndex = -1;
        this.animation.time = 0;
        this.animation.playing = false;
    }

    _disposeGrid() {
        if (this._grid) {
            this.gl.deleteBuffer(this._grid.buffer);
            this._grid = null;
        }
    }

    _disposeGlResources() {
        const gl = this.gl;
        if (!this.glResources) return;
        for (const primitives of this.glResources.primitives) {
            for (const p of primitives) {
                for (const buf of p.uploaded.buffers) {
                    if (buf) gl.deleteBuffer(buf);
                }
                if (p.uploaded.morph) gl.deleteTexture(p.uploaded.morph.texture);
                if (p.uploaded.vao) gl.deleteVertexArray(p.uploaded.vao);
            }
        }
        for (const tex of this.glResources.textures) {
            if (tex) gl.deleteTexture(tex);
        }
        for (const tex of this.scene?.textures || []) {
            tex.image?.close?.();
        }
        this.glResources = null;
    }

    dispose() {
        this._running = false;
        this._disposeGlResources();
        if (this._emptyMorphTexture) {
            this.gl.deleteTexture(this._emptyMorphTexture);
            this._emptyMorphTexture = null;
        }
        this._resizeObserver?.disconnect();
        if (this.program) this.gl.deleteProgram(this.program);
        if (this.lineProgram) this.gl.deleteProgram(this.lineProgram);
        if (this.backgroundProgram) this.gl.deleteProgram(this.backgroundProgram);
        if (this.backgroundVao) this.gl.deleteVertexArray(this.backgroundVao);
        this.environmentIbl?.dispose();
    }

    resetView() {
        this.camera.azimuth = DEFAULT_CAMERA_AZIMUTH;
        this.camera.elevation = DEFAULT_CAMERA_ELEVATION;
        if (this.scene) {
            this._updateWorldMatrices();
            this._updateSceneBounds();
            this._disposeGrid();
            this._fitCameraToScene();
        }
        else {
            this.camera.target[0] = this.camera.target[1] = this.camera.target[2] = 0;
            this.camera.distance = 3;
        }
    }

    _fitCameraToScene() {
        const box = this.scene?.aabb;
        if (!box) return;
        const cx = (box.min[0] + box.max[0]) * 0.5;
        const cy = (box.min[1] + box.max[1]) * 0.5;
        const cz = (box.min[2] + box.max[2]) * 0.5;
        const dx = box.max[0] - box.min[0];
        const dy = box.max[1] - box.min[1];
        const dz = box.max[2] - box.min[2];
        // A sphere enclosing the full world-space AABB fits from every orbit
        // angle, unlike the previous largest-axis estimate.
        const radius = Math.hypot(dx, dy, dz) * 0.5;
        const safeRadius = radius > 0 ? radius : 1;

        this.camera.target[0] = cx;
        this.camera.target[1] = cy;
        this.camera.target[2] = cz;
        const verticalFov = this.camera.fov;
        const aspect = this.canvas.width / Math.max(1, this.canvas.height);
        const horizontalFov = 2 * Math.atan(Math.tan(verticalFov * 0.5) * aspect);
        const fitFov = Math.min(verticalFov, horizontalFov);
        this.camera.distance = Math.max(0.5, (safeRadius / Math.sin(fitFov * 0.5)) * 1.12);
        const diameter = Math.max(0.001, safeRadius * 2);
        this.camera.near = Math.max(0.001, diameter * 0.001);
        this.camera.far = diameter * 1000 + this.camera.distance * 2;
        // Fixed limits would clamp a large asset below its own fit distance,
        // so one wheel notch would snap the camera inside the model.
        this.camera.minDistance = Math.max(0.001, this.camera.near * 2);
        this.camera.maxDistance = Math.max(this.camera.distance, safeRadius) * 100;
    }

    /**
     * Right and up axes of the camera plane, from the same angles
     * `_cameraPosition` uses: right = normalize(forward x worldUp),
     * up = right x forward. cos(elevation) stays positive under the clamp.
     */
    _cameraBasis(right, up, forward) {
        const ce = Math.cos(this.camera.elevation);
        const se = Math.sin(this.camera.elevation);
        const ca = Math.cos(this.camera.azimuth);
        const sa = Math.sin(this.camera.azimuth);
        right[0] = ca;
        right[1] = 0;
        right[2] = -sa;
        up[0] = -sa * se;
        up[1] = ce;
        up[2] = -ca * se;
        if (!forward) return;
        forward[0] = -ce * sa;
        forward[1] = -se;
        forward[2] = -ce * ca;
    }

    _cameraPosition(out) {
        const ce = Math.cos(this.camera.elevation);
        const se = Math.sin(this.camera.elevation);
        const ca = Math.cos(this.camera.azimuth);
        const sa = Math.sin(this.camera.azimuth);
        const r = this.camera.distance;
        out[0] = this.camera.target[0] + r * ce * sa;
        out[1] = this.camera.target[1] + r * se;
        out[2] = this.camera.target[2] + r * ce * ca;
        return out;
    }

    _loop(now) {
        if (!this._running) return;
        const dt = (now - this._lastTime) / 1000;
        this._lastTime = now;

        // Through `_orbitBy` so auto-rotation circles the model, not the target.
        if (this.autoRotate) this._orbitBy(-dt * 0.4, 0);
        this._applyKeyboardNavigation(dt);

        if (this.animation.playing && this.scene?.animations?.length) {
            this._advanceAnimation(dt);
        }

        this._resize();
        this._render();
        requestAnimationFrame(this._loop);
    }

    _advanceAnimation(dt) {
        const clip = this.scene.animations[this.animation.clipIndex];
        if (!clip) return;
        let time = this.animation.time + dt * this.animation.speed;
        if (time > clip.duration) {
            if (this.animation.loop) {
                time = clip.duration > 0 ? time % clip.duration : 0;
            } else {
                time = clip.duration;
                this.animation.playing = false;
                this.hooks.onAnimationEnded?.();
            }
        }
        this.seekAnimation(time);
    }

    seekAnimation(time) {
        const clip = this.scene?.animations?.[this.animation.clipIndex];
        if (!clip) return false;
        this.animation.time = Math.max(0, Math.min(clip.duration, Number(time) || 0));
        applyAnimation(this.scene, this.animation.clipIndex, this.animation.time);
        return true;
    }

    _updateWorldMatrices() {
        if (!this.scene) return;
        const nodes = this.scene.nodes;
        const roots = this.scene.rootIndices || nodes.map((_, i) => i);
        if (this._visitedNodes) this._visitedNodes.clear();
        else this._visitedNodes = new Set();
        for (const rootIndex of roots) {
            const node = nodes[rootIndex];
            if (node) this._updateNode(node, null);
        }
    }

    /** Recompute the framing bounds after node transforms have been applied. */
    _updateSceneBounds() {
        if (!this.scene) return;
        const aabb = {
            min: [Infinity, Infinity, Infinity],
            max: [-Infinity, -Infinity, -Infinity],
        };
        const point = this._boundsPoint || (this._boundsPoint = vec3.create());
        for (const renderable of this.scene.renderables || []) {
            const meshBox = this.scene.meshes[renderable.meshIndex]?.aabb;
            if (!meshBox) continue;
            const { min, max } = meshBox;
            for (const x of [min[0], max[0]]) {
                for (const y of [min[1], max[1]]) {
                    for (const z of [min[2], max[2]]) {
                        vec3.set(point, x, y, z);
                        vec3.transformMat4(point, point, renderable.node.world);
                        aabb.min[0] = Math.min(aabb.min[0], point[0]);
                        aabb.min[1] = Math.min(aabb.min[1], point[1]);
                        aabb.min[2] = Math.min(aabb.min[2], point[2]);
                        aabb.max[0] = Math.max(aabb.max[0], point[0]);
                        aabb.max[1] = Math.max(aabb.max[1], point[1]);
                        aabb.max[2] = Math.max(aabb.max[2], point[2]);
                    }
                }
            }
        }
        if (isFinite(aabb.min[0])) this.scene.aabb = aabb;
    }

    _updateNode(node, parentWorld) {
        if (!node || !node.trs) return;
        const world = node.world;
        if (node.localMatrix) mat4.copy(world, node.localMatrix);
        else composeMatrix(world, node.trs.translation, node.trs.rotation, node.trs.scale);
        if (parentWorld) {
            mat4.multiply(world, parentWorld, world);
        }
        const nodes = this.scene.nodes;
        const children = node.children || [];
        const visited = this._visitedNodes || (this._visitedNodes = new Set());
        visited.add(node);
        for (const child of children) {
            // glTF stores child references as node indices; mesh-loader uses
            // direct node objects. Support both.
            const childNode = typeof child === 'number' ? nodes[child] : child;
            if (childNode && childNode !== node && !visited.has(childNode)) {
                this._updateNode(childNode, world);
            }
        }
    }

    _render() {
        const gl = this.gl;
        gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
        if (!this.scene || !this.glResources) return;

        // Compute camera matrices
        const aspect = this.canvas.width / Math.max(1, this.canvas.height);
        mat4.perspective(this._projection, this.camera.fov, aspect, this.camera.near, this.camera.far);

        const eye = this._cameraPosition(this._eye || (this._eye = vec3.create()));
        const up = vec3.set(vec3.create(), 0, 1, 0);
        mat4.lookAt(this._view, eye, this.camera.target, up);

        this._updateWorldMatrices();

        this._drawBackground();

        // Grid (drawn first, depth-disabled so it sits behind everything)
        if (this.showGrid) this._drawGrid();

        gl.useProgram(this.program);
        gl.uniformMatrix4fv(this.uniforms.uProjection, false, this._projection);
        gl.uniformMatrix4fv(this.uniforms.uView, false, this._view);
        gl.uniform3fv(this.uniforms.uCameraPos, eye);
        this._bindEnvironmentIbl();

        for (const renderable of this.scene.renderables) {
            const node = renderable.node;
            const primitives = this.glResources.primitives[renderable.meshIndex];
            if (!primitives || primitives.length === 0) continue;

            mat4.copy(this._model, node.world);

            const skin = renderable.skinIndex >= 0 ? this.scene.skins[renderable.skinIndex] : null;
            const jointMatrices = skin
                ? this._computeJointMatrices(skin, node.world, this.glResources.jointMatrices[renderable.skinIndex])
                : null;

            // Normal matrix = inverse-transpose(model)
            mat4.invert(this._normalMatrix, this._model);
            mat4.transpose(this._normalMatrix, this._normalMatrix);
            gl.uniformMatrix4fv(this.uniforms.uModel, false, this._model);
            gl.uniformMatrix4fv(this.uniforms.uNormalMatrix, false, this._normalMatrix);

            for (let i = 0; i < primitives.length; i++) {
                const { uploaded, materialIndex } = primitives[i];
                const usesSkin = !!(jointMatrices && uploaded.hasJoints && uploaded.hasWeights);
                gl.uniform1i(this.uniforms.uUseSkin, usesSkin ? 1 : 0);
                gl.uniform1i(this.uniforms.uJointCount, usesSkin ? jointMatrices.length / 16 : 0);
                if (usesSkin) gl.uniformMatrix4fv(this.uniforms.uJointMatrix, false, jointMatrices);
                gl.bindVertexArray(uploaded.vao);
                const morph = uploaded.morph;
                gl.activeTexture(gl.TEXTURE0 + MORPH_TEXTURE_UNIT);
                gl.bindTexture(gl.TEXTURE_2D_ARRAY, morph ? morph.texture : this._morphPlaceholder());
                gl.uniform1i(this.uniforms.uMorphDeltas, MORPH_TEXTURE_UNIT);
                gl.uniform1i(this.uniforms.uMorphCount, this._selectMorphTargets(morph, node.weights));
                gl.uniform1i(this.uniforms.uMorphStride, morph ? morph.stride : 1);
                gl.uniform1i(this.uniforms.uMorphWidth, morph ? morph.width : 1);
                gl.uniform1fv(this.uniforms.uMorphWeights, this._morphWeights);
                gl.uniform1iv(this.uniforms.uMorphLayers, this._morphLayers);
                const useSmoothNormals = this.smoothNormals
                    && uploaded.hasSmoothNormals && uploaded.morphTargetCount === 0;
                gl.uniform1i(
                    this.uniforms.uUseSmoothNormals,
                    useSmoothNormals ? 1 : 0,
                );
                const material = this.scene.materials[materialIndex];
                this._applyMaterial(material, uploaded, useSmoothNormals);

                if (material?.doubleSided) gl.disable(gl.CULL_FACE);
                else gl.enable(gl.CULL_FACE);

                if (material?.alphaMode === 'BLEND') {
                    gl.enable(gl.BLEND);
                    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
                    gl.depthMask(false);
                } else {
                    gl.disable(gl.BLEND);
                    gl.depthMask(true);
                }

                const mode = this._glMode(uploaded.mode, this.wireframe);
                if (uploaded.indexed) {
                    gl.drawElements(mode, uploaded.elementCount, uploaded.indexType, 0);
                } else {
                    gl.drawArrays(mode, 0, uploaded.elementCount);
                }
            }
        }

        gl.depthMask(true);
        gl.disable(gl.BLEND);
        gl.bindVertexArray(null);
    }

    /**
     * Stage this draw's active morph targets into `_morphLayers`/`_morphWeights`
     * and return how many the shader should blend.
     *
     * The shader loop is bounded, so a mesh with more targets than that keeps
     * its strongest-weighted ones. Real clips stay far below the bound: a glTF
     * weight track blends two neighbouring poses at a time, and even a facial
     * rig drives only a handful of shapes at once.
     */
    _selectMorphTargets(morph, weights) {
        const staged = this._morphWeights
            || (this._morphWeights = new Float32Array(MAX_ACTIVE_MORPH_TARGETS));
        const layers = this._morphLayers
            || (this._morphLayers = new Int32Array(MAX_ACTIVE_MORPH_TARGETS));
        staged.fill(0);
        layers.fill(0);
        if (!morph || !weights) return 0;

        const order = this._morphOrder || (this._morphOrder = []);
        order.length = 0;
        for (let i = 0; i < morph.layerCount; i++) {
            if (weights[i] && morph.filled[i]) order.push(i);
        }
        // Ties keep the lower target index so a held pose stays stable.
        order.sort((a, b) => Math.abs(weights[b]) - Math.abs(weights[a]) || a - b);

        const count = Math.min(order.length, MAX_ACTIVE_MORPH_TARGETS);
        for (let slot = 0; slot < count; slot++) {
            layers[slot] = order[slot];
            staged[slot] = weights[order[slot]];
        }
        return count;
    }

    /**
     * Array texture bound when a primitive has no morph targets. A sampler must
     * reference a complete texture even on the branch that never samples it.
     */
    _morphPlaceholder() {
        if (!this._emptyMorphTexture) {
            const gl = this.gl;
            const texture = gl.createTexture();
            gl.bindTexture(gl.TEXTURE_2D_ARRAY, texture);
            gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
            gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
            gl.texStorage3D(gl.TEXTURE_2D_ARRAY, 1, gl.RGBA32F, 1, 1, 1);
            this._emptyMorphTexture = texture;
        }
        return this._emptyMorphTexture;
    }

    _glMode(mode, wireframe) {
        const gl = this.gl;
        if (wireframe) return gl.LINES;
        switch (mode) {
            case 0: return gl.POINTS;
            case 1: return gl.LINES;
            case 2: return gl.LINE_LOOP;
            case 3: return gl.LINE_STRIP;
            case 5: return gl.TRIANGLE_STRIP;
            case 6: return gl.TRIANGLE_FAN;
            default: return gl.TRIANGLES;
        }
    }

    _applyMaterial(material, uploaded, useSmoothNormals) {
        const gl = this.gl;
        const texCoord = material?.baseColorTexCoord ?? 0;
        const hasTexCoords = texCoord === 0 ? uploaded.hasTexCoords0
            : texCoord === 1 ? uploaded.hasTexCoords1 : false;
        const baseTexture = this.glResources.textures[material?.baseColorTexture];
        const hasTexture = !!baseTexture && hasTexCoords;
        gl.uniform1i(this.uniforms.uHasTexture, hasTexture ? 1 : 0);
        gl.uniform1i(this.uniforms.uHasNormals, uploaded.hasNormals || useSmoothNormals ? 1 : 0);
        gl.uniform1i(this.uniforms.uHasVertexColors, uploaded.hasColors ? 1 : 0);
        gl.uniform1i(this.uniforms.uUnlit, material?.unlit ? 1 : 0);
        gl.uniform1i(this.uniforms.uBaseColorOnly, this.baseColorOnly ? 1 : 0);

        if (hasTexture) {
            gl.activeTexture(gl.TEXTURE0);
            gl.bindTexture(gl.TEXTURE_2D, baseTexture);
            gl.uniform1i(this.uniforms.uBaseColor, 0);
        }
        const factor = material?.baseColorFactor || [1, 1, 1, 1];
        gl.uniform4f(this.uniforms.uBaseColorFactor, factor[0], factor[1], factor[2], factor[3]);
        const transform = material?.baseColorTextureTransform || {};
        const offset = transform.offset || [0, 0];
        const scale = transform.scale || [1, 1];
        gl.uniform1i(this.uniforms.uBaseColorTexCoord, texCoord);
        gl.uniform2f(this.uniforms.uBaseColorTexOffset, offset[0], offset[1]);
        gl.uniform2f(this.uniforms.uBaseColorTexScale, scale[0], scale[1]);
        gl.uniform1f(this.uniforms.uBaseColorTexRotation, transform.rotation || 0);

        const bindTexture = (binding, unit, textureUniform, hasUniform, texCoordUniform) => {
            const texCoord = binding?.texCoord ?? 0;
            const hasUv = texCoord === 0 ? uploaded.hasTexCoords0
                : texCoord === 1 ? uploaded.hasTexCoords1 : false;
            const textureIndex = binding?.index;
            const texture = this.glResources.textures[textureIndex];
            const enabled = !!texture && hasUv;
            gl.uniform1i(hasUniform, enabled ? 1 : 0);
            gl.uniform1i(texCoordUniform, texCoord);
            if (enabled) {
                gl.activeTexture(gl.TEXTURE0 + unit);
                gl.bindTexture(gl.TEXTURE_2D, texture);
                gl.uniform1i(textureUniform, unit);
            }
        };
        bindTexture(
            material?.metallicRoughnessTexture,
            1,
            this.uniforms.uMetallicRoughness,
            this.uniforms.uHasMetallicRoughnessTexture,
            this.uniforms.uMetallicRoughnessTexCoord,
        );
        gl.uniform1f(this.uniforms.uMetallic, material?.metallic ?? 0);
        gl.uniform1f(this.uniforms.uRoughness, material?.roughness ?? 1);
        bindTexture(
            material?.emissiveTexture,
            2,
            this.uniforms.uEmissive,
            this.uniforms.uHasEmissiveTexture,
            this.uniforms.uEmissiveTexCoord,
        );
        const emissive = material?.emissiveFactor || [0, 0, 0];
        gl.uniform3f(this.uniforms.uEmissiveFactor, emissive[0], emissive[1], emissive[2]);
        bindTexture(
            material?.normalTexture,
            3,
            this.uniforms.uNormalTexture,
            this.uniforms.uHasNormalTexture,
            this.uniforms.uNormalTexCoord,
        );
        gl.uniform1f(this.uniforms.uNormalScale, material?.normalTexture?.scale ?? 1);
        bindTexture(
            material?.occlusionTexture,
            4,
            this.uniforms.uOcclusionTexture,
            this.uniforms.uHasOcclusionTexture,
            this.uniforms.uOcclusionTexCoord,
        );
        gl.uniform1f(this.uniforms.uOcclusionStrength, material?.occlusionTexture?.strength ?? 1);
    }

    _computeJointMatrices(skin, meshWorld, jointOut) {
        if (!skin || !jointOut || !mat4.invert(this._scratch, meshWorld)) return null;
        const inverseMeshWorld = this._scratch;
        const tmp = this._jointScratch || (this._jointScratch = mat4.create());
        const count = jointOut.length / 16;
        for (let i = 0; i < count; i++) {
            const joint = skin.joints[i];
            if (!joint?.node) return null;
            // In glTF, the palette is inverse(mesh world) * joint world * IBM.
            mat4.multiply(tmp, inverseMeshWorld, joint.node.world);
            mat4.multiply(jointOut.subarray(i * 16, (i + 1) * 16), tmp, joint.inverseBind);
        }
        return jointOut;
    }

    _drawGrid() {
        const gl = this.gl;
        mat4.multiply(this._projectionView, this._projection, this._view);
        if (!this._grid) this._buildSceneGrid();
        if (!this._grid) return;
        gl.useProgram(this.lineProgram);
        gl.uniformMatrix4fv(this.lineUniforms.uProjectionView, false, this._projectionView);
        gl.uniform3f(this.lineUniforms.uColor, 0.31, 0.40, 0.56);
        gl.bindBuffer(gl.ARRAY_BUFFER, this._grid.buffer);
        gl.enableVertexAttribArray(0);
        gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
        gl.drawArrays(gl.LINES, 0, this._grid.count);
    }

    /** Render the same radiance cubemap used by material IBL. */
    _drawBackground() {
        const gl = this.gl;
        if (!mat4.invert(this._inverseProjection, this._projection)
            || !mat4.invert(this._inverseView, this._view)) return;
        gl.disable(gl.DEPTH_TEST);
        gl.depthMask(false);
        gl.useProgram(this.backgroundProgram);
        gl.uniformMatrix4fv(this.backgroundUniforms.uInverseProjection, false, this._inverseProjection);
        gl.uniformMatrix4fv(this.backgroundUniforms.uInverseView, false, this._inverseView);
        gl.activeTexture(gl.TEXTURE5);
        gl.bindTexture(gl.TEXTURE_CUBE_MAP, this.environmentIbl.environment);
        gl.uniform1i(this.backgroundUniforms.uEnvironment, 5);
        gl.bindVertexArray(this.backgroundVao);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        gl.bindVertexArray(null);
        gl.depthMask(true);
        gl.enable(gl.DEPTH_TEST);
    }

    _bindEnvironmentIbl() {
        const gl = this.gl;
        gl.activeTexture(gl.TEXTURE6);
        gl.bindTexture(gl.TEXTURE_CUBE_MAP, this.environmentIbl.irradiance);
        gl.uniform1i(this.uniforms.uIrradianceMap, 6);
        gl.activeTexture(gl.TEXTURE7);
        gl.bindTexture(gl.TEXTURE_CUBE_MAP, this.environmentIbl.prefiltered);
        gl.uniform1i(this.uniforms.uPrefilteredMap, 7);
        gl.activeTexture(gl.TEXTURE8);
        gl.bindTexture(gl.TEXTURE_2D, this.environmentIbl.brdfLut);
        gl.uniform1i(this.uniforms.uBrdfLut, 8);
        gl.uniform1f(this.uniforms.uEnvironmentMaxLod, this.environmentIbl.maxLod);
    }

    /** Build a grid scaled to the loaded model's AABB. */
    _buildSceneGrid() {
        const box = this.scene?.aabb;
        if (!box) return;
        const dx = box.max[0] - box.min[0];
        const dz = box.max[2] - box.min[2];
        const span = Math.max(dx, dz);
        if (!isFinite(span) || span <= 0) return;

        // Round the cell size up to a "nice" power of ten so the grid reads well.
        const targetCells = 20;
        const rawStep = span / targetCells;
        const magnitude = Math.pow(10, Math.floor(Math.log10(rawStep)));
        const step = [1, 2, 5, 10]
            .map((m) => m * magnitude)
            .find((s) => s >= rawStep) || rawStep;

        const cx = (box.min[0] + box.max[0]) * 0.5;
        const cz = (box.min[2] + box.max[2]) * 0.5;
        const halfExtent = Math.ceil(span / step / 2) + 2;
        const half = halfExtent * step;

        const positions = [];
        const minI = Math.round((cx - half) / step);
        const maxI = Math.round((cx + half) / step);
        const minJ = Math.round((cz - half) / step);
        const maxJ = Math.round((cz + half) / step);
        const gridY = box.min[1] - Math.max(step * 0.01, 0.0001);
        for (let i = minI; i <= maxI; i++) {
            const x = i * step;
            positions.push(x, gridY, minJ * step, x, gridY, maxJ * step);
        }
        for (let j = minJ; j <= maxJ; j++) {
            const z = j * step;
            positions.push(minI * step, gridY, z, maxI * step, gridY, z);
        }

        const buffer = this.gl.createBuffer();
        this.gl.bindBuffer(this.gl.ARRAY_BUFFER, buffer);
        this.gl.bufferData(this.gl.ARRAY_BUFFER, new Float32Array(positions), this.gl.STATIC_DRAW);
        this._grid = { buffer, count: positions.length / 3 };
    }
}

/**
 * Sample an animation channel at time t and apply it to the node TRS.
 * Mutates scene node TRS in place.
 */
function applyAnimation(scene, clipIndex, t) {
    const clip = scene.animations[clipIndex];
    if (!clip) return;
    // Reset each animated node before applying this frame. This is important
    // when switching clips, and for permissive FBX where a static rest matrix
    // is converted to TRS only once its Lcl animation is evaluated.
    const resetNodes = new Set();
    for (const channel of clip.channels) {
        const node = channel.node;
        const animationRest = node?.animationTrs || node?.restTrs;
        if (!node || resetNodes.has(node) || !animationRest) continue;
        node.trs.translation = [...animationRest.translation];
        node.trs.rotation = [...animationRest.rotation];
        node.trs.scale = [...animationRest.scale];
        resetNodes.add(node);
    }
    for (const channel of clip.channels) {
        const sampler = channel.sampler;
        const input = sampler.input;
        const output = sampler.output;
        if (!input || !output || input.length === 0) continue;

        let i0 = 0;
        for (let i = 0; i < input.length - 1; i++) {
            if (input[i] <= t && input[i + 1] >= t) { i0 = i; break; }
            if (t >= input[input.length - 1]) i0 = input.length - 1;
        }
        const t0 = input[i0];
        const t1 = input[Math.min(i0 + 1, input.length - 1)];
        let frac = (t1 > t0) ? (t - t0) / (t1 - t0) : 0;
        frac = Math.max(0, Math.min(1, frac));

        const interpolation = sampler.interpolation || 'LINEAR';
        const path = channel.path;
        applyChannel(
            channel.node,
            path,
            channel.targetCount,
            interpolation,
            output,
            i0,
            frac,
            t1 - t0,
        );
    }
}

function applyChannel(node, path, targetCount, interpolation, output, i0, frac, duration) {
    const out = path === 'weights' ? node.weights : node.trs[path];
    // Keep this guard at the render boundary so an unsupported future channel
    // cannot break the animation loop and leave the preview canvas stale.
    if (!out) return;

    if (path !== 'weights') {
        // A node animated through TRS must no longer use a static matrix. Such
        // an asset is invalid in strict glTF, but this gives the preview a
        // sensible result for permissively-authored files.
        node.localMatrix = null;
    }
    const components = path === 'weights' ? targetCount : path === 'rotation' ? 4 : 3;
    if (components <= 0) return;
    const stride = interpolation === 'CUBICSPLINE' ? components * 3 : components;
    const base0 = i0 * stride;
    const base1 = Math.min(i0 + 1, output.length / stride - 1) * stride;
    if (interpolation === 'STEP') {
        for (let k = 0; k < components; k++) out[k] = output[base0 + k];
        return;
    }
    if (interpolation === 'CUBICSPLINE') {
        // glTF cubic spline: out = (2t^3 - 3t^2 + 1) p0 + (t^3 - 2t^2 + t) m0 + (-2t^3 + 3t^2) p1 + (t^3 - t^2) m1
        // Layout per keyframe: [inTangent, value, outTangent]
        const p0 = base0 + components;
        const m0 = base0 + 2 * components;
        const p1 = base1 + components;
        const m1 = base1;
        for (let k = 0; k < components; k++) {
            out[k] = cubicSplineInterpolate(
                output[p0 + k],
                output[m0 + k],
                output[p1 + k],
                output[m1 + k],
                frac,
                duration,
            );
        }
        if (path === 'rotation') {
            const len = Math.hypot(out[0], out[1], out[2], out[3]) || 1;
            for (let k = 0; k < 4; k++) out[k] /= len;
        }
        return;
    }
    // LINEAR
    if (path === 'rotation') {
        const a = [output[base0], output[base0 + 1], output[base0 + 2], output[base0 + 3]];
        const b = [output[base1], output[base1 + 1], output[base1 + 2], output[base1 + 3]];
        quat.slerp(out, a, b, frac);
    } else {
        for (let k = 0; k < components; k++) {
            out[k] = output[base0 + k] + (output[base1 + k] - output[base0 + k]) * frac;
        }
    }
}

/** Evaluate one component of a glTF CUBICSPLINE animation segment. */
export function cubicSplineInterpolate(p0, outTangent0, p1, inTangent1, t, duration) {
    const t2 = t * t;
    const t3 = t2 * t;
    const c0 = 2 * t3 - 3 * t2 + 1;
    const c1 = t3 - 2 * t2 + t;
    const c2 = -2 * t3 + 3 * t2;
    const c3 = t3 - t2;
    return c0 * p0
        + c1 * duration * outTangent0
        + c2 * p1
        + c3 * duration * inTangent1;
}
