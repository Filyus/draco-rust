import type { Vec3 } from '../math.ts';
import type { ViewerScene } from '../viewer-scene.ts';

/**
 * Orbit camera state and the operations that move it.
 *
 * Free of WebGL: these only read the canvas rect and the scene bounds, and
 * write the camera angles, target and dolly limits. The viewer keeps the state
 * and the scratch vectors so its existing members stay where callers expect
 * them; everything that decides how the camera moves lives here.
 */

/**
 * What camera code reads and writes on the viewer.
 *
 * The state lives on the Viewer itself so its existing members stay where the
 * browser tests reach for them; this names exactly which of them are camera
 * business.
 */
export interface CameraHost {
  canvas: HTMLCanvasElement;
  scene: ViewerScene | null;
  camera: OrbitCamera;
  _basisRight: Vec3;
  _basisUp: Vec3;
  _basisForward: Vec3;
  _pivotScratch: Vec3;
  _navKeys: Set<string>;
  _navFast: boolean;
  _navSlow: boolean;
  /** Schedules a frame; every operation below moves what is on screen. */
  invalidate(): void;
}

/** Orbit camera: an eye on a sphere around `target`. */
export interface OrbitCamera {
  target: Vec3;
  distance: number;
  azimuth: number;
  elevation: number;
  fov: number;
  near: number;
  far: number;
  minDistance: number;
  maxDistance: number;
}

/** Starting framing, chosen to show a model's front-left three-quarter view. */
export const DEFAULT_CAMERA_AZIMUTH = Math.PI * 0.25;
export const DEFAULT_CAMERA_ELEVATION = Math.PI * 0.09;
export const ORBIT_RAD_PER_PIXEL = 0.01;
// Movement keys cross this fraction of the orbit distance per second, so the
// same tap feels alike on a small prop and on a whole level.
export const FLY_DISTANCE_PER_SECOND = 0.4;
export const ORBIT_RAD_PER_SECOND = 1.2;

export function orbitBy(host: CameraHost, dAz: number, dEl: number) {
  // Invalidated up front rather than at the end: the paths below have early
  // returns that leave the camera already moved.
  host.invalidate();
  const right = host._basisRight;
  const up = host._basisUp;
  const forward = host._basisForward;
  const pivot = orbitPivot(host, host._pivotScratch);

  let a = 0, b = 0, c = 0;
  if (pivot) {
    cameraBasis(host, right, up, forward);
    for (let i = 0; i < 3; i++) {
      const d = host.camera.target[i] - pivot[i];
      a += d * right[i];
      b += d * up[i];
      c += d * forward[i];
    }
  }

  host.camera.azimuth -= dAz;
  host.camera.elevation += dEl;
  host.camera.elevation = Math.max(
    -Math.PI * 0.495,
    Math.min(Math.PI * 0.495, host.camera.elevation),
  );

  if (!pivot) return;
  // Rebuilding the same camera-space offset in the turned basis rotates
  // the target around the pivot exactly as far as the eye turned.
  cameraBasis(host, right, up, forward);
  for (let i = 0; i < 3; i++) {
    host.camera.target[i] = pivot[i] + right[i] * a + up[i] * b + forward[i] * c;
  }
}

