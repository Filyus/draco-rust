import { mat4, vec3 } from '../math.ts';
import type { Mat4, Vec3 } from '../math.ts';
import type { ViewerNode, ViewerScene } from '../viewer-scene.ts';
import type { OrbitCamera } from './camera.ts';

/** Callbacks the embedding page supplies. */
export interface ViewerHooks {
  onSceneLoaded?(scene: ViewerScene): void;
  onError?(message: string): void;
  onLog?(message: string, type: string): void;
  onAutoRotateChange?(enabled: boolean): void;
  onAnimationEnded?(): void;
}

/** Playback state for the selected clip. */
export interface AnimationState {
  playing: boolean;
  clipIndex: number;
  time: number;
  speed: number;
  loop: boolean;
}
import { applyAnimation } from './animation.ts';

/**
 * Longest step the loop will advance in one frame. A browser tab that was in
 * the background can return with many seconds of wall clock elapsed.
 */
const MAX_FRAME_SECONDS = 1 / 15;
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
} from './camera.ts';
import { installViewerControls } from './controls.ts';
import {
  applyMaterial,
  buildSceneGrid,
  drawBackground,
  drawGrid,
  render,
  selectMorphTargets,
} from './renderer.ts';
import { setSampler, uploadImage } from './textures.ts';
import { uploadPrimitive } from './primitive-upload.ts';
import {
  computeJointMatrices,
  updateNode,
  updateSceneBounds,
  updateWorldMatrices,
} from './scene-graph.ts';
import { MAX_JOINTS } from './shaders.ts';
import { buildViewerPrograms } from './programs.ts';

export class Viewer {

  // Declared, not defined: `declare` erases completely, so the emitted class
  // keeps assigning these in the constructor exactly as before. That matters
  // for the Node tests, which drive a bare Object.create(Viewer.prototype).
  declare canvas: HTMLCanvasElement;
  declare hooks: ViewerHooks;
  declare gl: WebGL2RenderingContext;
  declare scene: ViewerScene | null;
  declare glResources: any;
  declare camera: OrbitCamera;
  declare animation: AnimationState;

  declare program: WebGLProgram;
  declare lineProgram: WebGLProgram;
  declare backgroundProgram: WebGLProgram;
  declare uniforms: Record<string, WebGLUniformLocation | null>;
  declare lineUniforms: Record<string, WebGLUniformLocation | null>;
  declare backgroundUniforms: Record<string, WebGLUniformLocation | null>;
  declare locations: Record<string, number>;
  declare backgroundVao: WebGLVertexArrayObject | null;
  declare environmentIbl: any;

  declare autoRotate: boolean;

  /**
   * Display flags are accessors so that `viewer.wireframe = true` schedules a
   * frame. Callers — the toolbar and the tests alike — keep assigning them
   * as plain fields.
   */
  declare _wireframe: boolean;
  declare _showGrid: boolean;
  declare _baseColorOnly: boolean;
  declare _smoothNormals: boolean;

  /** Set whenever something that affects the image changes. */
  declare _dirty: boolean;

  get wireframe() { return this._wireframe; }
  set wireframe(value: boolean) { this._wireframe = value; this.invalidate(); }

  get showGrid() { return this._showGrid; }
  set showGrid(value: boolean) { this._showGrid = value; this.invalidate(); }

  get baseColorOnly() { return this._baseColorOnly; }
  set baseColorOnly(value: boolean) { this._baseColorOnly = value; this.invalidate(); }

  get smoothNormals() { return this._smoothNormals; }
  set smoothNormals(value: boolean) { this._smoothNormals = value; this.invalidate(); }

  /** Schedule one frame. Cheap and idempotent; call it whenever in doubt. */
  invalidate() {
    this._dirty = true;
  }

