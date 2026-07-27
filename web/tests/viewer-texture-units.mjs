/**
 * No two samplers may claim one texture unit.
 *
 * GL allows a unit to hold one binding per target, so a `sampler2DArray` and a
 * `sampler2D` on the same unit bind without complaint — and then the draw that
 * uses both is invalid. ANGLE resolves it per target and renders correctly, so
 * the mistake leaves no mark on a frame: the morph delta array and the
 * `KHR_materials_specular` map shared unit 9 for as long as both existed, and
 * every pixel test stayed green.
 *
 * That makes the allocation itself the only thing worth checking. This gate
 * reads the map rather than a rendered result, and it needs no GL context to
 * do it.
 */
import assert from 'node:assert/strict';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const source = (name) => pathToFileURL(resolve(here, '..', 'src', 'viewer', name)).href;

// The unit map needs no context, but the module that reads it sits beside code
// that names the GL constants at import time.
globalThis.WebGL2RenderingContext = class {};

const {
  MAX_MATERIAL_TEXTURE_UNITS,
  MAX_TEXTURE_UNITS,
  SHARED_TEXTURE_UNITS,
  assertTextureUnitBudget,
  materialTextureUnit,
} = await import(source('texture-units.ts'));
const { TEXTURE_SLOTS } = await import(source('shaders.ts'));
const { MORPH_TEXTURE_UNIT } = await import(source('morph-texture.ts'));

const shared = Object.entries(SHARED_TEXTURE_UNITS);
const material = TEXTURE_SLOTS.map((_, slot) => materialTextureUnit(slot));

// Every unit the renderer can bind in one draw, named by what claims it.
const claims = new Map();
for (const [name, unit] of shared) claims.set(unit, [`shared:${name}`]);
for (const [slot, unit] of material.entries()) {
  claims.set(unit, [...(claims.get(unit) ?? []), `material:${TEXTURE_SLOTS[slot]}`]);
}

for (const [unit, owners] of claims) {
  assert.equal(owners.length, 1, `texture unit ${unit} is claimed by ${owners.join(' and ')}`);
  assert.ok(unit >= 0 && unit < MAX_TEXTURE_UNITS, `texture unit ${unit} is outside the guaranteed range`);
}

// The morph array is the sampler whose type differs from every material slot's,
// which is what made its collision invalid rather than merely wasteful.
assert.equal(
  MORPH_TEXTURE_UNIT,
  SHARED_TEXTURE_UNITS.morphDeltas,
  'the morph delta array must take its unit from the shared map',
);
assert.equal(
  material.includes(MORPH_TEXTURE_UNIT),
  false,
  'the morph delta array is a sampler2DArray; no material slot may share its unit',
);

// The budget is a real limit, not a comment: a slot list that reaches the
// frame's own samplers has to fail where it is declared.
assert.equal(
  MAX_MATERIAL_TEXTURE_UNITS,
  MAX_TEXTURE_UNITS - shared.length,
  'the material budget is whatever the frame does not reserve',
);
assert.doesNotThrow(() => assertTextureUnitBudget(MAX_MATERIAL_TEXTURE_UNITS));
assert.throws(
  () => assertTextureUnitBudget(MAX_MATERIAL_TEXTURE_UNITS + 1),
  /exceed/,
  'one slot past the budget must be refused where the list is declared',
);
assert.doesNotThrow(
  () => assertTextureUnitBudget(TEXTURE_SLOTS.length),
  'the slot list the shader is built from must fit',
);

console.log(`viewer-texture-units: OK (${TEXTURE_SLOTS.length} material slots of ${MAX_MATERIAL_TEXTURE_UNITS})`);
