/**
 * The layered glTF material extensions, declared once.
 *
 * Each of these says the same thing at seven points in the pipeline: what the
 * field is called, what the core model implies when the extension is absent,
 * and which texture slots come with it. Reading, lowering into a SceneDocument,
 * writing a manifest back out, filling defaults for two viewer producers and
 * the renderer, validating a document, and deciding what FBX cannot express —
 * every one of them restated the default in its own words, and `ior === 1.5`
 * appeared seven times across seven files.
 *
 * A default that disagrees between any two of those is silent in both
 * directions: too eager and every exported material declares an extension no
 * reader needs to honor, too lax and a coated asset shades flat. So the
 * declaration lives here and the seven behaviours are derived from it.
 *
 * Nothing here decides *policy* — which extensions a given consumer honors, and
 * what it says when it cannot, stays with the consumer.
 */

/** A scalar or vector the extension contributes to a material. */
export interface MaterialExtensionField {
  /** Property name on the interpreted, portable and viewer materials alike. */
  property: string;
  /** Field name inside the extension object, when it differs from `property`. */
  json?: string;
  /** What the core metallic-roughness model implies without the extension. */
  default: number | number[];
}

/** A texture slot the extension contributes. */
export interface MaterialExtensionTexture {
  property: string;
  json?: string;
  /** Normal maps carry a `scale` beside the binding. */
  scale?: boolean;
}

export interface MaterialExtensionSpec {
  /** The glTF extension name, exactly as it appears in `extensionsUsed`. */
  name: string;
  /**
   * A property the extension sets simply by being present, with no fields of
   * its own. `KHR_materials_unlit` is written as an empty object.
   */
  presence?: string;
  fields?: MaterialExtensionField[];
  textures?: MaterialExtensionTexture[];
}

export const MATERIAL_EXTENSIONS: readonly MaterialExtensionSpec[] = [
  { name: 'KHR_materials_unlit', presence: 'unlit' },
  {
    name: 'KHR_materials_emissive_strength',
    fields: [{ property: 'emissiveStrength', default: 1 }],
  },
  {
    // 1.5 is the index of refraction the core model implies (f0 = 0.04), so a
    // material without the extension is shaded exactly as before.
    name: 'KHR_materials_ior',
    fields: [{ property: 'ior', default: 1.5 }],
  },
  {
    name: 'KHR_materials_specular',
    fields: [
      { property: 'specularFactor', default: 1 },
      { property: 'specularColorFactor', default: [1, 1, 1] },
    ],
    textures: [{ property: 'specularTexture' }, { property: 'specularColorTexture' }],
  },
  {
    // Absent clearcoat means no coat at all, not a coat of default roughness.
    name: 'KHR_materials_clearcoat',
    fields: [
      { property: 'clearcoatFactor', default: 0 },
      { property: 'clearcoatRoughnessFactor', default: 0 },
    ],
    textures: [
      { property: 'clearcoatTexture' },
      { property: 'clearcoatRoughnessTexture' },
      { property: 'clearcoatNormalTexture', scale: true },
    ],
  },
] as const;

/** Every extension name the table covers. */
export const MATERIAL_EXTENSION_NAMES: readonly string[] = MATERIAL_EXTENSIONS.map((spec) => spec.name);

/** Every texture slot the extensions contribute, in table order. */
export const MATERIAL_EXTENSION_TEXTURE_SLOTS: readonly string[] = MATERIAL_EXTENSIONS
  .flatMap((spec) => (spec.textures ?? []).map((texture) => texture.property));

/**
 * What every extension property means when nobody set it.
 *
 * A presence flag defaults to `false`; every other property to the value the
 * core model implies. Consumers read defaults from here instead of spelling
 * them again.
 */
export const MATERIAL_EXTENSION_DEFAULTS: Readonly<Record<string, number | number[] | false>> =
  Object.freeze(Object.fromEntries(MATERIAL_EXTENSIONS.flatMap((spec) => [
    ...(spec.presence ? [[spec.presence, false] as const] : []),
    ...(spec.fields ?? []).map((field) => [field.property, field.default] as const),
  ])));

/** Whether `value` is what the core model implies for `property`. */
export function isMaterialExtensionDefault(property: string, value: unknown): boolean {
  const fallback = MATERIAL_EXTENSION_DEFAULTS[property];
  if (value === undefined) return true;
  if (Array.isArray(fallback)) {
    return Array.isArray(value) && value.length === fallback.length
      && value.every((entry, index) => entry === fallback[index]);
  }
  return value === fallback;
}

