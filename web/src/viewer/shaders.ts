/**
 * These two bound uniform array lengths declared below, so they belong with the
 * GLSL that spells them out. The morph bound is a shader loop length rather
 * than a hardware limit: a mesh may declare any number of targets, and the
 * renderer blends the strongest-weighted ones.
 */
export const MAX_JOINTS = 256;
export const MAX_ACTIVE_MORPH_TARGETS = 32;

/**
 * The material texture slots, in the order the shader indexes them.
 *
 * Every slot carries its own TEXCOORD set and its own `KHR_texture_transform`,
 * so the fragment shader addresses them through two uniform arrays rather than
 * a pair of scalars per slot. This list is the single definition of that index
 * space: the GLSL `SLOT_*` constants below are generated from it, and the
 * renderer builds its binding table by mapping over it. Reordering it moves
 * both sides at once, which is the point.
 */
export const TEXTURE_SLOTS = [
  'BASE_COLOR',
  'METALLIC_ROUGHNESS',
  'EMISSIVE',
  'NORMAL',
  'OCCLUSION',
  'SPECULAR',
  'SPECULAR_COLOR',
  'CLEARCOAT',
  'CLEARCOAT_ROUGHNESS',
  'CLEARCOAT_NORMAL',
] as const;

export type TextureSlotName = (typeof TEXTURE_SLOTS)[number];

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
uniform int uHasMetallicRoughnessTexture;
uniform sampler2D uMetallicRoughness;
uniform float uMetallic;
uniform float uRoughness;
uniform int uHasEmissiveTexture;
uniform sampler2D uEmissive;
uniform vec3 uEmissiveFactor;
uniform int uHasNormalTexture;
uniform sampler2D uNormalTexture;
uniform float uNormalScale;
uniform int uHasOcclusionTexture;
uniform sampler2D uOcclusionTexture;
uniform float uOcclusionStrength;
// KHR_materials_ior / KHR_materials_specular: the dielectric reflectance the
// index of refraction implies, tinted and weighted by the specular extension.
uniform float uIor;
uniform float uSpecularFactor;
uniform vec3 uSpecularColorFactor;
uniform int uHasSpecularTexture;
uniform sampler2D uSpecularTexture;
uniform int uHasSpecularColorTexture;
uniform sampler2D uSpecularColorTexture;
// KHR_materials_clearcoat: a second specular lobe over the whole material.
uniform float uClearcoatFactor;
uniform float uClearcoatRoughnessFactor;
uniform int uHasClearcoatTexture;
uniform sampler2D uClearcoatTexture;
uniform int uHasClearcoatRoughnessTexture;
uniform sampler2D uClearcoatRoughnessTexture;
uniform int uHasClearcoatNormalTexture;
uniform sampler2D uClearcoatNormalTexture;
uniform float uClearcoatNormalScale;
uniform samplerCube uIrradianceMap;
uniform samplerCube uPrefilteredMap;
uniform sampler2D uBrdfLut;
uniform float uEnvironmentMaxLod;
uniform vec3 uCameraPos;

// Per-slot UV state: which TEXCOORD set the slot reads, and the
// KHR_texture_transform matrix to run it through. The matrix is built on the
// CPU, so the shader neither knows nor cares whether the extension was present:
// an absent transform arrives as the identity.
${TEXTURE_SLOTS.map((name, slot) => `const int SLOT_${name} = ${slot};`).join('\n')}
const int SLOT_COUNT = ${TEXTURE_SLOTS.length};
uniform int uTexCoordSlot[SLOT_COUNT];
uniform mat3 uTexMatrix[SLOT_COUNT];

out vec4 outColor;

const float PI = 3.14159265359;

vec2 selectUv(int texCoord) {
    return texCoord == 1 ? vTexCoord1 : vTexCoord;
}

