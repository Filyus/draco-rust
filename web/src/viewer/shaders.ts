/**
 * These two bound uniform array lengths declared below, so they belong with the
 * GLSL that spells them out. The morph bound is a shader loop length rather
 * than a hardware limit: a mesh may declare any number of targets, and the
 * renderer blends the strongest-weighted ones.
 */
export const MAX_JOINTS = 256;
export const MAX_ACTIVE_MORPH_TARGETS = 32;

/** GLSL sources for the preview: PBR surface, debug lines and the backdrop. */

export const VERT_SRC = `#version 300 es
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

export const FRAG_SRC = `#version 300 es
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

export const LINE_VERT_SRC = `#version 300 es
precision highp float;
layout(location=0) in vec3 aPosition;
uniform mat4 uProjectionView;
void main() {
    gl_Position = uProjectionView * vec4(aPosition, 1.0);
}
`;

export const LINE_FRAG_SRC = `#version 300 es
precision highp float;
uniform vec3 uColor;
out vec4 outColor;
void main() {
    outColor = vec4(uColor, 1.0);
}
`;

export const BACKGROUND_VERT_SRC = `#version 300 es
precision highp float;
out vec2 vNdc;
void main() {
    vec2 positions[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    vNdc = positions[gl_VertexID];
    gl_Position = vec4(vNdc, 0.0, 1.0);
}
`;

export const BACKGROUND_FRAG_SRC = `#version 300 es
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