  declare _projection: Mat4;
  declare _view: Mat4;
  declare _projectionView: Mat4;
  declare _inverseProjection: Mat4;
  declare _inverseView: Mat4;
  declare _scratch: Mat4;
  declare _normalMatrix: Mat4;
  declare _model: Mat4;
  declare _basisRight: Vec3;
  declare _basisUp: Vec3;
  declare _basisForward: Vec3;
  declare _pivotScratch: Vec3;
  declare _eye?: Vec3;
  declare _boundsPoint?: Vec3;
  declare _jointScratch?: Mat4;
  declare _visitedNodes?: Set<ViewerNode>;

  declare _navKeys: Set<string>;
  declare _navFast: boolean;
  declare _navSlow: boolean;
  declare _lastPinch: { dist: number; midX: number; midY: number } | null;

  declare _grid?: { buffer: WebGLBuffer; count: number } | null;
  declare _gridVao?: WebGLVertexArrayObject | null;
  declare _morphWeights?: Float32Array;
  declare _morphLayers?: Int32Array;
  declare _morphOrder?: number[];
  declare _emptyMorphTexture?: WebGLTexture | null;
  declare _morphPlaceholderTexture?: WebGLTexture | null;

  declare _resizeObserver: ResizeObserver;
  declare _running: boolean;
  declare _lastTime: number;

  constructor(canvas: HTMLCanvasElement, hooks: ViewerHooks = {}) {
    this.canvas = canvas;
    this.hooks = hooks; // { onSceneLoaded(scene), onError(msg), onLog(msg, type), onAutoRotateChange(enabled) }
    const gl = canvas.getContext('webgl2', {
      antialias: true,
      alpha: true,
      premultipliedAlpha: false,
      preserveDrawingBuffer: false,
    });
    if (!gl) throw new Error('WebGL2 is not supported in this browser');
    this.gl = gl;

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
      this.gl.viewport(0, 0, w, h);
      this.invalidate();
    }
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
  _orbitBy(dAz: number, dEl: number) {
    orbitBy(this, dAz, dEl);
  }

  /** World-space centre of the loaded scene, or null when nothing is loaded. */
  _orbitPivot(out: Vec3) {
    return orbitPivot(this, out);
  }

  _zoomBy(factor: number, clientX?: number, clientY?: number) {
    zoomBy(this, factor, clientX, clientY);
  }

  _panBy(dx: number, dy: number) {
    panBy(this, dx, dy);
  }

  _applyKeyboardNavigation(dt: number) {
    applyKeyboardNavigation(this, dt);
  }

  _log(msg: string, type = 'info') {
    this.hooks.onLog?.(msg, type);
  }

  setAutoRotate(enabled: boolean) {
    const next = Boolean(enabled);
    if (this.autoRotate === next) return;
    this.autoRotate = next;
    this.invalidate();
    this.hooks.onAutoRotateChange?.(next);
  }

