import type { SceneDocument } from '../scene-document.ts';
import { buildFbxSceneFromDocument } from '../fbx-scene-document-writer.ts';
import { buildFbxSceneFromGltf, buildFlatMeshesFromGltf } from '../gltf-loader.ts';
import { serializeSceneDocumentToGlb } from '../scene-document-gltf.ts';
import { modules, state } from './state.ts';

/**
 * Which route a loaded file takes to a downloadable one, and what that route
 * costs.
 *
 * Separate from export.ts because that file reaches for the DOM at module load
 * and can therefore never be exercised outside a browser, while everything
 * here — route selection, mesh preparation, merging — is pure enough to test
 * directly. export.ts keeps the controls, the download and the console.
 *
 * Every route returns the same outcome shape, warnings included. That is the
 * point: five of the six used to compute warnings and drop them on the floor,
 * so a user converting a lit, textured glTF to FBX was told nothing about what
 * did not survive.
 */

/** What one export route produced, and what it cost to produce it. */
export interface ExportOutcome {
  result: any;
  warnings: string[];
  capabilities?: Record<string, unknown>;
}

/** The export controls, read once by the caller so this stays DOM-free. */
export interface ExportSettings {
  format: string;
  includeNormals: boolean;
  includeUvs: boolean;
  useDraco: boolean;
  encodingSpeed: number;
}

/** Route the loaded file to a writer and report what the route cost. */
export async function runExport(settings: ExportSettings): Promise<ExportOutcome> {
  const { format } = settings;
  const legacyFbx = format === 'fbx-legacy';
  const isFbxTarget = format === 'fbx' || legacyFbx;

  if (format === 'glb' && state.currentFileType === 'fbx' && state.currentSceneDocument) {
    return exportSceneDocumentToGlb(state.currentSceneDocument);
  }
  if (state.currentMeshData.document && (format === 'gltf' || format === 'glb')) {
    return exportGltfDocument(settings);
  }
  if (isFbxTarget && state.currentFileType === 'fbx' && state.currentSceneDocument) {
    const scene = buildFbxSceneFromDocument(state.currentSceneDocument, {
      provenance: state.currentFbxProvenance,
    });
    return {
      result: await exportToFbxScene(scene, legacyFbx),
      warnings: [...(scene.warnings || [])],
    };
  }
  if (isFbxTarget && state.currentMeshData.scene) {
    const scene = prepareFbxSceneForExport(state.currentMeshData.scene, settings, legacyFbx);
    return {
      result: await exportToFbxScene(scene, legacyFbx),
      warnings: [...(scene.warnings || [])],
    };
  }
  if (state.currentMeshData.document && isFbxTarget) {
    const source = buildFbxSceneFromGltf(
      state.currentSourceData!,
      state.currentSourceResources,
      modules.gltf.module,
      { legacyCompatibility: legacyFbx },
    );
    const scene = prepareFbxSceneForExport(source, settings, legacyFbx);
    return {
      result: await exportToFbxScene(scene, legacyFbx),
      warnings: [...(scene.warnings || [])],
    };
  }
  return exportFlattenedMeshes(settings);
}

/** GLB from the portable document — the route every non-glTF source takes. */
export function exportSceneDocumentToGlb(document: SceneDocument): ExportOutcome {
  if (!modules.gltf.loaded) throw new Error('glTF module not loaded');
  const output = serializeSceneDocumentToGlb(document, modules.gltf.module);
  return {
    result: {
      success: true,
      binary_data: output.binary,
      message: 'FBX SceneDocument exported as GLB',
    },
    warnings: [...(output.warnings || [])],
    capabilities: output.capabilities,
  };
}

