/**
 * The one map of GL texture units.
 *
 * Every unit used to be a number written where it was needed: the material
 * slots carried theirs in the renderer's binding table, the IBL maps had
 * theirs inline in `bindEnvironmentIbl`, and the morph delta array had a
 * constant of its own. Nothing related the three, and they collided — the
 * morph array and the specular map both claimed unit 9. GL forbids one unit
 * being addressed as `sampler2DArray` and `sampler2D` in a single draw, so a
 * mesh that morphs *and* carries a specular map was drawing invalidly. ANGLE
 * resolves it per target and renders correctly anyway, which is exactly why
 * nobody noticed and why the check has to be on the allocation rather than on
 * a frame.
 *
 * So the allocation is stated once and in one direction: material slots grow
 * upward from zero, the samplers that outlive any one material sit at the top
 * of the range, and the gap between them is the budget.
 */

/**
 * The floor WebGL2 guarantees for fragment texture units. Implementations may
 * offer more; nothing here may assume it.
 */
export const MAX_TEXTURE_UNITS = 16;

/**
 * Units for the samplers that belong to the frame rather than to a material.
 *
 * Placed at the top so that adding a material slot never has to move them, and
 * so that the collision this map exists to prevent needs a material list long
 * enough to reach them — which `assertTextureUnitBudget` refuses.
 */
export const SHARED_TEXTURE_UNITS = {
  /** Where a transmissive volume ends, and which way its far wall faces. */
  backFace: MAX_TEXTURE_UNITS - 6,
  /** The opaque half of the frame, which a transmissive surface refracts. */
  frameSnapshot: MAX_TEXTURE_UNITS - 5,
  morphDeltas: MAX_TEXTURE_UNITS - 4,
  irradiance: MAX_TEXTURE_UNITS - 3,
  prefiltered: MAX_TEXTURE_UNITS - 2,
  brdfLut: MAX_TEXTURE_UNITS - 1,
} as const;

/** How many material slots can be bound at once, given what the frame reserves. */
export const MAX_MATERIAL_TEXTURE_UNITS = MAX_TEXTURE_UNITS
  - Object.keys(SHARED_TEXTURE_UNITS).length;

/** The unit a material texture slot binds to, by its index in the slot list. */
export function materialTextureUnit(slot: number): number {
  return slot;
}

/**
 * Refuse a slot list that would reach into the reserved units.
 *
 * Called where the slot list is fixed rather than per draw: overflowing it is a
 * build-time mistake about how many maps one program can carry, and the useful
 * moment to hear about it is the moment the list grows.
 */
export function assertTextureUnitBudget(slotCount: number) {
  if (slotCount <= MAX_MATERIAL_TEXTURE_UNITS) return;
  throw new Error(
    `${slotCount} material texture slots exceed the ${MAX_MATERIAL_TEXTURE_UNITS} `
    + `units left by the frame's own samplers (${Object.keys(SHARED_TEXTURE_UNITS).join(', ')}).`,
  );
}
