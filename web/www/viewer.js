/**
 * Vanilla WebGL2 3D preview viewer.
 *
 * Renders the format-agnostic Scene produced by gltf-loader.js / mesh-loader.js.
 * Supports TRS + skinned animation, base color materials (texture + factor +
 * vertex colors), and orbit/touch camera controls. No external dependencies.
 */

import { mat4, vec3, quat, composeMatrix } from './math.js';

const MAX_JOINTS = 256;

const VERT_SRC = `#version 300 es
precision highp float;

layout(location=0) in vec3 aPosition;
layout(location=1) in vec3 aNormal;
layout(location=2) in vec2 aTexCoord;
layout(location=6) in vec2 aTexCoord1;
layout(location=3) in vec4 aColor;
layout(location=4) in vec4 aJoints;
layout(location=5) in vec4 aWeights;
layout(location=7) in vec3 aMorphPosition0;
layout(location=8) in vec3 aMorphPosition1;
layout(location=9) in vec3 aMorphPosition2;
layout(location=10) in vec3 aMorphPosition3;
layout(location=11) in vec3 aMorphNormal0;
layout(location=12) in vec3 aMorphNormal1;
layout(location=13) in vec3 aMorphNormal2;
layout(location=14) in vec3 aMorphNormal3;

uniform mat4 uProjection;
uniform mat4 uView;
uniform mat4 uModel;
uniform mat4 uNormalMatrix;
uniform int uUseSkin;
uniform int uJointCount;
uniform mat4 uJointMatrix[${MAX_JOINTS}];
uniform int uMorphTargetCount;
uniform float uMorphWeights[4];

out vec3 vNormal;
out vec2 vTexCoord;
out vec2 vTexCoord1;
out vec4 vColor;
out vec3 vWorldPos;

void main() {
    vec3 morphedPosition = aPosition;
    if (uMorphTargetCount > 0) morphedPosition += aMorphPosition0 * uMorphWeights[0];
    if (uMorphTargetCount > 1) morphedPosition += aMorphPosition1 * uMorphWeights[1];
    if (uMorphTargetCount > 2) morphedPosition += aMorphPosition2 * uMorphWeights[2];
    if (uMorphTargetCount > 3) morphedPosition += aMorphPosition3 * uMorphWeights[3];
    vec3 morphedNormal = aNormal;
    if (uMorphTargetCount > 0) morphedNormal += aMorphNormal0 * uMorphWeights[0];
    if (uMorphTargetCount > 1) morphedNormal += aMorphNormal1 * uMorphWeights[1];
    if (uMorphTargetCount > 2) morphedNormal += aMorphNormal2 * uMorphWeights[2];
    if (uMorphTargetCount > 3) morphedNormal += aMorphNormal3 * uMorphWeights[3];
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

uniform vec3 uLightDir;        // direction TO light (key)
uniform vec3 uLightColor;
uniform vec3 uFillDir;         // direction TO light (fill, softer)
uniform vec3 uFillColor;
uniform vec3 uCameraPos;

out vec4 outColor;

const float PI = 3.14159265359;

vec2 selectUv(int texCoord) {
    return texCoord == 1 ? vTexCoord1 : vTexCoord;
}

float distributionGgx(vec3 N, vec3 H, float roughness) {
    float a = roughness * roughness;
    float a2 = a * a;
    float nDotH = max(dot(N, H), 0.0);
    float nDotH2 = nDotH * nDotH;
    float denominator = nDotH2 * (a2 - 1.0) + 1.0;
    return a2 / max(PI * denominator * denominator, 0.0001);
}

float geometrySchlickGgx(float nDotV, float roughness) {
    float r = roughness + 1.0;
    float k = (r * r) / 8.0;
    return nDotV / max(nDotV * (1.0 - k) + k, 0.0001);
}

float geometrySmith(vec3 N, vec3 V, vec3 L, float roughness) {
    return geometrySchlickGgx(max(dot(N, V), 0.0), roughness)
        * geometrySchlickGgx(max(dot(N, L), 0.0), roughness);
}

vec3 fresnelSchlick(float cosTheta, vec3 f0) {
    return f0 + (1.0 - f0) * pow(1.0 - cosTheta, 5.0);
}

vec3 fresnelSchlickRoughness(float cosTheta, vec3 f0, float roughness) {
    return f0 + (max(vec3(1.0 - roughness), f0) - f0) * pow(1.0 - cosTheta, 5.0);
}

// A small analytic studio environment with a neutral matte floor and broad
// softboxes. It keeps reflections readable without an HDR decoder or a
// prefiltered cube map, while avoiding a colored floor cast on the asset.
vec3 studioRadiance(vec3 direction, float roughness) {
    float up = clamp(direction.y * 0.5 + 0.5, 0.0, 1.0);
    vec3 sharp = mix(vec3(0.045, 0.055, 0.075), vec3(0.30, 0.39, 0.57), up);
    sharp = mix(sharp, vec3(0.22, 0.27, 0.37), exp(-abs(direction.y) * 13.0) * 0.14);
    vec3 keyDirection = normalize(vec3(-0.45, 0.78, 0.42));
    vec3 rimDirection = normalize(vec3(0.52, 0.42, -0.72));
    float key = pow(max(dot(direction, keyDirection), 0.0), 72.0);
    float rim = pow(max(dot(direction, rimDirection), 0.0), 36.0);
    sharp += vec3(2.7, 2.75, 2.85) * key + vec3(0.62, 0.78, 1.05) * rim;
    vec3 blurred = vec3(0.18, 0.22, 0.29);
    return mix(sharp, blurred, roughness * roughness);
}

vec3 studioIrradiance(vec3 normal) {
    float up = clamp(normal.y * 0.5 + 0.5, 0.0, 1.0);
    return mix(vec3(0.10, 0.115, 0.14), vec3(0.40, 0.47, 0.60), up);
}

vec3 directPbrLight(vec3 N, vec3 V, vec3 L, vec3 radiance, vec3 baseColor, float metallic, float roughness) {
    vec3 H = normalize(V + L);
    float nDotL = max(dot(N, L), 0.0);
    float nDotV = max(dot(N, V), 0.0);
    if (nDotL == 0.0 || nDotV == 0.0) return vec3(0.0);
    vec3 f0 = mix(vec3(0.04), baseColor, metallic);
    vec3 F = fresnelSchlick(max(dot(H, V), 0.0), f0);
    float D = distributionGgx(N, H, roughness);
    float G = geometrySmith(N, V, L, roughness);
    vec3 specular = (D * G * F) / max(4.0 * nDotV * nDotL, 0.0001);
    vec3 diffuse = (1.0 - F) * (1.0 - metallic) * baseColor / PI;
    return (diffuse + specular) * radiance * nDotL;
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

    vec3 L = normalize(uLightDir);
    vec3 F = normalize(uFillDir);
    vec3 color = directPbrLight(N, V, L, uLightColor, baseColor, metallic, roughness);
    color += directPbrLight(N, V, F, uFillColor * 0.5, baseColor, metallic, roughness);
    float occlusion = 1.0;
    if (uHasOcclusionTexture == 1) {
        occlusion = mix(1.0, texture(uOcclusionTexture, selectUv(uOcclusionTexCoord)).r, uOcclusionStrength);
    }
    vec3 f0 = mix(vec3(0.04), baseColor, metallic);
    vec3 iblFresnel = fresnelSchlickRoughness(max(dot(N, V), 0.0), f0, roughness);
    vec3 diffuseIbl = (1.0 - iblFresnel) * (1.0 - metallic) * baseColor * studioIrradiance(N);
    vec3 reflected = reflect(-V, N);
    vec3 specularIbl = iblFresnel * studioRadiance(reflected, roughness);
    color += (diffuseIbl * 1.05 + specularIbl) * occlusion;
    vec3 emissive = uEmissiveFactor;
    if (uHasEmissiveTexture == 1) {
        emissive *= pow(texture(uEmissive, selectUv(uEmissiveTexCoord)).rgb, vec3(2.2));
    }
    color += emissive;
    color *= 1.15;
    color = color / (color + vec3(1.0));
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
out vec4 outColor;

vec3 studioRadiance(vec3 direction) {
    // A neutral floor / sky split gives the world-space horizon a clear edge
    // without tinting the lower part of the model.
    float sky = smoothstep(-0.035, 0.055, direction.y);
    vec3 floor = vec3(0.025, 0.032, 0.047);
    vec3 skyColor = mix(vec3(0.105, 0.145, 0.225), vec3(0.40, 0.51, 0.70), max(direction.y, 0.0));
    vec3 color = mix(floor, skyColor, sky);
    color = mix(color, vec3(0.23, 0.29, 0.39), exp(-abs(direction.y) * 20.0) * 0.12);
    vec3 keyDirection = normalize(vec3(-0.45, 0.78, 0.42));
    vec3 rimDirection = normalize(vec3(0.52, 0.42, -0.72));
    color += vec3(3.75, 3.85, 4.0) * pow(max(dot(direction, keyDirection), 0.0), 56.0);
    color += vec3(0.95, 1.18, 1.65) * pow(max(dot(direction, rimDirection), 0.0), 30.0);
    return color;
}

void main() {
    vec4 view = uInverseProjection * vec4(vNdc, 1.0, 1.0);
    view /= view.w;
    vec3 direction = normalize((uInverseView * vec4(view.xyz, 0.0)).xyz);
    vec3 color = studioRadiance(direction);
    color = color / (color + vec3(1.0));
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
        const buf = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, buf);
        gl.bufferData(gl.ARRAY_BUFFER, byteView(attr.bytes), gl.STATIC_DRAW);

        const normalized = attr.normalized || semantic.startsWith('COLOR_') || semantic.startsWith('WEIGHTS_');
        const components = attr.components;
        if (desiredComponents && components !== desiredComponents) {
            gl.disableVertexAttribArray(location);
            gl.bindBuffer(gl.ARRAY_BUFFER, null);
            gl.deleteBuffer(buf);
            return false;
        }
        gl.enableVertexAttribArray(location);
        gl.vertexAttribPointer(location, components, attr.componentType, normalized, 0, 0);
        buffers.push(buf);
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
        morphPositions: locationMap.morphPositions,
        morphNormals: locationMap.morphNormals,
    };

    const info = {
        vao,
        buffers,
        hasNormals: !!bindAttribute('NORMAL', layout.normal),
        hasTexCoords0: !!bindAttribute('TEXCOORD_0', layout.texCoord),
        hasTexCoords1: !!bindAttribute('TEXCOORD_1', layout.texCoord1),
        hasColors: !!bindAttribute('COLOR_0', layout.color),
        hasJoints: !!bindAttribute('JOINTS_0', layout.joints),
        hasWeights: !!bindAttribute('WEIGHTS_0', layout.weights),
        morphTargetCount: Math.min(4, (primitive.morphPositions || []).length),
        mode: primitive.mode,
        elementCount: 0,
        indexed: false,
    };

    bindAttribute('POSITION', layout.position);
    for (let i = 0; i < info.morphTargetCount; i++) {
        bindAccessor(primitive.morphPositions[i], `MORPH_POSITION_${i}`, layout.morphPositions[i], 3);
        bindAccessor((primitive.morphNormals || [])[i], `MORPH_NORMAL_${i}`, layout.morphNormals[i], 3);
    }

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
        this.hooks = hooks; // { onSceneLoaded(scene), onError(msg), onLog(msg, type) }
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
            azimuth: Math.PI * 0.25,
            elevation: Math.PI * 0.2,
            fov: Math.PI / 4,
            near: 0.05,
            far: 1000,
        };

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
        // Keep the canvas transparent so the visible studio backdrop in CSS
        // matches the analytic environment used by the PBR shader.
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
            uMorphTargetCount: gl.getUniformLocation(p, 'uMorphTargetCount'),
            uMorphWeights: gl.getUniformLocation(p, 'uMorphWeights[0]'),
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
            uLightDir: gl.getUniformLocation(p, 'uLightDir'),
            uLightColor: gl.getUniformLocation(p, 'uLightColor'),
            uFillDir: gl.getUniformLocation(p, 'uFillDir'),
            uFillColor: gl.getUniformLocation(p, 'uFillColor'),
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
            morphPositions: [7, 8, 9, 10],
            morphNormals: [11, 12, 13, 14],
        };
        this.lineUniforms = {
            uProjectionView: gl.getUniformLocation(this.lineProgram, 'uProjectionView'),
            uColor: gl.getUniformLocation(this.lineProgram, 'uColor'),
        };
        this.backgroundUniforms = {
            uInverseProjection: gl.getUniformLocation(this.backgroundProgram, 'uInverseProjection'),
            uInverseView: gl.getUniformLocation(this.backgroundProgram, 'uInverseView'),
        };
        // WebGL2 requires a VAO even for a shader driven solely by gl_VertexID.
        this.backgroundVao = gl.createVertexArray();
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
        let panning = false;
        const pointers = new Map();

        const updateFromPointers = () => {
            if (pointers.size === 1) {
                const [ptr] = pointers.values();
                const dx = ptr.x - lastX;
                const dy = ptr.y - lastY;
                this.camera.azimuth -= dx * 0.01;
                this.camera.elevation -= dy * 0.01;
                this.camera.elevation = Math.max(
                    -Math.PI * 0.495,
                    Math.min(Math.PI * 0.495, this.camera.elevation),
                );
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
                    this.camera.distance *= this._lastPinch.dist / (dist || 1);
                    this.camera.distance = Math.max(0.05, Math.min(1000, this.camera.distance));
                    this.camera.target[0] -= (midX - this._lastPinch.midX) * 0.002 * this.camera.distance;
                    this.camera.target[1] += (midY - this._lastPinch.midY) * 0.002 * this.camera.distance;
                }
                this._lastPinch = { dist, midX, midY };
            }
        };

        el.addEventListener('pointerdown', (e) => {
            el.setPointerCapture(e.pointerId);
            pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
            lastX = e.clientX;
            lastY = e.clientY;
            if (e.button === 2 || pointers.size === 2) {
                panning = true;
            } else {
            }
            this.autoRotate = false;
            this._lastPinch = null;
            e.preventDefault();
        });
        el.addEventListener('pointermove', (e) => {
            if (!pointers.has(e.pointerId)) return;
            pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
            if (panning && pointers.size === 1) {
                const dx = e.clientX - lastX;
                const dy = e.clientY - lastY;
                this.camera.target[0] -= dx * 0.002 * this.camera.distance;
                this.camera.target[1] += dy * 0.002 * this.camera.distance;
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
            if (pointers.size === 0) panning = false;
            try { el.releasePointerCapture(e.pointerId); } catch (_) { /* ignore */ }
        };
        el.addEventListener('pointerup', endPointer);
        el.addEventListener('pointercancel', endPointer);
        el.addEventListener('pointerleave', endPointer);

        el.addEventListener('contextmenu', (e) => e.preventDefault());

        el.addEventListener(
            'wheel',
            (e) => {
                e.preventDefault();
                const factor = Math.exp(e.deltaY * 0.001);
                this.camera.distance *= factor;
                this.camera.distance = Math.max(0.05, Math.min(1000, this.camera.distance));
            },
            { passive: false },
        );
    }

    _log(msg, type = 'info') {
        this.hooks.onLog?.(msg, type);
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
        this._resizeObserver?.disconnect();
        if (this.program) this.gl.deleteProgram(this.program);
        if (this.lineProgram) this.gl.deleteProgram(this.lineProgram);
        if (this.backgroundProgram) this.gl.deleteProgram(this.backgroundProgram);
        if (this.backgroundVao) this.gl.deleteVertexArray(this.backgroundVao);
    }

    resetView() {
        if (this.scene) {
            this._updateWorldMatrices();
            this._updateSceneBounds();
            this._disposeGrid();
            this._fitCameraToScene();
        }
        else {
            this.camera.target[0] = this.camera.target[1] = this.camera.target[2] = 0;
            this.camera.distance = 3;
            this.camera.azimuth = Math.PI * 0.25;
            this.camera.elevation = Math.PI * 0.2;
        }
        this.autoRotate = false;
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

        if (this.autoRotate) this.camera.azimuth += dt * 0.4;

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
        this.animation.time += dt * this.animation.speed;
        if (this.animation.time > clip.duration) {
            if (this.animation.loop) {
                this.animation.time = this.animation.time % clip.duration;
            } else {
                this.animation.time = clip.duration;
                this.animation.playing = false;
                this.hooks.onAnimationEnded?.();
            }
        }
        applyAnimation(this.scene, this.animation.clipIndex, this.animation.time);
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
        gl.uniform3fv(this.uniforms.uLightDir, this._lightDir || (this._lightDir = new Float32Array([0.5, 0.8, 0.6])));
        gl.uniform3fv(this.uniforms.uLightColor, this._lightColor || (this._lightColor = new Float32Array([1.0, 0.97, 0.9])));
        gl.uniform3fv(this.uniforms.uFillDir, this._fillDir || (this._fillDir = new Float32Array([-0.6, 0.3, -0.4])));
        gl.uniform3fv(this.uniforms.uFillColor, this._fillColor || (this._fillColor = new Float32Array([0.4, 0.45, 0.6])));
        gl.uniform3fv(this.uniforms.uCameraPos, eye);

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
                const morphWeights = this._morphWeights || (this._morphWeights = new Float32Array(4));
                morphWeights.fill(0);
                if (node.weights) {
                    morphWeights.set(node.weights.subarray ? node.weights.subarray(0, 4) : node.weights.slice(0, 4));
                }
                gl.uniform1i(this.uniforms.uMorphTargetCount, uploaded.morphTargetCount);
                gl.uniform1fv(this.uniforms.uMorphWeights, morphWeights);
                const material = this.scene.materials[materialIndex];
                this._applyMaterial(material, uploaded);
                gl.bindVertexArray(uploaded.vao);

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

    _applyMaterial(material, uploaded) {
        const gl = this.gl;
        const texCoord = material?.baseColorTexCoord ?? 0;
        const hasTexCoords = texCoord === 0 ? uploaded.hasTexCoords0
            : texCoord === 1 ? uploaded.hasTexCoords1 : false;
        const baseTexture = this.glResources.textures[material?.baseColorTexture];
        const hasTexture = !!baseTexture && hasTexCoords;
        gl.uniform1i(this.uniforms.uHasTexture, hasTexture ? 1 : 0);
        gl.uniform1i(this.uniforms.uHasNormals, uploaded.hasNormals ? 1 : 0);
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
        gl.uniform3f(this.lineUniforms.uColor, 0.14, 0.19, 0.29);
        gl.bindBuffer(gl.ARRAY_BUFFER, this._grid.buffer);
        gl.enableVertexAttribArray(0);
        gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
        gl.drawArrays(gl.LINES, 0, this._grid.count);
    }

    /** Render the analytic studio environment in camera space before geometry. */
    _drawBackground() {
        const gl = this.gl;
        if (!mat4.invert(this._inverseProjection, this._projection)
            || !mat4.invert(this._inverseView, this._view)) return;
        gl.disable(gl.DEPTH_TEST);
        gl.depthMask(false);
        gl.useProgram(this.backgroundProgram);
        gl.uniformMatrix4fv(this.backgroundUniforms.uInverseProjection, false, this._inverseProjection);
        gl.uniformMatrix4fv(this.backgroundUniforms.uInverseView, false, this._inverseView);
        gl.bindVertexArray(this.backgroundVao);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        gl.bindVertexArray(null);
        gl.depthMask(true);
        gl.enable(gl.DEPTH_TEST);
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
    // Reset nodes to rest pose for channels we are animating.
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
        applyChannel(channel.node, path, channel.targetCount, interpolation, output, i0, frac);
    }
}

function applyChannel(node, path, targetCount, interpolation, output, i0, frac) {
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
        const t = frac;
        const t2 = t * t;
        const t3 = t2 * t;
        const c0 = 2 * t3 - 3 * t2 + 1;
        const c1 = t3 - 2 * t2 + t;
        const c2 = -2 * t3 + 3 * t2;
        const c3 = t3 - t2;
        for (let k = 0; k < components; k++) {
            out[k] = c0 * output[p0 + k] + c1 * output[m0 + k] + c2 * output[p1 + k] + c3 * output[m1 + k];
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