/** The UV one material slot samples at, transform applied. */
vec2 slotUv(int slot) {
    return (uTexMatrix[slot] * vec3(selectUv(uTexCoordSlot[slot]), 1.0)).xy;
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

/**
 * Bend N by a tangent-space normal map sample.
 *
 * The frame is derived from screen-space derivatives because the preview does
 * not upload TANGENT. It is built from vectors perpendicular to N (the
 * cotangent frame) so it stays orthogonal to the shading normal, and both axes
 * are scaled by one common factor: the raw derivatives are world units *per
 * pixel* times UV per pixel, so their magnitude says nothing about whether the
 * frame is usable — testing them against a fixed epsilon silently switches
 * normal mapping off on small models and at high resolutions.
 */
vec3 applyTangentNormal(vec3 N, vec2 uv, vec3 tangentNormal) {
    vec3 dpdx = dFdx(vWorldPos);
    vec3 dpdy = dFdy(vWorldPos);
    vec2 duvdx = dFdx(uv);
    vec2 duvdy = dFdy(uv);
    vec3 dpdxPerp = cross(N, dpdx);
    vec3 dpdyPerp = cross(dpdy, N);
    vec3 T = dpdyPerp * duvdx.x + dpdxPerp * duvdy.x;
    vec3 B = dpdyPerp * duvdx.y + dpdxPerp * duvdy.y;
    float longest = max(dot(T, T), dot(B, B));
    // Only a genuinely absent UV gradient leaves the normal untouched.
    if (longest <= 0.0) return N;
    float invMax = inversesqrt(longest);
    return normalize(mat3(T * invMax, B * invMax, N) * tangentNormal);
}

void main() {
    vec4 base = uBaseColorFactor;
    if (uHasVertexColors == 1) base *= vColor;
    vec4 baseSample = vec4(1.0);
    if (uHasTexture == 1) {
        baseSample = texture(uBaseColor, slotUv(SLOT_BASE_COLOR));
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

    // Kept before the base normal map: the clearcoat layer has its own normal,
    // and when it ships none it follows the geometry rather than the coated
    // surface underneath.
    vec3 geometricN = N;

    if (uHasNormalTexture == 1) {
        vec2 uv = slotUv(SLOT_NORMAL);
        vec3 tangentNormal = texture(uNormalTexture, uv).xyz * 2.0 - 1.0;
        tangentNormal.xy *= uNormalScale;
        N = applyTangentNormal(N, uv, tangentNormal);
    }

    vec3 V = normalize(uCameraPos - vWorldPos);
    vec3 baseColor = uBaseColorFactor.rgb;
    if (uHasVertexColors == 1) baseColor *= vColor.rgb;
    if (uHasTexture == 1) baseColor *= pow(baseSample.rgb, vec3(2.2));

    float metallic = uMetallic;
    float roughness = clamp(uRoughness, 0.045, 1.0);
    if (uHasMetallicRoughnessTexture == 1) {
        vec4 packed = texture(uMetallicRoughness, slotUv(SLOT_METALLIC_ROUGHNESS));
        roughness = clamp(roughness * packed.g, 0.045, 1.0);
        metallic *= packed.b;
    }

    float occlusion = 1.0;
    if (uHasOcclusionTexture == 1) {
        occlusion = mix(1.0, texture(uOcclusionTexture, slotUv(SLOT_OCCLUSION)).r, uOcclusionStrength);
    }
    // Dielectric reflectance from the index of refraction (0.04 at the glTF
    // default of 1.5), tinted and scaled by KHR_materials_specular. Metals keep
    // taking their f0 from the base color, so the weight only fades dielectrics.
    float iorF0 = (uIor - 1.0) / (uIor + 1.0);
    iorF0 *= iorF0;
    float specularWeight = uSpecularFactor;
    if (uHasSpecularTexture == 1) {
        specularWeight *= texture(uSpecularTexture, slotUv(SLOT_SPECULAR)).a;
    }
    vec3 specularColor = uSpecularColorFactor;
    if (uHasSpecularColorTexture == 1) {
        specularColor *= pow(texture(uSpecularColorTexture, slotUv(SLOT_SPECULAR_COLOR)).rgb, vec3(2.2));
    }
    vec3 f0 = mix(min(vec3(iorF0) * specularColor, vec3(1.0)), baseColor, metallic);
    float nDotV = max(dot(N, V), 0.0);
    vec3 iblFresnel = fresnelSchlickRoughness(nDotV, f0, roughness);
    vec3 diffuseWeight = (1.0 - iblFresnel) * (1.0 - metallic);
    vec3 irradiance = texture(uIrradianceMap, N).rgb;
    vec3 diffuseIbl = irradiance * baseColor / PI;
    vec3 reflected = reflect(-V, N);
    vec3 prefiltered = textureLod(uPrefilteredMap, reflected, roughness * uEnvironmentMaxLod).rgb;
    vec2 brdf = texture(uBrdfLut, vec2(nDotV, roughness)).rg;
    vec3 specularIbl = prefiltered * (f0 * brdf.x + brdf.y) * mix(specularWeight, 1.0, metallic);
    vec3 color = (diffuseWeight * diffuseIbl + specularIbl) * occlusion;
    vec3 emissive = uEmissiveFactor;
    if (uHasEmissiveTexture == 1) {
        emissive *= pow(texture(uEmissive, slotUv(SLOT_EMISSIVE)).rgb, vec3(2.2));
    }
    color += emissive;

    // Clearcoat sits on top of everything below, emission included: the layer
    // reflects its own share of the environment and dims what shows through it.
    float clearcoat = uClearcoatFactor;
    if (uHasClearcoatTexture == 1) {
        clearcoat *= texture(uClearcoatTexture, slotUv(SLOT_CLEARCOAT)).r;
    }
    if (clearcoat > 0.0) {
        float coatRoughness = uClearcoatRoughnessFactor;
        if (uHasClearcoatRoughnessTexture == 1) {
            coatRoughness *= texture(uClearcoatRoughnessTexture, slotUv(SLOT_CLEARCOAT_ROUGHNESS)).g;
        }
        coatRoughness = clamp(coatRoughness, 0.045, 1.0);
        vec3 coatN = geometricN;
        if (uHasClearcoatNormalTexture == 1) {
            vec2 uv = slotUv(SLOT_CLEARCOAT_NORMAL);
            vec3 tangentNormal = texture(uClearcoatNormalTexture, uv).xyz * 2.0 - 1.0;
            tangentNormal.xy *= uClearcoatNormalScale;
            coatN = applyTangentNormal(coatN, uv, tangentNormal);
        }
        float coatNdotV = max(dot(coatN, V), 0.0);
        vec3 coatPrefiltered = textureLod(
            uPrefilteredMap, reflect(-V, coatN), coatRoughness * uEnvironmentMaxLod).rgb;
        vec2 coatBrdf = texture(uBrdfLut, vec2(coatNdotV, coatRoughness)).rg;
        vec3 coatSpecular = coatPrefiltered * (0.04 * coatBrdf.x + coatBrdf.y) * occlusion;
        float coatFresnel = clearcoat * fresnelSchlickRoughness(coatNdotV, vec3(0.04), coatRoughness).x;
        color = color * (1.0 - coatFresnel) + coatSpecular * clearcoat;
    }
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