/**
 * glTF and GLB out of a glTF source, straight from the original bytes.
 *
 * This route deliberately does **not** go through SceneDocument, and the
 * asymmetry with every other route here is the whole reason it exists. The
 * asset is mutated in place and re-serialized, so everything the portable
 * document cannot model survives: asset.copyright and extras at every level,
 * multiple scenes, cameras, KHR_lights_punctual, KHR_materials_variants and
 * any other unmodeled extension, sparse encoding, interleaved buffer views,
 * external image URIs, the original buffer layout and authored accessor
 * bounds. Draco is the sharpest case — `compressPrimitive` rewrites the
 * document in place, while SceneDocument has no way to even say a primitive is
 * compressed, so routing this through it would silently decompress the asset
 * and drop everything above with it.
 *
 * It reports no warnings for the same reason: it loses nothing.
 */
export function exportGltfDocument(settings: ExportSettings): ExportOutcome {
  const { format, useDraco, encodingSpeed } = settings;
  if (!modules.gltf.loaded) throw new Error('glTF module not loaded');

  const asset = modules.gltf.module.GltfAsset.withResources(
    state.currentSourceData,
    state.currentSourceResources,
    '2.1',
  );
  try {
    let compressedBytes = 0;
    let compressedPrimitives = 0;
    if (useDraco) {
      if (typeof asset.compressPrimitive !== 'function') {
        throw new Error('Draco encoding is not included in this WASM build');
      }
      for (let mesh = 0; mesh < asset.meshCount(); mesh += 1) {
        const primitiveCount = asset.primitiveCount(mesh);
        for (let primitive = 0; primitive < primitiveCount; primitive += 1) {
          // The encoder reports the payload it wrote; summing it is the only
          // compression figure this path can honestly produce.
          compressedBytes += asset.compressPrimitive(mesh, primitive, encodingSpeed, 5) || 0;
          compressedPrimitives += 1;
        }
      }
    }
    const dracoStats = useDraco
      ? { speed: encodingSpeed, compressed_size: compressedBytes, primitives: compressedPrimitives }
      : null;

    if (format === 'glb') {
      return {
        result: {
          success: true,
          binary_data: asset.glb(2),
          draco_stats: dracoStats,
          message: useDraco
            ? 'Document compressed with Draco and exported as GLB'
            : 'Document packaged and exported as GLB',
        },
        warnings: [],
      };
    }
    if (format === 'gltf' && state.currentFileType === 'gltf' && !useDraco) {
      return {
        result: {
          success: true,
          json_data: new TextDecoder().decode(asset.minifiedJson()),
          message: 'Document exported as minified JSON glTF',
        },
        warnings: [],
      };
    }
    if (format === 'gltf' && useDraco) {
      throw new Error('Compressed JSON glTF requires bundle download; select GLB instead');
    }
    throw new Error(`Document export to ${format.toUpperCase()} is not supported`);
  } finally {
    asset.free();
  }
}

/**
 * The last resort: triangles only, no scene.
 *
 * OBJ and PLY have nowhere to put a hierarchy, so for those sources this is
 * simply the format. Coming from a glTF document or an FBX scene it is a real
 * loss, and the warning says so rather than leaving the user to notice.
 */
async function exportFlattenedMeshes(settings: ExportSettings): Promise<ExportOutcome> {
  const { format } = settings;
  const warnings: string[] = [];
  const sourceMeshes = state.currentMeshData.document
    ? buildFlatMeshesFromGltf(
      state.currentSourceData!,
      state.currentSourceResources,
      modules.gltf.module,
    )
    : state.currentMeshData.meshes;
  if (state.currentMeshData.document || state.currentMeshData.scene) {
    warnings.push(
      `Exporting to ${format.toUpperCase()} flattens the scene: materials, textures, skins, `
      + 'animation and the node hierarchy are not written',
    );
  }
  const meshes = prepareMeshesForExport(sourceMeshes, settings);
  if (meshes.length === 0) {
    throw new Error('The document contains no triangle geometry to export');
  }
  if (format === 'ply' && meshes.length > 1) {
    warnings.push(`PLY holds one mesh: ${meshes.length} meshes were merged into one`);
  }

  switch (format) {
    case 'obj':
      return { result: await exportToObj(meshes, settings), warnings };
    case 'ply':
      return { result: await exportToPly(meshes, settings), warnings };
    case 'gltf':
    case 'glb':
      return { result: await exportToGltf(format), warnings };
    case 'fbx':
    case 'fbx-legacy':
      return { result: await exportToFbx(meshes), warnings };
    default:
      return { result: { success: false, error: `Unknown export format ${format}` }, warnings };
  }
}

