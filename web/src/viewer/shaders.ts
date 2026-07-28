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
 * How many punctual lights one draw can shade with.
 *
 * A loop bound rather than a hardware one, like the morph bound above: a scene
 * may declare any number, and the renderer sends the ones nearest the model.
 */
export const MAX_PUNCTUAL_LIGHTS = 8;

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

/**
 * How many wavelengths a dispersive surface is sampled at.
 *
 * Three - one per primary - is what the extension's reference implementation
 * does and what it costs the least, but three samples of a continuous spectrum
 * alias into three coloured ghosts rather than a spread. How many it takes to
 * read as a spread depends on how far apart they land, and at the factors that
 * asset tests - five, which is an Abbe number of four, lower than any real
 * glass - they land far apart indeed. Two dozen, sampled with a single
 * bilinear tap each rather than a bicubic sixteen, costs about what a dozen
 * bicubic ones did.
 */
const SPECTRAL_SAMPLES = 24;

/**
 * The CIE 1931 observer at one wavelength, as linear sRGB.
 *
 * Wyman, Sloan and Shirley's analytic fits of the colour matching functions -
 * a couple of skewed Gaussians apiece - through the D65 matrix. Computed here
 * rather than in GLSL because it is the same table every frame.
 */
function observerAt(wavelength: number): [number, number, number] {
  const lobe = (mean: number, below: number, above: number) => {
    const spread = wavelength < mean ? below : above;
    const offset = (wavelength - mean) / spread;
    return Math.exp(-0.5 * offset * offset);
  };
  const x = 1.056 * lobe(599.8, 37.9, 31.0)
    + 0.362 * lobe(442.0, 16.0, 26.7)
    - 0.065 * lobe(501.1, 20.4, 26.2);
  const y = 0.821 * lobe(568.8, 46.9, 40.5) + 0.286 * lobe(530.9, 16.3, 31.1);
  const z = 1.217 * lobe(437.0, 11.8, 36.0) + 0.681 * lobe(459.0, 26.0, 13.8);
  return [
    3.2406 * x - 1.5372 * y - 0.4986 * z,
    -0.9689 * x + 1.8758 * y + 0.0415 * z,
    0.0557 * x - 0.2040 * y + 1.0570 * z,
  ];
}

/**
 * The wavelengths and their weights, as a GLSL table.
 *
 * Sampled between the Fraunhofer F and C lines rather than across the whole
 * visible range, because that interval is the one the dispersion factor is
 * defined on: the Abbe number is the ratio of the index at the sodium line to
 * the spread between these two, and the fit the extension publishes places the
 * primaries at exactly that spread apart. Sampling out to violet is more of
 * the spectrum than the number describes, and at the factors that asset tests
 * it doubles the width of every fringe.
 *
 * Normalised so the weights sum to one in every channel: a surface reading a
 * flat image has to hand it back unchanged, whatever its Abbe number, or
 * dispersion would tint everything it touched.
 */
