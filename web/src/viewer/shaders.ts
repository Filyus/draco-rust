import { MATERIAL_EXTENSION_SLOTS } from '../material-extensions.ts';
import type { MaterialExtensionShaderSlot } from '../material-extensions.ts';

/**
 * These two bound uniform array lengths declared below, so they belong with the
 * GLSL that spells them out. The morph bound is a shader loop length rather
 * than a hardware limit: a mesh may declare any number of targets, and the
 * renderer blends the strongest-weighted ones.
 */
export const MAX_JOINTS = 256;
export const MAX_ACTIVE_MORPH_TARGETS = 32;

/**
 * Every material texture slot the surface shader knows how to sample.
 *
 * This is the vocabulary, not the layout: a given program declares only the
 * slots the material it was built for actually binds, and their GLSL `SLOT_*`
 * indices and texture units are dense over *that* subset. The renderer builds
 * its binding table by mapping over this list, so reordering it moves both
 * sides at once, which is the point.
 */
/**
 * The core metallic-roughness slots, whose sampler names predate any
 * convention and so are spelled out.
 */
const CORE_TEXTURE_SLOT_SAMPLERS = {
  BASE_COLOR: 'uBaseColor',
  METALLIC_ROUGHNESS: 'uMetallicRoughness',
  EMISSIVE: 'uEmissive',
  NORMAL: 'uNormalTexture',
  OCCLUSION: 'uOcclusionTexture',
} as const;

export const TEXTURE_SLOTS = [
  ...(Object.keys(CORE_TEXTURE_SLOT_SAMPLERS) as (keyof typeof CORE_TEXTURE_SLOT_SAMPLERS)[]),
  ...MATERIAL_EXTENSION_SLOTS.map((entry) => entry.slot),
] as const;

export type TextureSlotName = keyof typeof CORE_TEXTURE_SLOT_SAMPLERS | MaterialExtensionShaderSlot;

/** The sampler uniform each slot is declared as, when the program has it. */
export const TEXTURE_SLOT_SAMPLERS: Record<TextureSlotName, string> = {
  ...CORE_TEXTURE_SLOT_SAMPLERS,
  ...Object.fromEntries(MATERIAL_EXTENSION_SLOTS.map((entry) => [entry.slot, entry.sampler])),
} as Record<TextureSlotName, string>;

/**
 * The per-slot declarations one surface program carries.
 *
 * Only the slots handed in exist: their sampler, their `SLOT_*` index and their
 * entry in the two UV arrays are all dense over the subset, and the body reads
 * `HAS_<SLOT>` to know whether to sample. That is what keeps the sampler count
 * a property of one material rather than of everything the viewer supports —
 * WebGL2 guarantees sixteen units, and the full slot list plus the frame's own
 * samplers would not fit once the layered extensions arrive.
 */
function slotDeclarations(slots: readonly TextureSlotName[]): string {
  if (slots.length === 0) return '';
  return [
    ...slots.map((name, index) => `#define HAS_${name} 1\nconst int SLOT_${name} = ${index};`),
    `const int SLOT_COUNT = ${slots.length};`,
    'uniform int uTexCoordSlot[SLOT_COUNT];',
    'uniform mat3 uTexMatrix[SLOT_COUNT];',
    ...slots.map((name) => `uniform sampler2D ${TEXTURE_SLOT_SAMPLERS[name]};`),
    '',
    '/** The UV one material slot samples at, transform applied. */',
    'vec2 slotUv(int slot) {',
    '    return (uTexMatrix[slot] * vec3(selectUv(uTexCoordSlot[slot]), 1.0)).xy;',
    '}',
  ].join('\n');
}

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

/**
 * The surface program's fragment source, built for one set of texture slots.
 *
 * `slots` is the subset of `TEXTURE_SLOTS` the material being drawn actually
 * binds, in slot-list order. Everything else about the shader is the same
 * whichever set it is: the factors are always uniforms, because uniforms are
 * not the scarce resource — texture units are.
 */
