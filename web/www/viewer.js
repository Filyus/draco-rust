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
layout(location=3) in vec4 aColor;
layout(location=4) in vec4 aJoints;
layout(location=5) in vec4 aWeights;

uniform mat4 uProjection;
uniform mat4 uView;
uniform mat4 uModel;
uniform mat4 uNormalMatrix;
uniform int uUseSkin;
uniform int uJointCount;
uniform mat4 uJointMatrix[${MAX_JOINTS}];

out vec3 vNormal;
out vec2 vTexCoord;
out vec4 vColor;
out vec3 vWorldPos;

void main() {
    vec4 skinned = vec4(0.0);
    if (uUseSkin == 1 && uJointCount > 0) {
        vec4 pos = vec4(aPosition, 1.0);
        skinned +=
            (uJointMatrix[int(aJoints.x)] * pos) * aWeights.x +
            (uJointMatrix[int(aJoints.y)] * pos) * aWeights.y +
            (uJointMatrix[int(aJoints.z)] * pos) * aWeights.z +
            (uJointMatrix[int(aJoints.w)] * pos) * aWeights.w;

        vec4 nrm = vec4(aNormal, 0.0);
        vec3 skinnedNormal =
            (uJointMatrix[int(aJoints.x)] * nrm).xyz * aWeights.x +
            (uJointMatrix[int(aJoints.y)] * nrm).xyz * aWeights.y +
            (uJointMatrix[int(aJoints.z)] * nrm).xyz * aWeights.z +
            (uJointMatrix[int(aJoints.w)] * nrm).xyz * aWeights.w;
        vNormal = normalize((uNormalMatrix * vec4(skinnedNormal, 0.0)).xyz);
    } else {
        skinned = vec4(aPosition, 1.0);
        vNormal = normalize((uNormalMatrix * vec4(aNormal, 0.0)).xyz);
    }

    vec4 worldPos = uModel * skinned;
    vWorldPos = worldPos.xyz;
    vTexCoord = aTexCoord;
    vColor = aColor;
    gl_Position = uProjection * uView * worldPos;
}
`;

const FRAG_SRC = `#version 300 es
precision highp float;

in vec3 vNormal;
in vec2 vTexCoord;
in vec4 vColor;
in vec3 vWorldPos;

uniform int uHasTexture;
uniform int uHasNormals;
uniform int uHasVertexColors;
uniform int uUnlit;
uniform sampler2D uBaseColor;
uniform vec4 uBaseColorFactor;

uniform vec3 uLightDir;        // direction TO light (key)
uniform vec3 uLightColor;
uniform vec3 uFillDir;         // direction TO light (fill, softer)
uniform vec3 uFillColor;
uniform vec3 uAmbient;
uniform vec3 uCameraPos;

out vec4 outColor;

