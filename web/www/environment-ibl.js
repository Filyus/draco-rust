/**
 * Procedural HDR environment and the preintegrated textures used by glTF PBR.
 *
 * The visible cubemap is the only source of radiance. Diffuse irradiance is a
 * cosine convolution of it; specular mip levels are GGX convolutions; and the
 * BRDF LUT stores the split-sum visibility/Fresnel integral.
 */

const FULLSCREEN_VERTEX = `#version 300 es
precision highp float;
out vec2 vUv;
void main() {
    vec2 positions[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    vUv = positions[gl_VertexID];
    gl_Position = vec4(vUv, 0.0, 1.0);
}
`;

const CUBE_DIRECTION = `
vec3 cubeDirection(int face, vec2 uv) {
    if (face == 0) return normalize(vec3( 1.0, -uv.y, -uv.x));
    if (face == 1) return normalize(vec3(-1.0, -uv.y,  uv.x));
    if (face == 2) return normalize(vec3( uv.x,  1.0,  uv.y));
    if (face == 3) return normalize(vec3( uv.x, -1.0, -uv.y));
    if (face == 4) return normalize(vec3( uv.x, -uv.y,  1.0));
    return normalize(vec3(-uv.x, -uv.y, -1.0));
}
`;

const SAMPLE_SEQUENCE = `
const float PI = 3.14159265359;

float radicalInverse(uint bits) {
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return float(bits) * 2.3283064365386963e-10;
}

vec2 hammersley(uint i, uint count) {
    return vec2(float(i) / float(count), radicalInverse(i));
}

mat3 tangentBasis(vec3 normal) {
    vec3 up = abs(normal.y) < 0.999 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
    vec3 tangent = normalize(cross(up, normal));
    return mat3(tangent, cross(normal, tangent), normal);
}
`;

const ENVIRONMENT_FRAGMENT = `#version 300 es
precision highp float;
in vec2 vUv;
uniform int uFace;
out vec4 outColor;
${CUBE_DIRECTION}

vec3 environmentRadiance(vec3 direction) {
    float sky = smoothstep(-0.025, 0.045, direction.y);
    float height = pow(max(direction.y, 0.0), 0.45);
    vec3 floorRadiance = vec3(0.035, 0.040, 0.050);
    vec3 skyRadiance = mix(vec3(0.16, 0.21, 0.31), vec3(0.38, 0.52, 0.78), height);
    vec3 radiance = mix(floorRadiance, skyRadiance, sky);

    // Finite-area studio emitters represented directly in the radiance field.
    vec3 key = normalize(vec3(-0.45, 0.78, 0.42));
    vec3 rim = normalize(vec3(0.52, 0.42, -0.72));
    radiance += vec3(7.0, 6.7, 6.2) * pow(max(dot(direction, key), 0.0), 72.0);
    radiance += vec3(1.8, 2.2, 3.2) * pow(max(dot(direction, rim), 0.0), 44.0);
    return radiance;
}

void main() {
    outColor = vec4(environmentRadiance(cubeDirection(uFace, vUv)), 1.0);
}
`;

const IRRADIANCE_FRAGMENT = `#version 300 es
precision highp float;
in vec2 vUv;
uniform int uFace;
uniform samplerCube uEnvironment;
out vec4 outColor;
${CUBE_DIRECTION}
${SAMPLE_SEQUENCE}

void main() {
    vec3 normal = cubeDirection(uFace, vUv);
    mat3 basis = tangentBasis(normal);
    vec3 sum = vec3(0.0);
    const uint SAMPLE_COUNT = 64u;
    for (uint i = 0u; i < SAMPLE_COUNT; ++i) {
        vec2 xi = hammersley(i, SAMPLE_COUNT);
        float phi = 2.0 * PI * xi.x;
        float sinTheta = sqrt(xi.y);
        float cosTheta = sqrt(1.0 - xi.y);
        vec3 local = vec3(cos(phi) * sinTheta, sin(phi) * sinTheta, cosTheta);
        sum += textureLod(uEnvironment, basis * local, 0.0).rgb;
    }
    // Cosine-weighted sampling has pdf = cos(theta) / PI.
    outColor = vec4(PI * sum / float(SAMPLE_COUNT), 1.0);
}
`;

