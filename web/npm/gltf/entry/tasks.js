// What the pool's workers may run for this entry. A GltfAsset and the
// geometry it reads are wasm objects, which do not cross to another thread, so
// read_gltf walks them here and returns plain arrays.
import { GltfAsset } from './index.js';

export { inspect_gltf } from './index.js';

function plainGeometry(geometry) {
  const attributes = [];
  for (let i = 0; i < geometry.attributeCount(); i += 1) {
    attributes.push({
      semantic: geometry.attributeSemantic(i),
      count: geometry.attributeElementCount(i),
      components: geometry.attributeComponents(i),
      componentType: geometry.attributeComponentType(i),
      normalized: geometry.attributeNormalized(i),
      bytes: geometry.attributeBytes(i),
    });
  }
  const index = geometry.hasIndices()
    ? { count: geometry.indexCount(), componentType: geometry.indexComponentType(), bytes: geometry.indexBytes() }
    : null;
  return { mode: geometry.mode(), attributes, index };
}

/** The document's JSON and every primitive of every mesh, Draco or not. */
export function read_gltf(data, resources, profile = '2.0', decodeOptions) {
  const asset = resources
    ? GltfAsset.withResources(data, resources, profile, decodeOptions)
    : new GltfAsset(data, profile, decodeOptions);
  try {
    const meshes = [];
    for (let mesh = 0; mesh < asset.meshCount(); mesh += 1) {
      const primitives = [];
      for (let primitive = 0; primitive < asset.primitiveCount(mesh); primitive += 1) {
        const geometry = asset.readPrimitive(mesh, primitive);
        try {
          primitives.push(plainGeometry(geometry));
        } finally {
          geometry.free();
        }
      }
      meshes.push(primitives);
    }
    return { json: asset.json(), meshes };
  } finally {
    asset.free();
  }
}
