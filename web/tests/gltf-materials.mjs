/**
 * Material extension parsing for the preview.
 *
 * The renderer reads flat records, not glTF JSON, so a mis-defaulted extension
 * field is invisible until a coated asset renders flat. These gates pin the
 * defaults a material without extensions must keep (they reproduce the core
 * metallic-roughness model exactly) and the values the four supported material
 * extensions contribute.
 *
 * Runs against the compiled output, so `npm run build:ts` has to be current or
 * this passes against the previous edit.
 */
import assert from 'node:assert/strict';

const { buildMaterials, extensionWarnings } = await import('../src/gltf-loader.ts');

// A bare material must shade exactly as it did before the extensions landed:
// no coat, the index of refraction the core model implies, full specular.
const [bare] = buildMaterials([{}]);
assert.equal(bare.clearcoatFactor, 0, 'a material without KHR_materials_clearcoat must have no coat');
assert.equal(bare.clearcoatRoughnessFactor, 0);
assert.equal(bare.clearcoatTexture, null);
assert.equal(bare.clearcoatRoughnessTexture, null);
assert.equal(bare.clearcoatNormalTexture, null);
assert.equal(bare.ior, 1.5, 'the core model implies an index of refraction of 1.5 (f0 = 0.04)');
assert.equal(bare.specularFactor, 1);
assert.deepEqual(bare.specularColorFactor, [1, 1, 1]);
assert.equal(bare.emissiveStrength, 1);

// ClearcoatTest.glb drives its coated variants exactly this way: a full-strength
// coat whose roughness comes from a factor, a texture, or both.
const [coated] = buildMaterials([{
  extensions: {
    KHR_materials_clearcoat: {
      clearcoatFactor: 1,
      clearcoatRoughnessFactor: 0.03,
      clearcoatTexture: { index: 0, texCoord: 0 },
      clearcoatRoughnessTexture: { index: 2, texCoord: 1 },
      clearcoatNormalTexture: { index: 3, scale: 0.5 },
    },
  },
}]);
assert.equal(coated.clearcoatFactor, 1);
assert.equal(coated.clearcoatRoughnessFactor, 0.03);
assert.deepEqual(coated.clearcoatTexture, { index: 0, texCoord: 0 });
assert.deepEqual(coated.clearcoatRoughnessTexture, { index: 2, texCoord: 1 });
assert.deepEqual(coated.clearcoatNormalTexture, { index: 3, texCoord: 0, scale: 0.5 });

// A clearcoat texture without an explicit roughness factor still coats: the
// factors default to 0 only when the extension itself is absent.
const [partial] = buildMaterials([{
  extensions: { KHR_materials_clearcoat: { clearcoatFactor: 1 } },
}]);
assert.equal(partial.clearcoatFactor, 1);
assert.equal(partial.clearcoatRoughnessFactor, 0);

const [dielectric] = buildMaterials([{
  extensions: {
    KHR_materials_ior: { ior: 1.8 },
    KHR_materials_specular: {
      specularFactor: 0.25,
      specularColorFactor: [1, 0.5, 0.25],
      specularTexture: { index: 4 },
      specularColorTexture: { index: 5, texCoord: 1 },
    },
    KHR_materials_emissive_strength: { emissiveStrength: 4 },
  },
  emissiveFactor: [1, 1, 1],
}]);
assert.equal(dielectric.ior, 1.8);
assert.equal(dielectric.specularFactor, 0.25);
assert.deepEqual(dielectric.specularColorFactor, [1, 0.5, 0.25]);
assert.deepEqual(dielectric.specularTexture, { index: 4, texCoord: 0 });
assert.deepEqual(dielectric.specularColorTexture, { index: 5, texCoord: 1 });
assert.equal(dielectric.emissiveStrength, 4);

// An index of refraction of 0 is legal (KHR_materials_ior spells it as the
// "no reflectance" case) and must survive rather than fall back to 1.5.
assert.equal(buildMaterials([{ extensions: { KHR_materials_ior: { ior: 0 } } }])[0].ior, 0);

// The trailing entry is the fallback for primitives without a material, and it
// must stay unlit-free and uncoated.
const list = buildMaterials([{}]);
assert.equal(list.length, 2);
assert.equal(list[1].unlit, false);
assert.equal(list[1].clearcoatFactor ?? 0, 0);

// The extensions above are now acted on, so they must stop being reported as
// ignored — the warning is what tells a user the preview is incomplete.
const supported = [
  'KHR_materials_clearcoat',
  'KHR_materials_ior',
  'KHR_materials_specular',
  'KHR_materials_emissive_strength',
  'KHR_materials_unlit',
  'KHR_texture_transform',
];
assert.deepEqual(extensionWarnings({ extensionsUsed: supported }, new Map()), []);
assert.deepEqual(
  extensionWarnings({ extensionsUsed: [...supported, 'KHR_materials_sheen'] }, new Map()),
  ['Unsupported glTF extensions ignored: KHR_materials_sheen'],
);

console.log('glTF material extension smoke passed');