  setScene(scene: ViewerScene | null) {
    this._disposeGlResources();
    this._disposeGrid();
    this.scene = scene;
    this.glResources = null;
    if (!scene) {
      this.hooks.onSceneLoaded?.(null as unknown as ViewerScene);
      return;
    }

    const gl = this.gl;
    const resources: {
      primitives: { uploaded: any; materialIndex: number }[][];
      textures: (WebGLTexture | null)[];
      jointMatrices: Float32Array[] | null;
    } = {
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
          this._log(`Skipped primitive: ${(error as Error).message}`, 'warning');
        }
      }
      resources.primitives.push(primitives);
    }

    // Upload textures. A document commonly points many textures at one
    // image, and uploading each separately costs a full decode-sized copy
    // plus its mip chain, so share a GL texture whenever the image and the
    // sampler state match.
    const uploaded = new Map<unknown, Map<string, WebGLTexture>>();
    for (const tex of scene.textures) {
      // Absent means "not uploaded yet"; null is a texture that has no
      // image at all. Collapsing the two skips every upload.
      let glTexture: WebGLTexture | null | undefined = null;
      if (tex && tex.image) {
        let perImage = uploaded.get(tex.image);
        if (!perImage) {
          perImage = new Map();
          uploaded.set(tex.image, perImage);
        }
        const key = `${!!tex.flipY}|${tex.wrapS}|${tex.wrapT}|${tex.minFilter}|${tex.magFilter}`;
        glTexture = perImage.get(key);
        if (glTexture === undefined) {
          glTexture = this._uploadImage(tex);
          perImage.set(key, glTexture);
        }
      }
      resources.textures.push(glTexture ?? null);
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

    this.invalidate();
    this.hooks.onSceneLoaded?.(scene);
  }

  _uploadImage(tex: any) {
    return uploadImage(this.gl, tex);
  }

  _setSampler(gl: WebGL2RenderingContext, tex: any) {
    setSampler(gl, tex);
  }

  clear() {
    this._disposeGlResources();
    this._disposeGrid();
    this.scene = null;
    this.glResources = null;
    this.animation.clipIndex = -1;
    this.animation.time = 0;
    this.animation.playing = false;
    this.invalidate();
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
    this.invalidate();
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

  _cameraBasis(right: Vec3, up: Vec3, forward?: Vec3) {
    cameraBasis(this, right, up, forward);
  }

  _cameraPosition(out: Vec3) {
    return cameraPosition(this, out);
  }

  _loop(now: number) {
    if (!this._running) return;
    // Clamped because the browser stops delivering frames to a background
    // tab: without this, coming back would jump the animation and spin the
    // camera by however many seconds elapsed. A stall simply loses time
    // rather than teleporting the scene.
    const dt = Math.min((now - this._lastTime) / 1000, MAX_FRAME_SECONDS);
    this._lastTime = now;

    // Through `_orbitBy` so auto-rotation circles the model, not the target.
    if (this.autoRotate) this._orbitBy(-dt * 0.4, 0);
    this._applyKeyboardNavigation(dt);

    if (this.animation.playing && this.scene?.animations?.length) {
      this._advanceAnimation(dt);
    }

    // Everything that changes the image marks the viewer dirty, so a still
    // scene costs one callback per frame instead of a full redraw. The
    // frame rate then follows the scene, not the display.
    if (this._dirty) {
      this._dirty = false;
      this._render();
    }
    requestAnimationFrame(this._loop);
  }

  _advanceAnimation(dt: number) {
    const clip = this.scene?.animations[this.animation.clipIndex];
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

  seekAnimation(time: number) {
    const clip = this.scene?.animations?.[this.animation.clipIndex];
    if (!clip) return false;
    this.animation.time = Math.max(0, Math.min(clip.duration, Number(time) || 0));
    applyAnimation(this.scene, this.animation.clipIndex, this.animation.time);
    this.invalidate();
    return true;
  }

  _updateWorldMatrices() {
    updateWorldMatrices(this);
  }

  /** Recompute the framing bounds after node transforms have been applied. */
  _updateSceneBounds() {
    updateSceneBounds(this);
  }

  _updateNode(node: ViewerNode, parentWorld: Mat4 | null) {
    updateNode(this, node, parentWorld);
  }

  _render() {
    render(this);
  }

  _selectMorphTargets(morph: any, weights: ArrayLike<number> | undefined) {
    return selectMorphTargets(this, morph, weights);
  }

  _applyMaterial(material: any, uploaded: any, useSmoothNormals: boolean) {
    applyMaterial(this, material, uploaded, useSmoothNormals);
  }

  _drawGrid() {
    drawGrid(this);
  }

  _drawBackground() {
    drawBackground(this);
  }

  _buildSceneGrid() {
    buildSceneGrid(this);
  }


  _computeJointMatrices(skin: any, meshWorld: Mat4, jointOut: Float32Array | null) {
    return computeJointMatrices(this, skin, meshWorld, jointOut);
  }
}