/** World-space centre of the loaded scene, or null when nothing is loaded. */
export function orbitPivot(host: CameraHost, out: Vec3): Vec3 | null {
  const box = host.scene?.aabb;
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
export function zoomBy(host: CameraHost, factor: number, clientX?: number, clientY?: number) {
  host.invalidate();
  const before = host.camera.distance;
  host.camera.distance = Math.max(
    host.camera.minDistance,
    Math.min(host.camera.maxDistance, before * factor),
  );
  // Callers pass both cursor coordinates or neither.
  if (clientX === undefined || clientY === undefined) return;

  // The clamp may have swallowed part of the requested dolly.
  const applied = host.camera.distance / before;
  const rect = host.canvas.getBoundingClientRect();
  if (!rect.width || !rect.height) return;
  const right = host._basisRight;
  const up = host._basisUp;
  cameraBasis(host, right, up);
  // Offset of the cursor from the view centre, in world units on the
  // plane through the target.
  const k = (2 * before * Math.tan(host.camera.fov * 0.5)) / rect.height;
  const ax = (clientX - (rect.left + rect.width * 0.5)) * k;
  const ay = -(clientY - (rect.top + rect.height * 0.5)) * k;
  const shift = 1 - applied;
  for (let i = 0; i < 3; i++) {
    host.camera.target[i] += (right[i] * ax + up[i] * ay) * shift;
  }
}

/**
 * Slides the orbit target inside the camera plane so the point under the
 * cursor stays under the cursor, for any orbit angle and field of view.
 */
export function panBy(host: CameraHost, dx: number, dy: number) {
  host.invalidate();
  const right = host._basisRight;
  const up = host._basisUp;
  cameraBasis(host, right, up);
  const height = host.canvas.clientHeight || host.canvas.height;
  const k = (2 * host.camera.distance * Math.tan(host.camera.fov * 0.5))
    / Math.max(1, height);
  for (let i = 0; i < 3; i++) {
    host.camera.target[i] += (up[i] * dy - right[i] * dx) * k;
  }
}

/**
 * Applies the held navigation keys for one frame: WASD and Q/E move the
 * orbit target along the camera axes, arrows orbit.
 */
export function applyKeyboardNavigation(host: CameraHost, dt: number) {
  const keys = host._navKeys;
  if (keys.size === 0) return;

  let scale = 1;
  if (host._navFast) scale *= 4;
  if (host._navSlow) scale *= 0.25;

  const orbitStep = ORBIT_RAD_PER_SECOND * dt * scale;
  let dAz = 0, dEl = 0;
  if (keys.has('ArrowLeft')) dAz -= 1;
  if (keys.has('ArrowRight')) dAz += 1;
  if (keys.has('ArrowUp')) dEl += 1;
  if (keys.has('ArrowDown')) dEl -= 1;
  if (dAz || dEl) orbitBy(host, dAz * orbitStep, dEl * orbitStep);

  let fwd = 0, side = 0, lift = 0;
  if (keys.has('KeyW')) fwd += 1;
  if (keys.has('KeyS')) fwd -= 1;
  if (keys.has('KeyD')) side += 1;
  if (keys.has('KeyA')) side -= 1;
  if (keys.has('KeyE')) lift += 1;
  if (keys.has('KeyQ')) lift -= 1;
  if (!fwd && !side && !lift) return;

  const speed = FLY_DISTANCE_PER_SECOND * host.camera.distance * dt * scale;
  const right = host._basisRight;
  const up = host._basisUp;
  const forward = host._basisForward;
  cameraBasis(host, right, up, forward);
  for (let i = 0; i < 3; i++) {
    host.camera.target[i] +=
      (forward[i] * fwd + right[i] * side + up[i] * lift) * speed;
  }
  host.invalidate();
}

export function fitCameraToScene(host: CameraHost) {
  host.invalidate();
  const box = host.scene?.aabb;
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

  host.camera.target[0] = cx;
  host.camera.target[1] = cy;
  host.camera.target[2] = cz;
  const verticalFov = host.camera.fov;
  const aspect = host.canvas.width / Math.max(1, host.canvas.height);
  const horizontalFov = 2 * Math.atan(Math.tan(verticalFov * 0.5) * aspect);
  const fitFov = Math.min(verticalFov, horizontalFov);
  host.camera.distance = Math.max(0.5, (safeRadius / Math.sin(fitFov * 0.5)) * 1.12);
  const diameter = Math.max(0.001, safeRadius * 2);
  host.camera.near = Math.max(0.001, diameter * 0.001);
  host.camera.far = diameter * 1000 + host.camera.distance * 2;
  // Fixed limits would clamp a large asset below its own fit distance,
  // so one wheel notch would snap the camera inside the model.
  host.camera.minDistance = Math.max(0.001, host.camera.near * 2);
  host.camera.maxDistance = Math.max(host.camera.distance, safeRadius) * 100;
}

/**
 * Right and up axes of the camera plane, from the same angles
 * `_cameraPosition` uses: right = normalize(forward x worldUp),
 * up = right x forward. cos(elevation) stays positive under the clamp.
 */
export function cameraBasis(
  host: CameraHost,
  right: Vec3,
  up: Vec3,
  forward?: Vec3,
) {
  const ce = Math.cos(host.camera.elevation);
  const se = Math.sin(host.camera.elevation);
  const ca = Math.cos(host.camera.azimuth);
  const sa = Math.sin(host.camera.azimuth);
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

export function cameraPosition(host: CameraHost, out: Vec3): Vec3 {
  const ce = Math.cos(host.camera.elevation);
  const se = Math.sin(host.camera.elevation);
  const ca = Math.cos(host.camera.azimuth);
  const sa = Math.sin(host.camera.azimuth);
  const r = host.camera.distance;
  out[0] = host.camera.target[0] + r * ce * sa;
  out[1] = host.camera.target[1] + r * se;
  out[2] = host.camera.target[2] + r * ce * ca;
  return out;
}
