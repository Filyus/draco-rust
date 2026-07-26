import { composeMatrix, mat4, quat, vec3 } from '../math.js';
import { applyAnimation } from './animation.js';
import {
    applyKeyboardNavigation,
    cameraBasis,
    cameraPosition,
    DEFAULT_CAMERA_AZIMUTH,
    DEFAULT_CAMERA_ELEVATION,
    fitCameraToScene,
    orbitBy,
    orbitPivot,
    panBy,
    zoomBy,
} from './camera.js';
import { installViewerControls } from './controls.js';
import { GL } from './gl-utils.js';
import { MORPH_TEXTURE_UNIT } from './morph-texture.js';
import { uploadPrimitive } from './primitive-upload.js';
import { MAX_ACTIVE_MORPH_TARGETS, MAX_JOINTS } from './shaders.js';
import { buildViewerPrograms } from './programs.js';

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
        const built = buildViewerPrograms(this.gl, (message, type) => this._log(message, type));
        this.program = built.program;
        this.lineProgram = built.lineProgram;
        this.backgroundProgram = built.backgroundProgram;
        this.uniforms = built.uniforms;
        this.locations = built.locations;
        this.lineUniforms = built.lineUniforms;
        this.backgroundUniforms = built.backgroundUniforms;
        this.backgroundVao = built.backgroundVao;
        this.environmentIbl = built.environmentIbl;
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
        installViewerControls(this);
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
        orbitBy(this, dAz, dEl);
    }

    /** World-space centre of the loaded scene, or null when nothing is loaded. */
    _orbitPivot(out) {
        return orbitPivot(this, out);
    }

    _zoomBy(factor, clientX, clientY) {
        zoomBy(this, factor, clientX, clientY);
    }

    _panBy(dx, dy) {
        panBy(this, dx, dy);
    }

    _applyKeyboardNavigation(dt) {
        applyKeyboardNavigation(this, dt);
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
                    if (uploaded.driftedWeights > 0) {
                        this._log(
                            `Mesh ${mesh.name}: skin weights on ${uploaded.driftedWeights} vertices did not sum to one and were renormalized`,
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

        // Upload textures. A document commonly points many textures at one
        // image, and uploading each separately costs a full decode-sized copy
        // plus its mip chain, so share a GL texture whenever the image and the
        // sampler state match.
        const uploaded = new Map();
        for (const tex of scene.textures) {
            let glTexture = null;
            if (tex && tex.image) {
                const perImage = uploaded.get(tex.image)
                    || uploaded.set(tex.image, new Map()).get(tex.image);
                const key = `${!!tex.flipY}|${tex.wrapS}|${tex.wrapT}|${tex.minFilter}|${tex.magFilter}`;
                glTexture = perImage.get(key);
                if (glTexture === undefined) {
                    glTexture = this._uploadImage(tex);
                    perImage.set(key, glTexture);
                }
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
        // Textures and images are both shared across slots, so delete and close
        // each distinct object once.
        for (const tex of new Set(this.glResources.textures)) {
            if (tex) gl.deleteTexture(tex);
        }
        for (const image of new Set((this.scene?.textures || []).map((tex) => tex.image))) {
            image?.close?.();
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
        fitCameraToScene(this);
    }

    _cameraBasis(right, up, forward) {
        cameraBasis(this, right, up, forward);
    }

    _cameraPosition(out) {
        return cameraPosition(this, out);
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
