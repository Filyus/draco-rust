/**
 * Engine limits and interaction defaults.
 *
 * These are policy, not hardware ceilings: the morph bound is a shader loop
 * length, and the camera values only pick a starting framing.
 */

export const MAX_JOINTS = 256;
// Morph targets blended in one draw. Deltas are sampled from an array texture,
// so this is a shader loop bound rather than a hardware limit; a mesh may
// declare any number of targets and the strongest-weighted ones are blended.
export const MAX_ACTIVE_MORPH_TARGETS = 32;
// Texture unit for the morph delta array. Units 0..4 are material maps and
// 5..8 belong to the environment IBL.
export const MORPH_TEXTURE_UNIT = 9;
// How far a vertex's skin weights may sum from one before the preview rebuilds
// the attribute. Loose enough to pass ordinary quantization rounding, tight
// enough to catch a vertex that would otherwise be dragged toward the origin.
export const WEIGHT_SUM_TOLERANCE = 1e-3;
export const DEFAULT_CAMERA_AZIMUTH = Math.PI * 0.25;
export const DEFAULT_CAMERA_ELEVATION = Math.PI * 0.09;
export const ORBIT_RAD_PER_PIXEL = 0.01;
// Movement keys cross this fraction of the orbit distance per second, so the
// same tap feels alike on a small prop and on a whole level.
export const FLY_DISTANCE_PER_SECOND = 0.4;
export const ORBIT_RAD_PER_SECOND = 1.2;
// Keys the viewport claims while focused, so they never scroll the page.
export const NAV_KEYS = new Set([
    'KeyW', 'KeyA', 'KeyS', 'KeyD', 'KeyQ', 'KeyE',
    'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight',
]);