function spectralTable(): string {
  const shortest = 486.13;
  const longest = 656.27;
  const wavelengths: number[] = [];
  const weights: [number, number, number][] = [];
  for (let index = 0; index < SPECTRAL_SAMPLES; index += 1) {
    const wavelength = shortest + (longest - shortest) * ((index + 0.5) / SPECTRAL_SAMPLES);
    wavelengths.push(wavelength);
    weights.push(observerAt(wavelength).map((value) => Math.max(value, 0)) as [number, number, number]);
  }
  const totals = weights.reduce(
    (sum, weight) => [sum[0] + weight[0], sum[1] + weight[1], sum[2] + weight[2]],
    [0, 0, 0],
  );
  const glsl = (values: number[]) => `vec3(${values.map((v) => v.toFixed(6)).join(', ')})`;
  return [
    `const int SPECTRAL_SAMPLES = ${SPECTRAL_SAMPLES};`,
    `const float SPECTRAL_WAVELENGTH[SPECTRAL_SAMPLES] = float[SPECTRAL_SAMPLES](`,
    `    ${wavelengths.map((value) => value.toFixed(2)).join(', ')});`,
    `const vec3 SPECTRAL_WEIGHT[SPECTRAL_SAMPLES] = vec3[SPECTRAL_SAMPLES](`,
    `    ${weights.map((weight) => glsl(weight.map((value, channel) => value / totals[channel]))).join(',\n    ')});`,
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
// EXT_mesh_gpu_instancing: one mat4 per copy, as four columns. A draw that is
// not instanced leaves these disabled, and the constant attribute the renderer
// sets is the identity - so the same program draws both.
layout(location=7) in vec4 aInstanceColumn0;
layout(location=8) in vec4 aInstanceColumn1;
layout(location=9) in vec4 aInstanceColumn2;
layout(location=10) in vec4 aInstanceColumn3;

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
    mat4 instance = mat4(aInstanceColumn0, aInstanceColumn1, aInstanceColumn2, aInstanceColumn3);
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
    vNormal = normalize(mat3(instance) * vNormal);

    // The instance transform sits under the node's own: each copy is placed
    // relative to the node, which is what the extension means.
    vec4 worldPos = uModel * instance * skinned;
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
// KHR_materials_transmission / KHR_materials_volume: what passes through the
// surface, and the interior it crosses on the way.
// KHR_materials_anisotropy: a specular lobe stretched along a tangent.
uniform float uAnisotropyStrength;
uniform float uAnisotropyRotation;
uniform float uTransmissionFactor;
uniform float uDispersion;
uniform float uThicknessFactor;
uniform float uAttenuationDistance;
uniform vec3 uAttenuationColor;
// The opaque half of the frame, which the transmitted ray reads out of.
uniform sampler2D uFrameSnapshot;
// The far wall of this volume, as world normal and distance from the eye. A
// zero normal means the pass found none, and the authored thickness stands in.
uniform sampler2D uBackFace;
uniform vec2 uFrameSize;
uniform float uFrameMaxLod;
uniform mat4 uProjection;
uniform mat4 uView;
// The node transform, which the fragment stage needs for one thing only: the
// volume's thickness is stated in local space, so the ray inside it has to be
// carried into world space before it is projected back onto the frame.
uniform mat4 uModel;
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
// KHR_lights_punctual, resolved to world space by the renderer. Type 0 is
// directional, 1 point, 2 spot; a range of zero never falls off.
uniform int uLightCount;
uniform int uLightType[${MAX_PUNCTUAL_LIGHTS}];
uniform vec3 uLightColor[${MAX_PUNCTUAL_LIGHTS}];
uniform vec3 uLightPosition[${MAX_PUNCTUAL_LIGHTS}];
uniform vec3 uLightDirection[${MAX_PUNCTUAL_LIGHTS}];
uniform vec4 uLightParams[${MAX_PUNCTUAL_LIGHTS}];

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

${spectralTable()}

vec3 fresnelSchlickRoughness(float cosTheta, vec3 f0, float roughness) {
    return f0 + (max(vec3(1.0 - roughness), f0) - f0) * pow(1.0 - cosTheta, 5.0);
}

/** GGX normal distribution, the microfacet term of the direct lobe. */
float distributionGgx(float nDotH, float roughness) {
    float a = roughness * roughness;
    float a2 = a * a;
    float d = nDotH * nDotH * (a2 - 1.0) + 1.0;
    return a2 / max(PI * d * d, 1e-7);
}

/** Smith visibility, height-correlated, already divided by the 4 NdotL NdotV. */
float visibilitySmith(float nDotL, float nDotV, float roughness) {
    float a = roughness * roughness;
    float a2 = a * a;
    float v = nDotL * sqrt(nDotV * nDotV * (1.0 - a2) + a2);
    float l = nDotV * sqrt(nDotL * nDotL * (1.0 - a2) + a2);
    return 0.5 / max(v + l, 1e-7);
}

/**
 * What one punctual light delivers here: its direction and its radiance.
 *
 * The three types differ only in how the two are found. Distance attenuation
 * is the extension's own: inverse square, cut off by the range when it states
 * one, with the window term that takes it smoothly to nothing at the edge.
 */
void punctualLight(int index, vec3 position, out vec3 lightDir, out vec3 radiance) {
    int type = uLightType[index];
    vec3 color = uLightColor[index] * uLightParams[index].x;
    if (type == 0) {
        lightDir = -uLightDirection[index];
        radiance = color;
        return;
    }
    vec3 toLight = uLightPosition[index] - position;
    float distance = length(toLight);
    lightDir = toLight / max(distance, 1e-6);
    float attenuation = 1.0 / max(distance * distance, 1e-6);
    float range = uLightParams[index].y;
    if (range > 0.0) {
        float ratio = distance / range;
        float window = clamp(1.0 - ratio * ratio * ratio * ratio, 0.0, 1.0);
        attenuation *= window * window;
    }
    if (type == 2) {
        // The cone: full inside the inner angle, nothing outside the outer one.
        float cosOuter = cos(uLightParams[index].w);
        float cosInner = cos(uLightParams[index].z);
        float cosAngle = dot(-lightDir, uLightDirection[index]);
        attenuation *= clamp((cosAngle - cosOuter) / max(cosInner - cosOuter, 1e-4), 0.0, 1.0);
    }
    radiance = color * attenuation;
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
 * What the frame behind this surface looks like through it.
 *
 * The ray is refracted at the surface and followed for the volume's thickness,
 * and where it comes out is projected back onto the frame that was captured
 * before any of this drew. That is a screen-space approximation - it can only
 * show what the opaque pass left visible - but it is the one the extension is
 * written for, and it moves correctly with the index of refraction, which a
 * plain alpha blend does not.
 *
 * Roughness picks the mip level: a rough transmissive surface scatters, and
 * the mip chain is the blur that stands in for it.
 */
/**
 * The refracted ray across the volume, in world units.
 *
 * KHR_materials_volume states the thickness in the node's own space, so the
 * model's scale belongs here rather than in the caller: the same glass scaled
 * up is a thicker piece of glass, and a ray that ignored that would exit at the
 * unscaled depth and read the wrong pixel.
 */
vec3 volumeTransmissionRay(vec3 normal, vec3 view, float ior, float thickness) {
    // Which way the interface is crossed decides the ratio. A back face is the
    // far wall of the volume seen from inside it, where light leaves a dense
    // medium for air - and where, past the critical angle, it does not leave
    // at all - refract says so by returning nothing, and the caller reflects
    // instead.
    float eta = gl_FrontFacing ? 1.0 / max(ior, 1.0001) : max(ior, 1.0001);
    vec3 refracted = refract(-view, normalize(normal), eta);
    if (dot(refracted, refracted) < 1e-8) return vec3(0.0);
    vec3 modelScale = vec3(
        length(uModel[0].xyz), length(uModel[1].xyz), length(uModel[2].xyz));
    return normalize(refracted) * thickness * modelScale;
}

/**
 * The index of refraction at one wavelength, from the material's own.
 *
 * Cauchy's equation, with its two constants pinned by the Abbe number the
 * dispersion factor states as twenty over. The extension's own implementation
 * note spreads the three primaries linearly instead, which is a fit to this
 * over the visible range; sampling the spectrum properly, there is no reason
 * to use the fit rather than the thing it fits.
 */
float dispersedIor(float ior, float wavelength) {
    if (uDispersion <= 0.0) return ior;
    const float FRAUNHOFER_F = 486.13;
    const float FRAUNHOFER_D = 587.56;
    const float FRAUNHOFER_C = 656.27;
    float abbe = 20.0 / uDispersion;
    float b = (ior - 1.0)
        / (abbe * (1.0 / (FRAUNHOFER_F * FRAUNHOFER_F) - 1.0 / (FRAUNHOFER_C * FRAUNHOFER_C)));
    float a = ior - b / (FRAUNHOFER_D * FRAUNHOFER_D);
    return a + b / (wavelength * wavelength);
}

/**
 * Which mip of the snapshot a surface of this roughness and index reads.
 *
 * Roughness alone is not the blur: microfacet refraction scales with how much
 * the interface bends light at all, so an index of 1.0 - a surface that does
 * not refract - takes level zero however rough it is, and 1.5 takes the full
 * amount. That is the factor the extension's reference implementation applies.
 */
float transmissionLod(float roughness, float ior) {
    return roughness * clamp(ior * 2.0 - 2.0, 0.0, 1.0) * uFrameMaxLod;
}

/**
 * Bicubic reconstruction of the capture, across two mip levels.
 *
 * The B-spline weights are folded into four bilinear taps rather than sixteen
 * point ones, which is the standard trick and what three.js samples its own
 * transmission frame with. It matters most where roughness has sent the read
 * up the mip chain: the level under a rough pane is a fraction of the frame's
 * resolution, and reconstructing it linearly shows the texel grid as facets on
 * something that should read as frosted.
 */
float bicubicW0(float a) { return (1.0 / 6.0) * (a * (a * (-a + 3.0) - 3.0) + 1.0); }
float bicubicW1(float a) { return (1.0 / 6.0) * (a * a * (3.0 * a - 6.0) + 4.0); }
float bicubicW2(float a) { return (1.0 / 6.0) * (a * (a * (-3.0 * a + 3.0) + 3.0) + 1.0); }
float bicubicW3(float a) { return (1.0 / 6.0) * (a * a * a); }
float bicubicG0(float a) { return bicubicW0(a) + bicubicW1(a); }
float bicubicG1(float a) { return bicubicW2(a) + bicubicW3(a); }
float bicubicH0(float a) { return -1.0 + bicubicW1(a) / (bicubicW0(a) + bicubicW1(a)); }
float bicubicH1(float a) { return 1.0 + bicubicW3(a) / (bicubicW2(a) + bicubicW3(a)); }

vec3 bicubicLevel(vec2 uv, vec4 texelSize, float lod) {
    vec2 scaled = uv * texelSize.zw + 0.5;
    vec2 iuv = floor(scaled);
    vec2 fuv = fract(scaled);
    float g0x = bicubicG0(fuv.x);
    float g1x = bicubicG1(fuv.x);
    float h0x = bicubicH0(fuv.x);
    float h1x = bicubicH1(fuv.x);
    float h0y = bicubicH0(fuv.y);
    float h1y = bicubicH1(fuv.y);
    vec2 p0 = (vec2(iuv.x + h0x, iuv.y + h0y) - 0.5) * texelSize.xy;
    vec2 p1 = (vec2(iuv.x + h1x, iuv.y + h0y) - 0.5) * texelSize.xy;
    vec2 p2 = (vec2(iuv.x + h0x, iuv.y + h1y) - 0.5) * texelSize.xy;
    vec2 p3 = (vec2(iuv.x + h1x, iuv.y + h1y) - 0.5) * texelSize.xy;
    return bicubicG0(fuv.y) * (g0x * textureLod(uFrameSnapshot, p0, lod).rgb
                             + g1x * textureLod(uFrameSnapshot, p1, lod).rgb)
         + bicubicG1(fuv.y) * (g0x * textureLod(uFrameSnapshot, p2, lod).rgb
                             + g1x * textureLod(uFrameSnapshot, p3, lod).rgb);
}

vec3 captureBicubic(vec2 uv, float lod) {
    vec2 lowerSize = vec2(textureSize(uFrameSnapshot, int(lod)));
    vec2 upperSize = vec2(textureSize(uFrameSnapshot, int(lod) + 1));
    vec3 lower = bicubicLevel(uv, vec4(1.0 / lowerSize, lowerSize), floor(lod));
    vec3 upper = bicubicLevel(uv, vec4(1.0 / upperSize, upperSize), ceil(lod));
    return mix(lower, upper, fract(lod));
}

/**
 * Which way the far wall of this volume faces, where there is one.
 *
 * How *far* it is does not come from here. The extension states thickness as a
 * material property in mesh space, and a conformant renderer uses the number
 * the author wrote: the asset that exists to test dispersion states a
 * hundredth of a unit for prisms far deeper than that, and measuring them
 * instead multiplies every refraction by the ratio. Geometry answers the
 * question the file cannot - which way the light leaves - and nothing else.
 */
bool farWallNormal(out vec3 wallNormal) {
    vec4 wall = texture(uBackFace, gl_FragCoord.xy / uFrameSize);
    wallNormal = wall.xyz;
    return dot(wallNormal, wallNormal) > 1e-4
        && wall.w > distance(uCameraPos, vWorldPos);
}

/**
 * Where on the frame a ray leaving here at this index comes out.
 *
 * Bent at the near wall, carried across the volume, and bent again at the far
 * one - which is where a prism becomes a prism. Without the far wall in hand
 * the ray is only bent once and walked the authored thickness, which is what
 * every screen-space refraction does and what makes glass look like a decal.
 */
vec2 exitCoords(vec3 position, vec3 normal, vec3 view, float ior, float thickness) {
    vec3 inside = volumeTransmissionRay(normal, view, ior, thickness);
    vec3 exitPoint = position + inside;
    // A thickness of zero is the extension's thin-walled surface: no interior
    // for a ray to cross, and so no far wall to leave through.
    vec3 wallNormal;
    if (thickness > 0.0 && dot(inside, inside) > 1e-8 && farWallNormal(wallNormal)) {
        // Out through the far wall, whose normal faces away from the eye: the
        // ratio is the reciprocal of the way in, and past the critical angle
        // the ray stays inside and the exit is the wall itself. Where the two
        // walls are parallel this bends the ray back the way it came, which is
        // exactly what a slab does; where they are not, what is left is the
        // deviation that makes a prism a prism.
        vec3 outward = refract(normalize(inside), -normalize(wallNormal), max(ior, 1.0001));
        if (dot(outward, outward) > 1e-8) exitPoint += normalize(outward) * length(inside);
    }
    vec4 clip = uProjection * uView * vec4(exitPoint, 1.0);
    return clamp(clip.xy / clip.w * 0.5 + 0.5, vec2(0.0), vec2(1.0));
}

vec3 sampleCaptured(vec3 position, vec3 normal, vec3 view, float ior, float thickness, float roughness) {
    return captureBicubic(
        exitCoords(position, normal, view, ior, thickness), transmissionLod(roughness, ior));
}

vec3 transmittedRadiance(vec3 position, vec3 normal, vec3 view, float ior, float thickness, float roughness, out float rayLength) {
    vec3 ray = volumeTransmissionRay(normal, view, ior, thickness);
    // How far the light travelled through the medium, which is what Beer's law
    // absorbs over. Dispersion parts the wavelengths by a fraction of a percent
    // in index, so one length stands for all of them.
    rayLength = length(ray);
    // KHR_materials_dispersion: one index per channel instead of one for all,
    // spread by the Abbe number the extension states as its reciprocal. Zero
    // leaves the three rays on top of each other, which is the single-index
    // case, so the branch is what the extension costs when absent.
    // An index of one does not refract, so it cannot disperse either - and
    // taking the spectral path there would change nothing but the
    // reconstruction filter, which would show as a seam where the factor
    // crossed zero on a surface that never bent anything.
    if (uDispersion <= 0.0 || ior <= 1.0001) {
        return sampleCaptured(position, normal, view, ior, thickness, roughness);
    }
    // The spread the extension's reference implementation defines: the red and
    // blue ends sit half of it apart in index, green at the material's own. It
    // is proportional to how far the index is from air, so a material that does
    // not refract does not disperse either, whatever its Abbe number says.
    // A dozen wavelengths across the visible range, each bent by its own
    // index and weighted by what the eye makes of it. The weights sum to one
    // per channel, so a flat frame comes back flat however strong the
    // dispersion: what changes is where each wavelength lands, not how much of
    // it there is.
    float lod = transmissionLod(roughness, ior);
    vec3 spread = vec3(0.0);
    for (int index = 0; index < SPECTRAL_SAMPLES; index += 1) {
        float wavelengthIor = dispersedIor(ior, SPECTRAL_WAVELENGTH[index]);
        vec2 uv = exitCoords(position, normal, view, wavelengthIor, thickness);
        spread += SPECTRAL_WEIGHT[index] * textureLod(uFrameSnapshot, uv, lod).rgb;
    }
    return spread;
}

/**
 * Beer-Lambert absorption over a distance inside the volume.
 *
 * An attenuation distance of zero is the extension's "infinite": the medium
 * takes nothing out, which is also what a material with no volume at all does.
 */
vec3 attenuate(vec3 radiance, float distance) {
    if (uAttenuationDistance <= 0.0 || distance <= 0.0) return radiance;
    vec3 density = -log(clamp(uAttenuationColor, 1e-4, 1.0)) / uAttenuationDistance;
    return radiance * exp(-density * distance);
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

    // Hard unlit materials (KHR_materials_unlit) keep flat shading. Their
    // colour is the picture already, so the capture takes it back to linear
    // rather than leaving it encoded twice over.
    if (uUnlit == 1 || uBaseColorOnly == 1) {
        // Authored as a colour, so it is taken back to the light that colour
        // stands for; the output pass turns light into a picture, not this.
        outColor = vec4(pow(base.rgb, vec3(2.2)), base.a);
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
    // Kept apart on purpose. The floor exists for the specular lobes, whose
    // GGX terms degenerate at zero, and it is a lie about the surface: what
    // transmission does with roughness is pick a mip of the frame behind, and
    // a floor there makes a pane the asset called mirror-smooth read half of
    // the level below - a thin bright thing behind it arrives in blocks.
    float materialRoughness = clamp(uRoughness, 0.0, 1.0);
    #ifdef HAS_METALLIC_ROUGHNESS
    vec4 packed = texture(uMetallicRoughness, slotUv(SLOT_METALLIC_ROUGHNESS));
    materialRoughness = clamp(materialRoughness * packed.g, 0.0, 1.0);
    metallic *= packed.b;
    #endif
    float roughness = max(materialRoughness, 0.045);

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
    vec3 prefiltered;
    // KHR_materials_anisotropy stretches the lobe along a tangent, which in a
    // split-sum renderer shows as a bent reflection vector and a roughness
    // that differs along it: the direction the surface is combed reflects more
    // sharply than the one across it.
    float anisotropy = uAnisotropyStrength;
    vec2 anisotropyDirection = vec2(cos(uAnisotropyRotation), sin(uAnisotropyRotation));
    #ifdef HAS_ANISOTROPY
    vec3 anisotropySample = texture(uAnisotropyTexture, slotUv(SLOT_ANISOTROPY)).rgb;
    vec2 sampledDirection = anisotropySample.rg * 2.0 - 1.0;
    anisotropyDirection = mat2(
        anisotropyDirection.x, anisotropyDirection.y,
        -anisotropyDirection.y, anisotropyDirection.x
    ) * normalize(sampledDirection);
    anisotropy *= anisotropySample.b;
    #endif
    if (abs(anisotropy) > 0.0) {
        // The tangent frame comes from the same screen-space derivatives the
        // normal map uses, so an asset with no TANGENT still bends correctly.
        vec3 dpdx = dFdx(vWorldPos);
        vec3 dpdy = dFdy(vWorldPos);
        vec3 tangent = normalize(dpdx * anisotropyDirection.x + dpdy * anisotropyDirection.y);
        vec3 bitangent = normalize(cross(N, tangent));
        // Reflect about the axis the lobe is stretched along: at full strength
        // the highlight runs the length of it.
        vec3 bentNormal = normalize(mix(N, bitangent * dot(V, bitangent) + N, abs(anisotropy)));
        reflected = reflect(-V, bentNormal);
        prefiltered = textureLod(
            uPrefilteredMap, reflected,
            mix(roughness, 1.0, abs(anisotropy) * 0.5) * uEnvironmentMaxLod
        ).rgb;
    } else {
        prefiltered = textureLod(uPrefilteredMap, reflected, roughness * uEnvironmentMaxLod).rgb;
    }
    vec2 brdf = texture(uBrdfLut, vec2(nDotV, roughness)).rg;
    vec3 specularIbl = prefiltered * (f0 * brdf.x + brdf.y) * mix(specularWeight, 1.0, metallic);

    // KHR_materials_transmission: what the surface lets through replaces what
    // it would have scattered, rather than adding to it. The specular lobe
    // above is untouched - a pane of glass still has a highlight.
    vec3 diffuseTerm = diffuseWeight * diffuseIbl;
    float transmission = uTransmissionFactor;
    #ifdef HAS_TRANSMISSION
    transmission *= texture(uTransmissionTexture, slotUv(SLOT_TRANSMISSION)).r;
    #endif
    if (transmission > 0.0) {
        float thickness = uThicknessFactor;
        #ifdef HAS_THICKNESS
        thickness *= texture(uThicknessTexture, slotUv(SLOT_THICKNESS)).g;
        #endif
        float rayLength;
        vec3 transmitted = transmittedRadiance(
            vWorldPos, N, V, uIor, thickness, materialRoughness, rayLength);
        // Past the critical angle nothing crosses the interface; the surface
        // becomes a mirror, and what it shows is the environment it reflects.
        if (volumeTransmissionRay(N, V, uIor, 1.0) == vec3(0.0)) {
            transmitted = textureLod(
                uPrefilteredMap, reflect(-V, N), materialRoughness * uEnvironmentMaxLod).rgb;
            rayLength = 0.0;
        }
        // Tinted by the base colour, like the diffuse term it replaces: the
        // spec's transmission BTDF carries it, and without it coloured glass
        // shades as clear.
        vec3 tinted = baseColor * attenuate(transmitted, rayLength);
        diffuseTerm = mix(diffuseTerm, diffuseWeight * tinted, transmission);
    }
    vec3 color = (diffuseTerm + specularIbl) * occlusion;

    // Direct light on top of the environment. The same f0 and roughness the
    // image-based terms used, so a material shaded by both does not change
    // character between them.
    for (int i = 0; i < uLightCount; i += 1) {
        vec3 L;
        vec3 radiance;
        punctualLight(i, vWorldPos, L, radiance);
        float nDotL = max(dot(N, L), 0.0);
        if (nDotL <= 0.0) continue;
        vec3 H = normalize(L + V);
        vec3 fresnel = fresnelSchlickRoughness(max(dot(V, H), 0.0), f0, roughness);
        vec3 specular = fresnel
            * distributionGgx(max(dot(N, H), 0.0), roughness)
            * visibilitySmith(nDotL, nDotV, roughness)
            * mix(specularWeight, 1.0, metallic);
        vec3 diffuse = (vec3(1.0) - fresnel) * (1.0 - metallic) * baseColor / PI;
        color += (diffuse + specular) * radiance * nDotL;
    }
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
    outColor = vec4(color, base.a);
}
`;
}

/**
 * The far wall of a transmissive volume: its world normal, and how far away it
 * is from the eye.
 *
 * Shares the surface vertex shader, so a wall that is skinned, morphed or
 * instanced moves with the surface in front of it rather than lagging behind
 * in the bind pose.
 */
export const BACK_FACE_FRAG_SRC = `#version 300 es
precision highp float;
in vec3 vNormal;
in vec3 vWorldPos;
uniform vec3 uCameraPos;
out vec4 outColor;

void main() {
    // Facing the eye is what a far wall does not do; the pass culls front
    // faces, so the normal here points away and is flipped to face the ray
    // that will leave through it.
    vec3 normal = normalize(vNormal);
    outColor = vec4(normal, distance(uCameraPos, vWorldPos));
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

/**
 * The output pass, and the only place in the viewer that turns light into a
 * picture.
 *
 * Two things happen here that used to happen per surface, and both belong
 * after everything has been added up rather than during. Glare, because the
 * spread function of an optical system acts on the image, not on one object in
 * it. And the tone curve, because it is a display transform: applied early it
 * poisons every average taken afterwards, which is what a snapshot lifted off
 * a tone-mapped canvas kept doing.
 *
 * The curve maps luminance and rescales the channels by what it did, rather
 * than running per channel. A per-channel curve pulls whichever channel is
 * brightest down hardest, so a saturated highlight drifts toward whichever
 * primary saturates last - the reason a warm filament turns pink or a red
 * emitter turns orange as it brightens. Mapping luminance leaves the hue where
 * the material put it; what remains is that a very bright colour cannot stay
 * saturated inside the display gamut, so past the point where a channel would
 * leave it the colour walks to white, which is what film does and what the eye
 * expects of anything that bright.
 */
export const OUTPUT_FRAG_SRC = `#version 300 es
precision highp float;
in vec2 vNdc;
uniform sampler2D uScene;
uniform sampler2D uBloom;
uniform float uBloomStrength;
uniform float uExposure;
// The base-colour view is an inspection mode, not a photograph: it exists to
// show the texel the asset stores, so the curve that makes an image out of
// light has no business in it.
uniform int uToneMap;
out vec4 outColor;

float luminance(vec3 color) {
    return dot(color, vec3(0.2126, 0.7152, 0.0722));
}

/** The Narkowicz fit of the ACES curve, on one value rather than three. */
float toneCurve(float x) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), 0.0, 1.0);
}

vec3 toneMap(vec3 radiance) {
    float level = luminance(radiance);
    if (level <= 0.0) return vec3(0.0);
    float mapped = toneCurve(level);
    vec3 scaled = radiance * (mapped / level);
    // Scaling by what the curve did to luminance can push a saturated colour
    // outside the display cube even where the luminance fits, so it walks
    // toward the grey of its own luminance - but only far enough to fit, and
    // not a step further. Anything else would grey out a mid-tone red for
    // being red, and clipping instead would move the hue exactly the way
    // mapping per channel does.
    float peak = max(max(scaled.r, scaled.g), scaled.b);
    if (peak <= 1.0) return scaled;
    float fit = (peak - 1.0) / max(peak - mapped, 1e-5);
    return clamp(mix(scaled, vec3(mapped), clamp(fit, 0.0, 1.0)), 0.0, 1.0);
}

void main() {
    vec2 uv = vNdc * 0.5 + 0.5;
    // One tap at the centre of the block, which is an exact box average of the
    // supersampled frame because the filter is linear.
    vec3 scene = texture(uScene, uv).rgb;
    vec3 glare = texture(uBloom, uv).rgb;
    // Mixed rather than added: glare moves light about, it does not make any.
    vec3 radiance = mix(scene, glare, uBloomStrength) * uExposure;
    vec3 shown = uToneMap == 1 ? toneMap(radiance) : clamp(radiance, 0.0, 1.0);
    outColor = vec4(pow(shown, vec3(1.0 / 2.2)), 1.0);
}
`;

/**
 * One step down the glare pyramid: the thirteen-tap filter the pyramid is
 * usually built with, which keeps the level stable as the frame moves rather
 * than shimmering the way a plain box does.
 */
export const BLOOM_DOWN_FRAG_SRC = `#version 300 es
precision highp float;
in vec2 vNdc;
uniform sampler2D uSource;
uniform vec2 uTexel;
out vec4 outColor;

void main() {
    vec2 uv = vNdc * 0.5 + 0.5;
    vec3 a = texture(uSource, uv + uTexel * vec2(-2.0, 2.0)).rgb;
    vec3 b = texture(uSource, uv + uTexel * vec2(0.0, 2.0)).rgb;
    vec3 c = texture(uSource, uv + uTexel * vec2(2.0, 2.0)).rgb;
    vec3 d = texture(uSource, uv + uTexel * vec2(-2.0, 0.0)).rgb;
    vec3 e = texture(uSource, uv).rgb;
    vec3 f = texture(uSource, uv + uTexel * vec2(2.0, 0.0)).rgb;
    vec3 g = texture(uSource, uv + uTexel * vec2(-2.0, -2.0)).rgb;
    vec3 h = texture(uSource, uv + uTexel * vec2(0.0, -2.0)).rgb;
    vec3 i = texture(uSource, uv + uTexel * vec2(2.0, -2.0)).rgb;
    vec3 j = texture(uSource, uv + uTexel * vec2(-1.0, 1.0)).rgb;
    vec3 k = texture(uSource, uv + uTexel * vec2(1.0, 1.0)).rgb;
    vec3 l = texture(uSource, uv + uTexel * vec2(-1.0, -1.0)).rgb;
    vec3 m = texture(uSource, uv + uTexel * vec2(1.0, -1.0)).rgb;
    vec3 sum = e * 0.125;
    sum += (a + c + g + i) * 0.03125;
    sum += (b + d + f + h) * 0.0625;
    sum += (j + k + l + m) * 0.125;
    outColor = vec4(sum, 1.0);
}
`;

/**
 * One step back up: a tent filter, added into the level above.
 *
 * The sum of the levels is what gives the spread its long tail - each is twice
 * as wide and carries less, which is far closer to a real point spread than
 * any single blur of one width.
 */
export const BLOOM_UP_FRAG_SRC = `#version 300 es
precision highp float;
in vec2 vNdc;
uniform sampler2D uSource;
uniform vec2 uTexel;
out vec4 outColor;

void main() {
    vec2 uv = vNdc * 0.5 + 0.5;
    vec3 sum = texture(uSource, uv + uTexel * vec2(-1.0, 1.0)).rgb;
    sum += texture(uSource, uv + uTexel * vec2(0.0, 1.0)).rgb * 2.0;
    sum += texture(uSource, uv + uTexel * vec2(1.0, 1.0)).rgb;
    sum += texture(uSource, uv + uTexel * vec2(-1.0, 0.0)).rgb * 2.0;
    sum += texture(uSource, uv).rgb * 4.0;
    sum += texture(uSource, uv + uTexel * vec2(1.0, 0.0)).rgb * 2.0;
    sum += texture(uSource, uv + uTexel * vec2(-1.0, -1.0)).rgb;
    sum += texture(uSource, uv + uTexel * vec2(0.0, -1.0)).rgb * 2.0;
    sum += texture(uSource, uv + uTexel * vec2(1.0, -1.0)).rgb;
    outColor = vec4(sum / 16.0, 1.0);
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

void main() {
    vec4 view = uInverseProjection * vec4(vNdc, 1.0, 1.0);
    view /= view.w;
    vec3 direction = normalize((uInverseView * vec4(view.xyz, 0.0)).xyz);
    outColor = vec4(textureLod(uEnvironment, direction, 0.0).rgb, 1.0);
}
`;