/**
 * Read every extension the table covers off one `materials[]` entry.
 *
 * Fields absent from the JSON come back as their defaults, so a caller never
 * has to ask whether the extension was there.
 *
 * @param extensions  The material's `extensions` object, or nothing.
 * @param readTexture How this caller turns a glTF textureInfo into its own
 *   binding; the table knows which slots exist, not what a binding looks like.
 */
export function readMaterialExtensions<Binding>(
  extensions: Record<string, any> | undefined | null,
  readTexture: (info: unknown, scalars: { scale?: boolean }) => Binding,
): Record<string, unknown> {
  const source = extensions || {};
  const material: Record<string, unknown> = {};
  for (const spec of MATERIAL_EXTENSIONS) {
    const declared = source[spec.name];
    if (spec.presence) material[spec.presence] = Boolean(declared);
    for (const field of spec.fields ?? []) {
      const value = declared?.[field.json ?? field.property];
      material[field.property] = Array.isArray(field.default)
        ? numbers(value, field.default)
        : value ?? field.default;
    }
    for (const texture of spec.textures ?? []) {
      material[texture.property] = readTexture(
        declared?.[texture.json ?? texture.property],
        texture.scale ? { scale: true } : {},
      );
    }
  }
  return material;
}

/**
 * The extension objects a material asks a glTF manifest to carry.
 *
 * Only what the material states beyond the core model is written: emitting
 * `KHR_materials_ior` with the default 1.5 on every material would declare an
 * extension no reader needs to honor and grow `extensionsUsed` for nothing.
 * A block appears when any of its fields differs or any of its textures binds,
 * and then carries exactly those.
 *
 * @param writeTexture Lowers one binding into this manifest's index space, or
 *   returns null when the texture it named could not be written.
 */
export function writeMaterialExtensions<Info>(
  material: Record<string, any>,
  writeTexture: (property: string, scale: boolean) => Info | null,
): Record<string, Record<string, unknown>> {
  const extensions: Record<string, Record<string, unknown>> = {};
  for (const spec of MATERIAL_EXTENSIONS) {
    if (spec.presence) {
      if (material[spec.presence]) extensions[spec.name] = {};
      continue;
    }
    const block: Record<string, unknown> = {};
    for (const field of spec.fields ?? []) {
      const value = material[field.property];
      if (value === undefined || isMaterialExtensionDefault(field.property, value)) continue;
      block[field.json ?? field.property] = Array.isArray(value) ? [...value] : value;
    }
    for (const texture of spec.textures ?? []) {
      const info = writeTexture(texture.property, Boolean(texture.scale));
      if (info) block[texture.json ?? texture.property] = info;
    }
    if (Object.keys(block).length > 0) extensions[spec.name] = block;
  }
  return extensions;
}

/**
 * Every extension scalar and flag a material carries, defaults filled in.
 *
 * What a consumer that has to shade the material needs: the portable form omits
 * anything equal to its default, and a material from OBJ, PLY or FBX never had
 * these properties at all, so both arrive here as the core model's values.
 * Texture slots are left to the caller — each consumer binds them differently.
 */
export function materialExtensionFactors(
  material: Record<string, any> | undefined | null,
): Record<string, number | number[] | boolean> {
  const source = material || {};
  const values: Record<string, number | number[] | boolean> = {};
  for (const [property, fallback] of Object.entries(MATERIAL_EXTENSION_DEFAULTS)) {
    const value = source[property];
    if (typeof fallback === 'boolean') values[property] = Boolean(value);
    else if (Array.isArray(fallback)) values[property] = [...(Array.isArray(value) ? value : fallback)];
    else values[property] = value ?? fallback;
  }
  return values;
}

/**
 * Whether a material states anything beyond the core metallic-roughness model.
 *
 * Asked by every writer whose format has no layered materials, so that "this
 * export dropped something" is one question with one answer.
 */
export function hasMaterialExtensionValues(material: Record<string, any>): boolean {
  return MATERIAL_EXTENSIONS.some((spec) => {
    if (spec.presence && material[spec.presence]) return true;
    if ((spec.fields ?? []).some((field) => !isMaterialExtensionDefault(field.property, material[field.property]))) {
      return true;
    }
    return (spec.textures ?? []).some((texture) => Boolean(material[texture.property]));
  });
}

function numbers(value: unknown, fallback: number[]): number[] {
  return Array.isArray(value) ? Array.from(value, Number) : [...fallback];
}