void main() {
    vec4 base = uBaseColorFactor;
    if (uHasVertexColors == 1) base *= vColor;
    if (uHasTexture == 1) base *= texture(uBaseColor, vTexCoord);

    // Hard unlit materials (KHR_materials_unlit) keep flat shading.
    if (uUnlit == 1) {
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

    vec3 L = normalize(uLightDir);
    float diff = max(dot(N, L), 0.0);

    vec3 F = normalize(uFillDir);
    float fill = max(dot(N, F), 0.0) * 0.5;

    // Hemisphere fill so back faces are never pure black.
    float hemi = 0.5 + 0.5 * N.y;
    vec3 ambient = uAmbient * (0.7 + 0.5 * hemi);

    vec3 V = normalize(uCameraPos - vWorldPos);
    vec3 H = normalize(L + V);
    float spec = pow(max(dot(N, H), 0.0), 48.0) * 0.3 * float(uHasNormals);

    vec3 color = base.rgb * (ambient + uLightColor * diff + uFillColor * fill)
                 + uLightColor * spec;
    outColor = vec4(color, base.a);
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

    function bindAttribute(semantic, location, desiredComponents) {
        const attr = primitive.attributes[semantic];
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

    const layout = {
        position: locationMap.position,
        normal: locationMap.normal,
        texCoord: locationMap.texCoord,
        color: locationMap.color,
        joints: locationMap.joints,
        weights: locationMap.weights,
    };

    const info = {
        vao,
        buffers,
        hasNormals: !!bindAttribute('NORMAL', layout.normal),
        hasTexCoords: !!bindAttribute('TEXCOORD_0', layout.texCoord),
        hasColors: !!bindAttribute('COLOR_0', layout.color),
        hasJoints: !!bindAttribute('JOINTS_0', layout.joints),
        hasWeights: !!bindAttribute('WEIGHTS_0', layout.weights),
        mode: primitive.mode,
        elementCount: 0,
        indexed: false,
    };

    bindAttribute('POSITION', layout.position);

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
            alpha: false,
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
        this.autoRotate = false;

        // Matrices
        this._projection = mat4.create();
        this._view = mat4.create();
        this._projectionView = mat4.create();
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
        gl.clearColor(0.058, 0.09, 0.164, 1.0); // matches app --bg-dark
    }

    _buildPrograms() {
        this.program = linkProgram(this.gl, VERT_SRC, FRAG_SRC);
        this.lineProgram = linkProgram(this.gl, LINE_VERT_SRC, LINE_FRAG_SRC);

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
            uHasTexture: gl.getUniformLocation(p, 'uHasTexture'),
            uHasNormals: gl.getUniformLocation(p, 'uHasNormals'),
            uHasVertexColors: gl.getUniformLocation(p, 'uHasVertexColors'),
            uUnlit: gl.getUniformLocation(p, 'uUnlit'),
            uBaseColor: gl.getUniformLocation(p, 'uBaseColor'),
            uBaseColorFactor: gl.getUniformLocation(p, 'uBaseColorFactor'),
            uLightDir: gl.getUniformLocation(p, 'uLightDir'),
            uLightColor: gl.getUniformLocation(p, 'uLightColor'),
            uFillDir: gl.getUniformLocation(p, 'uFillDir'),
            uFillColor: gl.getUniformLocation(p, 'uFillColor'),
            uAmbient: gl.getUniformLocation(p, 'uAmbient'),
            uCameraPos: gl.getUniformLocation(p, 'uCameraPos'),
        };
        this.locations = {
            position: 0,
            normal: 1,
            texCoord: 2,
            color: 3,
            joints: 4,
            weights: 5,
        };
        this.lineUniforms = {
            uProjectionView: gl.getUniformLocation(this.lineProgram, 'uProjectionView'),
            uColor: gl.getUniformLocation(this.lineProgram, 'uColor'),
        };
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

        // Grid (drawn first, depth-disabled so it sits behind everything)
        if (this.showGrid) this._drawGrid();

        gl.useProgram(this.program);
        gl.uniformMatrix4fv(this.uniforms.uProjection, false, this._projection);
        gl.uniformMatrix4fv(this.uniforms.uView, false, this._view);
        gl.uniform3fv(this.uniforms.uLightDir, this._lightDir || (this._lightDir = new Float32Array([0.5, 0.8, 0.6])));
        gl.uniform3fv(this.uniforms.uLightColor, this._lightColor || (this._lightColor = new Float32Array([1.0, 0.97, 0.9])));
        gl.uniform3fv(this.uniforms.uFillDir, this._fillDir || (this._fillDir = new Float32Array([-0.6, 0.3, -0.4])));
        gl.uniform3fv(this.uniforms.uFillColor, this._fillColor || (this._fillColor = new Float32Array([0.4, 0.45, 0.6])));
        gl.uniform3fv(this.uniforms.uAmbient, this._ambient || (this._ambient = new Float32Array([0.45, 0.47, 0.55])));
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
        const hasTexture = !!material?.baseColorTexture;
        gl.uniform1i(this.uniforms.uHasTexture, hasTexture ? 1 : 0);
        gl.uniform1i(this.uniforms.uHasNormals, uploaded.hasNormals ? 1 : 0);
        gl.uniform1i(this.uniforms.uHasVertexColors, uploaded.hasColors ? 1 : 0);
        gl.uniform1i(this.uniforms.uUnlit, material?.unlit ? 1 : 0);

        if (hasTexture) {
            const tex = this.glResources.textures[material.baseColorTexture];
            gl.activeTexture(gl.TEXTURE0);
            gl.bindTexture(gl.TEXTURE_2D, tex);
            gl.uniform1i(this.uniforms.uBaseColor, 0);
        }
        const factor = material?.baseColorFactor || [1, 1, 1, 1];
        gl.uniform4f(this.uniforms.uBaseColorFactor, factor[0], factor[1], factor[2], factor[3]);
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
        gl.uniform3f(this.lineUniforms.uColor, 0.25, 0.3, 0.4);
        gl.bindBuffer(gl.ARRAY_BUFFER, this._grid.buffer);
        gl.enableVertexAttribArray(0);
        gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
        gl.drawArrays(gl.LINES, 0, this._grid.count);
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
        applyChannel(channel.node, path, interpolation, output, i0, frac);
    }
}

function applyChannel(node, path, interpolation, output, i0, frac) {
    // A node animated through TRS must no longer use a static matrix. Such an
    // asset is invalid in strict glTF, but this gives the preview a sensible
    // result for permissively-authored files.
    node.localMatrix = null;
    const components = path === 'rotation' ? 4 : 3;
    const stride = interpolation === 'CUBICSPLINE' ? components * 3 : components;
    const base0 = i0 * stride;
    const base1 = Math.min(i0 + 1, output.length / stride - 1) * stride;
    const out = node.trs[path];

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