export function buildSurfaceFragmentSource(slots: readonly TextureSlotName[]) {
  return `#version 300 es
precision highp float;

in vec3 vNormal;
in vec2 vTexCoord;
in vec2 vTexCoord1;
in vec4 vColor;
in vec3 vWorldPos;

uniform int uHasNormals;
uniform int uHasVertexColors;
uniform int uUnlit;
uniform int uBaseColorOnly;
uniform vec4 uBaseColorFactor;
uniform float uMetallic;
uniform float uRoughness;
uniform vec3 uEmissiveFactor;
uniform float uNormalScale;
uniform float uOcclusionStrength;
// KHR_materials_ior / KHR_materials_specular: the dielectric reflectance the
// index of refraction implies, tinted and weighted by the specular extension.
uniform float uIor;
uniform float uSpecularFactor;
uniform vec3 uSpecularColorFactor;
// KHR_materials_iridescence: a thin film over the specular lobe, whose
// thickness in nanometres decides which wavelengths it reinforces.
uniform float uIridescenceFactor;
uniform float uIridescenceIor;
uniform float uIridescenceThicknessMinimum;
uniform float uIridescenceThicknessMaximum;
// KHR_materials_sheen: a retroreflective lobe for cloth, over the base layer.
uniform vec3 uSheenColorFactor;
uniform float uSheenRoughnessFactor;
// KHR_materials_clearcoat: a second specular lobe over the whole material.
uniform float uClearcoatFactor;
uniform float uClearcoatRoughnessFactor;
uniform float uClearcoatNormalScale;
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

// Per-slot UV state: which TEXCOORD set the slot reads, and the
// KHR_texture_transform matrix to run it through. The matrix is built on the
// CPU, so the shader neither knows nor cares whether the extension was present:
// an absent transform arrives as the identity.
${slotDeclarations(slots)}

vec3 fresnelSchlickRoughness(float cosTheta, vec3 f0, float roughness) {
    return f0 + (max(vec3(1.0 - roughness), f0) - f0) * pow(1.0 - cosTheta, 5.0);
}

/**
 * The Fresnel reflectance of a thin film over a base of reflectance baseF0.
 *
 * KHR_materials_iridescence is defined against Belcour and Barla's "A
 * Practical Extension to Microfacet Theory for the Modeling of Varying
 * Iridescence" (2017): light reflected off the film's two interfaces
 * interferes, and which wavelengths survive depends on the optical path
 * difference — the film's thickness times its index, twice, along the
 * refracted direction. The spectrum is integrated against Gaussian fits of the
 * eye's response and converted to linear sRGB, which is what makes a
 * few-hundred-nanometre film read as a colour rather than as a brightness.
 */
vec3 filmSensitivity(float opd, vec3 shift) {
    // Three Gaussians in wavenumber, fitted to the CIE curves; the third is a
    // second lobe of the x-bar response, which is why it is added to x alone.
    float phase = 6.2831853 * opd * 1.0e-9;
    vec3 val = vec3(5.4856e-13, 4.4201e-13, 5.2481e-13);
    vec3 pos = vec3(1.6810e+06, 1.7953e+06, 2.2084e+06);
    vec3 var_ = vec3(4.3278e+09, 9.3046e+09, 6.6121e+09);
    vec3 xyz = val * sqrt(6.2831853 * var_) * cos(pos * phase + shift) * exp(-var_ * phase * phase);
    float x = 9.7470e-14 * sqrt(6.2831853 * 4.5282e+09)
        * cos(2.2399e+06 * phase + shift[0]) * exp(-4.5282e+09 * phase * phase);
    xyz = vec3(xyz.x + x, xyz.y, xyz.z) / 1.0685e-7;
    return mat3(
        3.2404542, -0.9692660, 0.0556434,
        -1.5371385, 1.8760108, -0.2040259,
        -0.4985314, 0.0415560, 1.0572252
    ) * xyz;
}

vec3 iorToF0(vec3 transmitted, float incident) {
    vec3 ratio = (transmitted - vec3(incident)) / (transmitted + vec3(incident));
    return ratio * ratio;
}

vec3 f0ToIor(vec3 f0) {
    vec3 root = sqrt(clamp(f0, 0.0, 0.9999));
    return (vec3(1.0) + root) / (vec3(1.0) - root);
}

vec3 iridescentFresnel(float filmIor, float cosTheta1, float thickness, vec3 baseF0) {
    // Reflection off the outer face of the film. A film thinner than a
    // wavelength has nothing to interfere with, so it fades back to the base.
    float iridescenceIor = mix(1.0, filmIor, smoothstep(0.0, 0.03, thickness));
    float sinTheta2Sq = 1.0 - cosTheta1 * cosTheta1;
    sinTheta2Sq /= iridescenceIor * iridescenceIor;
    if (sinTheta2Sq > 1.0) return vec3(1.0); // total internal reflection
    float cosTheta2 = sqrt(1.0 - sinTheta2Sq);

    float r0 = (1.0 - iridescenceIor) / (1.0 + iridescenceIor);
    r0 *= r0;
    float r12 = r0 + (1.0 - r0) * pow(1.0 - cosTheta1, 5.0);
    float r21 = r12;
    float t121 = 1.0 - r12;

    // The base seen through the film, in the film's own index space.
    vec3 baseIor = f0ToIor(clamp(baseF0, 0.0, 0.9999));
    vec3 r1 = iorToF0(baseIor, iridescenceIor);
    vec3 r23 = r1 + (vec3(1.0) - r1) * pow(1.0 - cosTheta2, 5.0);

    float phi12 = iridescenceIor < 1.0 ? PI : 0.0;
    float phi21 = PI - phi12;
    vec3 phi23 = vec3(
        baseIor.r < iridescenceIor ? PI : 0.0,
        baseIor.g < iridescenceIor ? PI : 0.0,
        baseIor.b < iridescenceIor ? PI : 0.0
    );
    float opd = 2.0 * iridescenceIor * thickness * cosTheta2;
    vec3 phi = vec3(phi21) + phi23;

    vec3 r123 = clamp(r12 * r23, 1e-5, 0.9999);
    vec3 rs = (t121 * t121) * r23 / (vec3(1.0) - r123);

    // The first bounce, then every subsequent one, summed as a series.
    vec3 c0 = vec3(r12) + rs;
    vec3 sum = c0;
    vec3 cm = rs - vec3(t121);
    for (int m = 1; m <= 2; m += 1) {
        cm *= sqrt(r123);
        sum += 2.0 * cm * filmSensitivity(float(m) * opd, float(m) * phi);
    }
    return max(sum, vec3(0.0));
}

/**
 * How much of the environment a Charlie sheen lobe returns at this angle.
 *
 * KHR_materials_sheen is defined against the Charlie distribution, whose
 * directional albedo has no closed form; the extension ships a lookup texture
 * for it. This is the published analytic fit of that same term (Estevez and
 * Kulla, "Production Friendly Microfacet Sheen BRDF"), which the glTF sample
 * viewer and three.js both use in place of the texture. Two curves, because
 * the fit changes shape below a quarter roughness.
 */
float sheenDirectionalAlbedo(float nDotV, float roughness) {
    float r2 = roughness * roughness;
    bool smooth_ = roughness < 0.25;
    float a = smooth_ ? -339.2 * r2 + 161.4 * roughness - 25.9
                      : -8.48 * r2 + 14.3 * roughness - 9.95;
    float b = smooth_ ? 44.0 * r2 - 23.7 * roughness + 3.26
                      : 1.97 * r2 - 3.27 * roughness + 0.72;
    float fit = exp(a * nDotV + b) + (smooth_ ? 0.0 : 0.1 * (roughness - 0.25));
    return clamp(fit / PI, 0.0, 1.0);
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
    #ifdef HAS_BASE_COLOR
    baseSample = texture(uBaseColor, slotUv(SLOT_BASE_COLOR));
    #endif
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

    #ifdef HAS_NORMAL
    vec2 uv = slotUv(SLOT_NORMAL);
    vec3 tangentNormal = texture(uNormalTexture, uv).xyz * 2.0 - 1.0;
    tangentNormal.xy *= uNormalScale;
    N = applyTangentNormal(N, uv, tangentNormal);
    #endif

    vec3 V = normalize(uCameraPos - vWorldPos);
    vec3 baseColor = uBaseColorFactor.rgb;
    if (uHasVertexColors == 1) baseColor *= vColor.rgb;
    #ifdef HAS_BASE_COLOR
    baseColor *= pow(baseSample.rgb, vec3(2.2));
    #endif

    float metallic = uMetallic;
    float roughness = clamp(uRoughness, 0.045, 1.0);
    #ifdef HAS_METALLIC_ROUGHNESS
    vec4 packed = texture(uMetallicRoughness, slotUv(SLOT_METALLIC_ROUGHNESS));
    roughness = clamp(roughness * packed.g, 0.045, 1.0);
    metallic *= packed.b;
    #endif

    float occlusion = 1.0;
    #ifdef HAS_OCCLUSION
    occlusion = mix(1.0, texture(uOcclusionTexture, slotUv(SLOT_OCCLUSION)).r, uOcclusionStrength);
    #endif
    // Dielectric reflectance from the index of refraction (0.04 at the glTF
    // default of 1.5), tinted and scaled by KHR_materials_specular. Metals keep
    // taking their f0 from the base color, so the weight only fades dielectrics.
    float iorF0 = (uIor - 1.0) / (uIor + 1.0);
    iorF0 *= iorF0;
    float specularWeight = uSpecularFactor;
    #ifdef HAS_SPECULAR
    specularWeight *= texture(uSpecularTexture, slotUv(SLOT_SPECULAR)).a;
    #endif
    vec3 specularColor = uSpecularColorFactor;
    #ifdef HAS_SPECULAR_COLOR
    specularColor *= pow(texture(uSpecularColorTexture, slotUv(SLOT_SPECULAR_COLOR)).rgb, vec3(2.2));
    #endif
    vec3 f0 = mix(min(vec3(iorF0) * specularColor, vec3(1.0)), baseColor, metallic);
    float nDotV = max(dot(N, V), 0.0);

    // KHR_materials_iridescence replaces the Fresnel term with the thin film's
    // own, weighted by the factor: the lobe stays where it was, only what it
    // reflects changes colour with the angle.
    float iridescence = uIridescenceFactor;
    #ifdef HAS_IRIDESCENCE
    iridescence *= texture(uIridescenceTexture, slotUv(SLOT_IRIDESCENCE)).r;
    #endif
    if (iridescence > 0.0) {
        float thicknessMix = 1.0;
        #ifdef HAS_IRIDESCENCE_THICKNESS
        thicknessMix = texture(uIridescenceThicknessTexture, slotUv(SLOT_IRIDESCENCE_THICKNESS)).g;
        #endif
        float thickness = mix(uIridescenceThicknessMinimum, uIridescenceThicknessMaximum, thicknessMix);
        f0 = mix(f0, clamp(iridescentFresnel(uIridescenceIor, nDotV, thickness, f0), 0.0, 1.0), iridescence);
    }
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
    #ifdef HAS_EMISSIVE
    emissive *= pow(texture(uEmissive, slotUv(SLOT_EMISSIVE)).rgb, vec3(2.2));
    #endif
    color += emissive;

    // KHR_materials_sheen: a broad retroreflective lobe for cloth. It is
    // energy-conserving against the layer underneath rather than added on top,
    // so what the sheen returns is taken out of the base first — a velvet rim
    // that also brightened the whole surface would read as emission.
    vec3 sheenColor = uSheenColorFactor;
    #ifdef HAS_SHEEN_COLOR
    sheenColor *= pow(texture(uSheenColorTexture, slotUv(SLOT_SHEEN_COLOR)).rgb, vec3(2.2));
    #endif
    if (sheenColor.r + sheenColor.g + sheenColor.b > 0.0) {
        float sheenRoughness = uSheenRoughnessFactor;
        #ifdef HAS_SHEEN_ROUGHNESS
        sheenRoughness *= texture(uSheenRoughnessTexture, slotUv(SLOT_SHEEN_ROUGHNESS)).a;
        #endif
        sheenRoughness = clamp(sheenRoughness, 0.07, 1.0);
        float sheenAlbedo = sheenDirectionalAlbedo(nDotV, sheenRoughness);
        // Sheen is a wide lobe, so it gathers the environment far off the
        // mirror direction: the surface normal at a roughness-driven level is
        // closer to what it integrates than the reflection is.
        vec3 sheenIbl = textureLod(uPrefilteredMap, N, sheenRoughness * uEnvironmentMaxLod).rgb;
        color *= 1.0 - max(max(sheenColor.r, sheenColor.g), sheenColor.b) * sheenAlbedo;
        color += sheenIbl * sheenColor * sheenAlbedo * occlusion;
    }

    // Clearcoat sits on top of everything below, emission included: the layer
    // reflects its own share of the environment and dims what shows through it.
    float clearcoat = uClearcoatFactor;
    #ifdef HAS_CLEARCOAT
    clearcoat *= texture(uClearcoatTexture, slotUv(SLOT_CLEARCOAT)).r;
    #endif
    if (clearcoat > 0.0) {
        float coatRoughness = uClearcoatRoughnessFactor;
        #ifdef HAS_CLEARCOAT_ROUGHNESS
        coatRoughness *= texture(uClearcoatRoughnessTexture, slotUv(SLOT_CLEARCOAT_ROUGHNESS)).g;
        #endif
        coatRoughness = clamp(coatRoughness, 0.045, 1.0);
        vec3 coatN = geometricN;
        #ifdef HAS_CLEARCOAT_NORMAL
        vec2 uv = slotUv(SLOT_CLEARCOAT_NORMAL);
        vec3 tangentNormal = texture(uClearcoatNormalTexture, uv).xyz * 2.0 - 1.0;
        tangentNormal.xy *= uClearcoatNormalScale;
        coatN = applyTangentNormal(coatN, uv, tangentNormal);
        #endif
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
}

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
