/**
 * Vanilla WebGL2 3D preview viewer.
 *
 * Renders the format-agnostic Scene produced by gltf-loader / mesh-loader.
 * Supports TRS + skinned animation, base color materials (texture + factor +
 * vertex colors), and orbit/touch camera controls. No external dependencies.
 *
 * The engine itself lives in ./viewer/: shader sources, GL helpers, CPU-side
 * attribute preparation, morph and primitive upload, animation sampling, and
 * the Viewer class that drives them. This module stays the single public entry
 * point, so `www/viewer.js` keeps the exports its browser and Node callers
 * already import.
 */

export { Viewer } from './viewer/viewer-class.ts';
export { buildNormalizedWeightAttribute, buildSmoothNormalAttribute } from './viewer/geometry.ts';
export { cubicSplineInterpolate } from './viewer/animation.ts';