const PREFILTER_FRAGMENT = `#version 300 es
precision highp float;
in vec2 vUv;
uniform int uFace;
uniform float uRoughness;
uniform samplerCube uEnvironment;
out vec4 outColor;
${CUBE_DIRECTION}
${SAMPLE_SEQUENCE}

vec3 importanceSampleGgx(vec2 xi, vec3 normal, float roughness) {
    float alpha = roughness * roughness;
    float alpha2 = alpha * alpha;
    float phi = 2.0 * PI * xi.x;
    float cosTheta = sqrt((1.0 - xi.y) / (1.0 + (alpha2 - 1.0) * xi.y));
    float sinTheta = sqrt(max(0.0, 1.0 - cosTheta * cosTheta));
    vec3 halfVector = vec3(cos(phi) * sinTheta, sin(phi) * sinTheta, cosTheta);
    return normalize(tangentBasis(normal) * halfVector);
}

void main() {
    vec3 normal = cubeDirection(uFace, vUv);
    if (uRoughness < 0.001) {
        outColor = vec4(textureLod(uEnvironment, normal, 0.0).rgb, 1.0);
        return;
    }
    vec3 view = normal;
    vec3 sum = vec3(0.0);
    float weight = 0.0;
    const uint SAMPLE_COUNT = 64u;
    for (uint i = 0u; i < SAMPLE_COUNT; ++i) {
        vec3 halfVector = importanceSampleGgx(hammersley(i, SAMPLE_COUNT), normal, uRoughness);
        vec3 light = normalize(2.0 * dot(view, halfVector) * halfVector - view);
        float nDotL = max(dot(normal, light), 0.0);
        if (nDotL > 0.0) {
            sum += textureLod(uEnvironment, light, 0.0).rgb * nDotL;
            weight += nDotL;
        }
    }
    outColor = vec4(sum / max(weight, 0.0001), 1.0);
}
`;

const BRDF_FRAGMENT = `#version 300 es
precision highp float;
in vec2 vUv;
out vec4 outColor;
${SAMPLE_SEQUENCE}

vec3 importanceSampleGgx(vec2 xi, float roughness) {
    float alpha = roughness * roughness;
    float alpha2 = alpha * alpha;
    float phi = 2.0 * PI * xi.x;
    float cosTheta = sqrt((1.0 - xi.y) / (1.0 + (alpha2 - 1.0) * xi.y));
    float sinTheta = sqrt(max(0.0, 1.0 - cosTheta * cosTheta));
    return vec3(cos(phi) * sinTheta, sin(phi) * sinTheta, cosTheta);
}

float geometrySchlickGgx(float nDotV, float roughness) {
    float k = roughness * roughness * 0.5;
    return nDotV / (nDotV * (1.0 - k) + k);
}

float geometrySmith(float nDotV, float nDotL, float roughness) {
    return geometrySchlickGgx(nDotV, roughness) * geometrySchlickGgx(nDotL, roughness);
}

void main() {
    float nDotV = clamp(vUv.x * 0.5 + 0.5, 0.001, 0.999);
    float roughness = clamp(vUv.y * 0.5 + 0.5, 0.001, 1.0);
    vec3 view = vec3(sqrt(1.0 - nDotV * nDotV), 0.0, nDotV);
    float scale = 0.0;
    float bias = 0.0;
    const uint SAMPLE_COUNT = 128u;
    for (uint i = 0u; i < SAMPLE_COUNT; ++i) {
        vec3 halfVector = importanceSampleGgx(hammersley(i, SAMPLE_COUNT), roughness);
        vec3 light = normalize(2.0 * dot(view, halfVector) * halfVector - view);
        float nDotL = max(light.z, 0.0);
        float nDotH = max(halfVector.z, 0.0);
        float vDotH = max(dot(view, halfVector), 0.0);
        if (nDotL > 0.0) {
            float visibility = geometrySmith(nDotV, nDotL, roughness)
                * vDotH / max(nDotH * nDotV, 0.0001);
            float fresnel = pow(1.0 - vDotH, 5.0);
            scale += (1.0 - fresnel) * visibility;
            bias += fresnel * visibility;
        }
    }
    outColor = vec4(scale / float(SAMPLE_COUNT), bias / float(SAMPLE_COUNT), 0.0, 1.0);
}
`;

function compile(gl, type, source) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        const message = gl.getShaderInfoLog(shader);
        gl.deleteShader(shader);
        throw new Error(`environment shader compile error: ${message}`);
    }
    return shader;
}

function program(gl, fragment) {
    const result = gl.createProgram();
    const vertex = compile(gl, gl.VERTEX_SHADER, FULLSCREEN_VERTEX);
    const pixel = compile(gl, gl.FRAGMENT_SHADER, fragment);
    gl.attachShader(result, vertex);
    gl.attachShader(result, pixel);
    gl.linkProgram(result);
    gl.deleteShader(vertex);
    gl.deleteShader(pixel);
    if (!gl.getProgramParameter(result, gl.LINK_STATUS)) {
        const message = gl.getProgramInfoLog(result);
        gl.deleteProgram(result);
        throw new Error(`environment program link error: ${message}`);
    }
    return result;
}

function cubeTexture(gl, size, levels, format) {
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_CUBE_MAP, texture);
    for (let level = 0; level < levels; level++) {
        const dimension = Math.max(1, size >> level);
        for (let face = 0; face < 6; face++) {
            gl.texImage2D(
                gl.TEXTURE_CUBE_MAP_POSITIVE_X + face,
                level,
                format.internal,
                dimension,
                dimension,
                0,
                gl.RGBA,
                format.type,
                null,
            );
        }
    }
    gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_WRAP_R, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(
        gl.TEXTURE_CUBE_MAP,
        gl.TEXTURE_MIN_FILTER,
        levels > 1 ? gl.LINEAR_MIPMAP_LINEAR : gl.LINEAR,
    );
    gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_MAX_LEVEL, levels - 1);
    return texture;
}