/** Flatten source meshes into the shape the OBJ/PLY/FBX writers accept. */
export function prepareMeshesForExport(meshes: any[], settings: Pick<ExportSettings, 'includeNormals' | 'includeUvs'>) {
  const layerSets = (sets: any[]) => (sets || []).map((set: any) => ({
    name: set.name,
    mapping: set.mapping,
    reference: set.reference,
    values: Array.from(set.values || []),
    indices: Array.from(set.indices || []),
  }));
  return meshes.map((mesh: any, idx) => ({
    name: mesh.name || `mesh_${idx}`,
    positions: Array.from(mesh.positions || []),
    indices: Array.from(mesh.indices || []),
    normals: settings.includeNormals ? Array.from(mesh.normals || []) : null,
    uvs: settings.includeUvs ? Array.from(mesh.uvs || []) : null,
    controlPoints: mesh.controlPoints ? Array.from(mesh.controlPoints) : null,
    polygonVertexIndices: mesh.polygonVertexIndices
      ? Array.from(mesh.polygonVertexIndices)
      : null,
    uvSets: layerSets(mesh.uvSets),
    normalSets: layerSets(mesh.normalSets),
    colorSets: layerSets(mesh.colorSets),
  }));
}

export function prepareFbxSceneForExport(
  scene: any,
  settings: Pick<ExportSettings, 'includeNormals' | 'includeUvs'>,
  legacyCompatibility = false,
) {
  const prepareNode = (node: any) => ({
    ...node,
    meshes: (node.meshes || []).map((sourceMesh: any) => {
      const [mesh] = prepareMeshesForExport([sourceMesh], settings);
      return {
        ...mesh,
      // Keep FBX per-polygon assignments: `fbx-wasm` maps these to
      // LayerElementMaterial when serializing the scene again.
        materialIndices: Array.isArray(sourceMesh.materialIndices)
        ? sourceMesh.materialIndices
        : [],
        skin: sourceMesh.skin || null,
        morphTargets: sourceMesh.morphTargets || [],
      };
    }),
    children: (node.children || []).map(prepareNode),
  });
  // Named field by field rather than spread: this function exists to strip the
  // viewer-only state a scene carries, and `...scene` would put all of it back.
  // Which is also why `warnings` has to be listed — it was being dropped here,
  // silently, on the two routes that compute it.
  return {
    ...(scene.globalSettings ? { globalSettings: scene.globalSettings } : {}),
    rootNodes: (scene.rootNodes || []).map(prepareNode),
    materials: (scene.materials || []).map(prepareFbxMaterialForExport),
    textures: scene.textures || [],
    animations: (scene.animations || []).map((animation: any) => prepareFbxAnimationForExport(
      animation, legacyCompatibility,
    )),
    warnings: [...(scene.warnings || [])],
  };
}

/** Strip state.viewer-only fields and keep what fbx-wasm's MaterialInput accepts. */
export function prepareFbxMaterialForExport(material: any) {
  if (!material) return material;
  return {
    name: material.name,
    shadingModel: material.shadingModel,
    diffuse: material.diffuse,
    specular: material.specular,
    emissive: material.emissive,
    ambient: material.ambient,
    diffuseFactor: material.diffuseFactor,
    specularFactor: material.specularFactor,
    shininess: material.shininess,
    emissiveFactor: material.emissiveFactor,
    reflectionFactor: material.reflectionFactor,
    transparencyFactor: material.transparencyFactor,
    opacity: material.opacity,
    bumpFactor: material.bumpFactor,
    textures: material.textures || [],
  };
}