function renderCube(gl, framebuffer, vao, target, size, levels, renderLevel) {
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.bindVertexArray(vao);
    for (let level = 0; level < levels; level++) {
        const dimension = Math.max(1, size >> level);
        gl.viewport(0, 0, dimension, dimension);
        renderLevel(level, levels);
        for (let face = 0; face < 6; face++) {
            gl.framebufferTexture2D(
                gl.FRAMEBUFFER,
                gl.COLOR_ATTACHMENT0,
                gl.TEXTURE_CUBE_MAP_POSITIVE_X + face,
                target,
                level,
            );
            if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
                throw new Error('environment cubemap framebuffer is incomplete');
            }
            renderLevel(level, levels, face);
            gl.drawArrays(gl.TRIANGLES, 0, 3);
        }
    }
}

export function createEnvironmentIbl(gl, onLog = () => {}) {
    const hdr = !!gl.getExtension('EXT_color_buffer_float');
    if (!hdr) onLog('Float render targets unavailable; environment IBL uses LDR precision', 'warning');
    const format = hdr
        ? { internal: gl.RGBA16F, type: gl.HALF_FLOAT }
        : { internal: gl.RGBA8, type: gl.UNSIGNED_BYTE };
    const resources = { textures: [], programs: [] };

    try {
        const framebuffer = gl.createFramebuffer();
        const vao = gl.createVertexArray();
        resources.framebuffer = framebuffer;
        resources.vao = vao;

        const environment = cubeTexture(gl, 128, 1, format);
        const irradiance = cubeTexture(gl, 32, 1, format);
        const prefilteredLevels = 8;
        const prefiltered = cubeTexture(gl, 128, prefilteredLevels, format);
        resources.textures.push(environment, irradiance, prefiltered);

        const environmentProgram = program(gl, ENVIRONMENT_FRAGMENT);
        const irradianceProgram = program(gl, IRRADIANCE_FRAGMENT);
        const prefilterProgram = program(gl, PREFILTER_FRAGMENT);
        const brdfProgram = program(gl, BRDF_FRAGMENT);
        resources.programs.push(environmentProgram, irradianceProgram, prefilterProgram, brdfProgram);

        gl.disable(gl.DEPTH_TEST);
        gl.depthMask(false);
        renderCube(gl, framebuffer, vao, environment, 128, 1, (_level, _levels, face) => {
            gl.useProgram(environmentProgram);
            if (face !== undefined) gl.uniform1i(gl.getUniformLocation(environmentProgram, 'uFace'), face);
        });

        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_CUBE_MAP, environment);
        gl.useProgram(irradianceProgram);
        gl.uniform1i(gl.getUniformLocation(irradianceProgram, 'uEnvironment'), 0);
        renderCube(gl, framebuffer, vao, irradiance, 32, 1, (_level, _levels, face) => {
            gl.useProgram(irradianceProgram);
            if (face !== undefined) gl.uniform1i(gl.getUniformLocation(irradianceProgram, 'uFace'), face);
        });

        gl.useProgram(prefilterProgram);
        gl.uniform1i(gl.getUniformLocation(prefilterProgram, 'uEnvironment'), 0);
        renderCube(gl, framebuffer, vao, prefiltered, 128, prefilteredLevels, (level, levels, face) => {
            gl.useProgram(prefilterProgram);
            gl.uniform1f(gl.getUniformLocation(prefilterProgram, 'uRoughness'), level / (levels - 1));
            if (face !== undefined) gl.uniform1i(gl.getUniformLocation(prefilterProgram, 'uFace'), face);
        });

        const brdfLut = gl.createTexture();
        resources.textures.push(brdfLut);
        gl.bindTexture(gl.TEXTURE_2D, brdfLut);
        gl.texImage2D(gl.TEXTURE_2D, 0, format.internal, 128, 128, 0, gl.RGBA, format.type, null);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
        gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, brdfLut, 0);
        if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
            throw new Error('environment BRDF framebuffer is incomplete');
        }
        gl.viewport(0, 0, 128, 128);
        gl.useProgram(brdfProgram);
        gl.bindVertexArray(vao);
        gl.drawArrays(gl.TRIANGLES, 0, 3);

        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.bindVertexArray(null);
        gl.depthMask(true);
        gl.enable(gl.DEPTH_TEST);
        return {
            environment,
            irradiance,
            prefiltered,
            brdfLut,
            maxLod: prefilteredLevels - 1,
            hdr,
            dispose() {
                for (const texture of resources.textures) gl.deleteTexture(texture);
                for (const shaderProgram of resources.programs) gl.deleteProgram(shaderProgram);
                gl.deleteFramebuffer(framebuffer);
                gl.deleteVertexArray(vao);
            },
        };
    } catch (error) {
        for (const texture of resources.textures) gl.deleteTexture(texture);
        for (const shaderProgram of resources.programs) gl.deleteProgram(shaderProgram);
        if (resources.framebuffer) gl.deleteFramebuffer(resources.framebuffer);
        if (resources.vao) gl.deleteVertexArray(resources.vao);
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
        gl.bindVertexArray(null);
        gl.depthMask(true);
        gl.enable(gl.DEPTH_TEST);
        throw error;
    }
}