/** Pass animation clips through; fbx-wasm's AnimationInput mirrors the reader. */
export function prepareFbxAnimationForExport(animation: any, legacyCompatibility = false) {
  if (!animation) return animation;
  return {
    name: animation.name,
    duration: animation.duration,
    channels: (animation.channels || []).map((channel: any) => ({
      nodeName: channel.nodeName,
      nodeId: channel.nodeId,
      path: channel.path,
      ...(channel.morphTargetIndex === undefined ? {} : { morphTargetIndex: channel.morphTargetIndex }),
      sampler: legacyCompatibility ? {
        ...channel.sampler,
        // Legacy's importer has fragile support for cubic tangents.
        // Preserve key values but write robust linear curves.
        interpolation: 'linear',
        inTangents: null,
        outTangents: null,
      } : channel.sampler,
    })),
  };
}

export async function exportToObj(meshes: any[], settings: Pick<ExportSettings, 'includeNormals' | 'includeUvs'>) {
  if (!modules.obj.loaded) {
    return { success: false, error: 'OBJ module not loaded' };
  }
  const options = {
    include_normals: settings.includeNormals,
    include_uvs: settings.includeUvs,
    precision: 6,
  };
  if (meshes.length === 1) return modules.obj.module.create_obj(meshes[0], options);
  return modules.obj.module.create_obj_multi(meshes, options);
}

export async function exportToPly(meshes: any[], settings: Pick<ExportSettings, 'includeNormals'>) {
  if (!modules.ply.loaded) {
    return { success: false, error: 'PLY module not loaded' };
  }
  // PLY only supports single mesh, merge if multiple
  const merged = mergeMeshes(meshes);
  const options = {
    include_normals: settings.includeNormals,
    include_colors: true,
    precision: 6,
    format: 'ascii',
  };
  return modules.ply.module.create_ply(merged, options);
}

export async function exportToGltf(format: string) {
  return {
    success: false,
    error: `Creating ${format.toUpperCase()} from flattened meshes is not part of the document API`,
  };
}

export async function exportToFbx(meshes: any[]) {
  if (!modules.fbx.loaded) {
    return { success: false, error: 'FBX module not loaded' };
  }
  return modules.fbx.module.create_fbx(meshes, { version: 7500 });
}

export async function exportToFbxScene(scene: any, legacyCompatibility = false) {
  if (!modules.fbx.loaded) {
    return { success: false, error: 'FBX module not loaded' };
  }
  return modules.fbx.module.create_fbx_scene(scene, { version: 7500, legacyCompatibility });
}

/** Merge multiple meshes into one. */
export function mergeMeshes(meshes: any[]) {
  if (meshes.length === 1) return meshes[0];

  const merged = {
    name: 'merged',
    positions: [] as number[],
    indices: [] as number[],
    normals: [] as number[],
    uvs: [] as number[],
  };

  let vertexOffset = 0;

  // Appended one element at a time on purpose. `push(...values)` passes the
  // whole array as arguments and blows the call stack somewhere past a
  // hundred thousand of them, which a mesh reaches at roughly 40k vertices.
  const append = (into: number[], values: ArrayLike<number>) => {
    for (let index = 0; index < values.length; index += 1) into.push(values[index]);
  };

  for (const mesh of meshes) {
    append(merged.positions, mesh.positions);
    if (mesh.indices) {
      for (const idx of mesh.indices) merged.indices.push(idx + vertexOffset);
    }
    if (mesh.normals) append(merged.normals, mesh.normals);
    if (mesh.uvs) append(merged.uvs, mesh.uvs);
    vertexOffset += mesh.positions.length / 3;
  }

  return merged;
}
