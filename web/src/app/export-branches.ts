import type { LoadedLayerSet, LoadedMesh, OpaqueAttribute } from '../mesh-loader.ts';
import { FBX_METERS_Y_UP } from '../fbx-space.ts';
import { buildSceneDocumentFromMeshes, flattenSceneDocument } from '../mesh-scene-document.ts';
import type { MeshDocumentOptions } from '../mesh-scene-document.ts';
import type { SceneCapabilities, SceneDocument } from '../scene-document.ts';
import { buildFbxSceneFromDocument } from '../fbx-scene-document-writer.ts';
import { buildFbxSceneFromGltf, buildFlatSceneMeshesFromGltf } from '../gltf-loader.ts';
import { glbToEmbeddedGltf, serializeSceneDocumentToGlb } from '../scene-document-gltf.ts';
import type { FbxSceneData, LoadedFile } from './state.ts';
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

/**
 * What a writer handed back.
 *
 * The three payload fields are mutually exclusive and format-dependent: the
 * binary writers fill `binary_data`, the glTF document route fills `json_data`,
 * and OBJ and PLY fill `data`. The downloader picks whichever is present.
 */
export interface ExportResult {
  success: boolean;
  error?: string;
  data?: string;
  json_data?: string;
  binary_data?: ArrayLike<number>;
  /** What the route did, for the console. */
  message?: string;
  /** Set by any route that writes a Draco-compressed payload. */
  draco_stats?: DracoStats | null;
  /** Set by an FBX route with the actual zlib-compressed array totals. */
  fbx_stats?: FbxStats | null;
}

/** What the FBX writer actually stored with zlib array encoding. */
export interface FbxStats {
  requested: boolean;
  compressed_arrays: number;
  compressed_raw_bytes: number;
  compressed_stored_bytes: number;
}

/** What the Draco encoder reported about the pass, summed over primitives. */
export interface DracoStats {
  speed: number;
  compressed_size: number;
  /** Resolved binary bytes before Draco rewrites the geometry resources. */
  source_bytes: number;
  /** Resolved binary bytes remaining after Draco rewrites the geometry resources. */
  output_bytes: number;
  /** Size of the complete downloaded export, when known to the UI. */
  output_size?: number;
  primitives: number;
  method?: string;
  prediction_scheme?: string;
}

/** What one glTF WASM compression call says the encoder actually selected. */
interface DracoPrimitiveReport {
  encoded_bytes: number;
  source_bytes: number;
  output_bytes: number;
  method: string;
  speed: number;
  prediction_scheme?: string | null;
}

/** What one export route produced, and what it cost to produce it. */
export interface ExportOutcome {
  result: ExportResult;
  warnings: string[];
  capabilities?: Partial<SceneCapabilities> & Record<string, boolean | number>;
}

/** The export controls, read once by the caller so this stays DOM-free. */
export interface ExportSettings {
  format: string;
  includeNormals: boolean;
  includeUvs: boolean;
  useDraco: boolean;
  /** Whether binary FBX arrays may use zlib compression. */
  fbxCompression?: boolean;
  /** Whether to flatten animation curves for Blender's legacy Python importer. */
  fbxLegacyCompatibility?: boolean;
  encodingSpeed: number;
  /** Quantization, shared by the glTF Draco pass and the `.drc` route. */
  positionBits?: number;
  normalBits?: number;
  texcoordBits?: number;
  colorBits?: number;
  genericBits?: number;
}

/** Route the loaded file to a writer and report what the route cost. */
export async function runExport(settings: ExportSettings): Promise<ExportOutcome> {
  const { format } = settings;
  const legacyFbx = format === 'fbx-legacy'
    || (format === 'fbx' && settings.fbxLegacyCompatibility === true);
  const isFbxTarget = format === 'fbx' || legacyFbx;
  // export.ts checks this before offering the controls; repeated here because
  // every route below reads the parse result and none of them can invent one.
  const loaded = state.currentMeshData;
  if (!loaded) throw new Error('No parsed file to export');

  if ((format === 'glb' || format === 'gltf')
    && state.currentFileType === 'fbx' && state.currentSceneDocument) {
    // JSON as much as GLB: both are the same document, and flattening an FBX
    // into a mesh list to reach one of them would throw away the hierarchy the
    // other keeps.
    return asGltfIfAsked(exportSceneDocumentToGlb(state.currentSceneDocument, settings), format);
  }
  if (loaded.document && (format === 'gltf' || format === 'glb')) {
    return exportGltfDocument(settings);
  }
  if (isFbxTarget && state.currentFileType === 'fbx' && state.currentSceneDocument) {
    const scene = buildFbxSceneFromDocument(state.currentSceneDocument, {
      provenance: state.currentFbxProvenance,
    });
    return {
      result: await exportToFbxScene(scene, legacyFbx, settings.fbxCompression),
      warnings: [...(scene.warnings || [])],
    };
  }
  if (isFbxTarget && loaded.scene) {
    const scene = prepareFbxSceneForExport(loaded.scene, settings, legacyFbx);
    return {
      result: await exportToFbxScene(scene, legacyFbx, settings.fbxCompression),
      warnings: [...(scene.warnings || [])],
    };
  }
  if (loaded.document && isFbxTarget) {
    const source = buildFbxSceneFromGltf(
      state.currentSourceData!,
      state.currentSourceResources,
      modules.gltf.module,
      { legacyCompatibility: legacyFbx },
    );
    const scene = prepareFbxSceneForExport(source, settings, legacyFbx);
    return {
      result: await exportToFbxScene(scene, legacyFbx, settings.fbxCompression),
      warnings: [...(scene.warnings || [])],
    };
  }
  return exportFlattenedMeshes(settings, loaded);
}

/** What the Draco pass reads out of the export controls. */
export type DracoPassSettings = Pick<
  ExportSettings,
  'useDraco' | 'encodingSpeed' | 'positionBits' | 'normalBits' | 'texcoordBits' | 'colorBits' | 'genericBits'
>;

/**
 * Quantization defaults, matching the sliders the panel ships with, which in
 * turn match Blender's glTF exporter: 14/10/12/10/12.
 *
 * A caller that omits them still gets a quantized payload, because the
 * alternative is not "slightly larger" — an unquantized attribute never reaches
 * Draco's integer coder, so no prediction scheme runs on it and the encoding
 * speed makes no difference to it whatsoever.
 */
const DRACO_DEFAULT_BITS = {
  position: 14, normal: 10, texcoord: 12, color: 10, generic: 12,
} as const;

function dracoQuantization(settings: DracoPassSettings) {
  return {
    position: settings.positionBits ?? DRACO_DEFAULT_BITS.position,
    normal: settings.normalBits ?? DRACO_DEFAULT_BITS.normal,
    texcoord: settings.texcoordBits ?? DRACO_DEFAULT_BITS.texcoord,
    color: settings.colorBits ?? DRACO_DEFAULT_BITS.color,
    generic: settings.genericBits ?? DRACO_DEFAULT_BITS.generic,
  };
}

/**
 * One `compressPrimitive` call, with both speeds carrying the slider.
 *
 * Draco resolves its two speeds as `max(encoding, decoding)`, so a decoding
 * speed pinned at a constant silently raises every slider position below it and
 * leaves that whole part of the control with no effect.
 */
function compressOnePrimitive(
  asset: { compressPrimitive: (...args: number[]) => DracoPrimitiveReport },
  mesh: number,
  primitive: number,
  encodingSpeed: number,
  quantization: ReturnType<typeof dracoQuantization>,
): DracoPrimitiveReport {
  return asset.compressPrimitive(
    mesh,
    primitive,
    encodingSpeed,
    encodingSpeed,
    quantization.position,
    quantization.normal,
    quantization.texcoord,
    quantization.color,
    quantization.generic,
  );
}

function aggregateDracoStats(
  reports: DracoPrimitiveReport[],
  requestedSpeed: number,
): DracoStats {
  const unique = (values: Array<string | null | undefined>) => [...new Set(
    values.filter((value): value is string => Boolean(value)),
  )];
  const methods = unique(reports.map((report) => report.method));
  const predictions = unique(reports.map((report) => report.prediction_scheme));
  return {
    speed: reports[0]?.speed ?? requestedSpeed,
    compressed_size: reports.reduce((total, report) => total + report.encoded_bytes, 0),
    source_bytes: reports.reduce((total, report) => total + report.source_bytes, 0),
    output_bytes: reports.reduce((total, report) => total + report.output_bytes, 0),
    primitives: reports.length,
    ...(methods.length > 0 ? { method: methods.join(', ') } : {}),
    ...(predictions.length > 0 ? { prediction_scheme: predictions.join('; ') } : {}),
  };
}

/**
 * GLB from the portable document — the route every non-glTF source takes.
 *
 * Draco is applied as a second pass over the GLB this route just produced,
 * rather than as something the document itself can express. That is the whole
 * of it: the encoder works on a glTF document, the document lowers to one, and
 * `SceneDocument` needs no idea that compression exists. Before this the
 * checkbox was simply read and dropped here, so an FBX exported as a
 * "compressed" GLB came out uncompressed and said nothing about it.
 */
export function exportSceneDocumentToGlb(
  document: SceneDocument,
  settings?: DracoPassSettings,
): ExportOutcome {
  if (!modules.gltf.loaded) throw new Error('glTF module not loaded');
  const output = serializeSceneDocumentToGlb(document, modules.gltf.module);
  const warnings = [...(output.warnings || [])];
  const compressed = settings?.useDraco
    ? compressGlb(output.binary, settings, warnings)
    : null;
  return {
    result: {
      success: true,
      binary_data: compressed?.binary ?? output.binary,
      draco_stats: compressed?.stats ?? null,
      message: compressed
        ? 'SceneDocument compressed with Draco and exported as GLB'
        : 'SceneDocument exported as GLB',
    },
    warnings,
    capabilities: output.capabilities,
  };
}

/**
 * Compress a GLB this application produced, or explain why it could not.
 *
 * A refusal here is our own doing, not a property of the user's file: the only
 * extensions this GLB can carry are the ones the document writer put in it. So
 * the export still succeeds and hands over the uncompressed bytes with the
 * reason attached, rather than failing and leaving the user with nothing.
 */
function compressGlb(glb: Uint8Array, settings: DracoPassSettings, warnings: string[]) {
  const asset = modules.gltf.module.GltfAsset.withResources(glb, Object.create(null), '2.1');
  const { encodingSpeed } = settings;
  const quantization = dracoQuantization(settings);
  try {
    if (typeof asset.compressPrimitive !== 'function') {
      warnings.push('Draco encoding is not included in this WASM build; the GLB was written uncompressed');
      return null;
    }
    const reports: DracoPrimitiveReport[] = [];
    for (let mesh = 0; mesh < asset.meshCount(); mesh += 1) {
      const primitiveCount = asset.primitiveCount(mesh);
      for (let primitive = 0; primitive < primitiveCount; primitive += 1) {
        reports.push(compressOnePrimitive(asset, mesh, primitive, encodingSpeed, quantization));
      }
    }
    return {
      binary: asset.glb(2),
      stats: aggregateDracoStats(reports, encodingSpeed),
    };
  } catch (error) {
    warnings.push(
      `Draco compression was refused, so the GLB was written uncompressed: ${error instanceof Error ? error.message : String(error)}`,
    );
    return null;
  } finally {
    asset.free();
  }
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
    const reports: DracoPrimitiveReport[] = [];
    if (useDraco) {
      if (typeof asset.compressPrimitive !== 'function') {
        throw new Error('Draco encoding is not included in this WASM build');
      }
      const quantization = dracoQuantization(settings);
      for (let mesh = 0; mesh < asset.meshCount(); mesh += 1) {
        const primitiveCount = asset.primitiveCount(mesh);
        for (let primitive = 0; primitive < primitiveCount; primitive += 1) {
          // The encoder reports the payload it wrote; summing it is the only
          // compression figure this path can honestly produce.
          reports.push(compressOnePrimitive(asset, mesh, primitive, encodingSpeed, quantization));
        }
      }
    }
    const dracoStats = useDraco
      ? aggregateDracoStats(reports, encodingSpeed)
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
      // The source document itself, byte for byte past minification. Nothing
      // this route could rebuild would be closer to what was opened.
      return {
        result: {
          success: true,
          json_data: new TextDecoder().decode(asset.minifiedJson()),
          message: 'Document exported as minified JSON glTF',
        },
        warnings: [],
      };
    }
    if (format === 'gltf') {
      // A GLB source, or a compressed one: the buffer is a chunk rather than a
      // file beside the JSON, so it is embedded rather than refused.
      return {
        result: {
          success: true,
          json_data: glbToEmbeddedGltf(asset.glb(2)),
          draco_stats: dracoStats,
          message: 'Exported as JSON glTF with its binary embedded',
        },
        warnings: [],
      };
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
async function exportFlattenedMeshes(settings: ExportSettings, loaded: LoadedFile): Promise<ExportOutcome> {
  const { format } = settings;
  const warnings: string[] = [];
  // Placed rather than raw: the target has no hierarchy to put node transforms
  // into, so they are baked into the coordinates here. Without that a scene's
  // objects all come out stacked on the origin, inside one another.
  //
  // The portable document is the source of that placement wherever one was
  // built, which is every scene-bearing format. The glTF-specific flattener
  // remains for the one case that has no document: a glTF whose document could
  // not be built still previews and still exports, and it says so when it is
  // opened.
  const sourceMeshes = state.currentSceneDocument
    ? flattenSceneDocument(state.currentSceneDocument)
    : loaded.document
      ? buildFlatSceneMeshesFromGltf(
        state.currentSourceData!,
        state.currentSourceResources,
        modules.gltf.module,
      )
      : loaded.meshes || [];
  if (loaded.document || loaded.scene) {
    warnings.push(
      `Exporting to ${format.toUpperCase()} flattens the scene: materials, textures, skins, `
      + 'animation and the node hierarchy are not written',
    );
  }
  const meshes = prepareMeshesForExport(sourceMeshes, settings);
  if (meshes.length === 0) {
    throw new Error('The document contains no triangle geometry to export');
  }
  // Every single-mesh target says the same thing about the same merge.
  if (['ply', 'stl', 'drc'].includes(format) && meshes.length > 1) {
    warnings.push(
      `${format.toUpperCase()} holds one mesh: ${meshes.length} meshes were merged into one`,
    );
  }
  // Uninterpreted attributes only exist because a .drc brought them, and only
  // a .drc can take them back. Every other target drops them, and that is worth
  // one line rather than a silent difference in the file that comes out.
  const opaque = meshes.flatMap((mesh) => mesh.extras);
  if (format !== 'drc' && opaque.length > 0) {
    warnings.push(
      `${format.toUpperCase()} has nowhere for ${opaque.length} attribute(s) the source carried `
      + `without interpreting: ${opaque.map((extra) => `${extra.type} (id ${extra.uniqueId})`).join(', ')}`,
    );
  }
  if (format === 'stl' && (settings.includeNormals || settings.includeUvs)) {
    warnings.push(
      'STL holds triangle positions only: normals are recomputed per facet from the winding, '
      + 'and texture coordinates are not written',
    );
  }

  switch (format) {
    case 'obj':
      return { result: await exportToObj(meshes, settings), warnings };
    case 'ply':
      return { result: await exportToPly(meshes, settings), warnings };
    case 'stl':
      return { result: await exportToStl(meshes), warnings };
    case 'drc':
      return { result: await exportToDrc(meshes, settings), warnings };
    case 'gltf':
    case 'glb': {
      // The document route reports its own warnings, and the flattening ones
      // above do not apply: a GLB keeps the hierarchy, the materials and the
      // attributes a mesh list carries.
      const gltf = exportMeshesAsGltf(meshes, settings, {
        materials: loaded.materials,
        resources: state.currentSourceResources,
      });
      return { ...gltf, warnings: gltf.warnings };
    }
    case 'fbx':
    case 'fbx-legacy':
      return { result: await exportToFbx(meshes, settings.fbxCompression), warnings };
    default:
      return { result: { success: false, error: `Unknown export format ${format}` }, warnings };
  }
}

/**
 * One mesh in the shape the flat writers accept: the reader's arrays copied
 * into real ones, with excluded channels nulled out. Inferred from the function
 * that produces it so the two cannot disagree.
 */
export type PreparedMesh = ReturnType<typeof prepareMeshesForExport>[number];

/** Flatten source meshes into the shape the OBJ/PLY/FBX writers accept. */
export function prepareMeshesForExport(
  meshes: LoadedMesh[],
  settings: Pick<ExportSettings, 'includeNormals' | 'includeUvs'>,
) {
  const layerSets = (sets: LoadedLayerSet[] | undefined) => (sets || []).map((set) => ({
    name: set.name,
    mapping: set.mapping,
    reference: set.reference,
    values: set.values || [],
    indices: set.indices || [],
  }));
  return meshes.map((mesh, idx) => ({
    name: mesh.name || `mesh_${idx}`,
    // The `usemtl` name, which only the glTF route uses: it is the one target
    // that can hold what an OBJ's material library says.
    material: mesh.material,
    // Reader arrays are already owned typed arrays, so pass them through.
    positions: mesh.positions || [],
    indices: mesh.indices || [],
    normals: settings.includeNormals ? mesh.normals || [] : null,
    uvs: settings.includeUvs ? mesh.uvs || [] : null,
    // No checkbox governs colours: PLY and Draco are the only targets that can
    // hold them, and a file carrying them has nowhere else for them to go.
    colors: mesh.colors || null,
    // Carried, not read. Only the writer that produced them can put them back,
    // and it does so by the description rather than by any meaning.
    extras: (mesh.extras || []).map((extra) => ({
      ...extra,
      values: extra.values,
    })),
    // No checkbox governs colours: PLY and Draco are the only targets that can
    // hold them, and a file carrying them has nowhere else for them to go.
    controlPoints: mesh.controlPoints || null,
    polygonVertexIndices: mesh.polygonVertexIndices || null,
    uvSets: layerSets(mesh.uvSets),
    normalSets: layerSets(mesh.normalSets),
    colorSets: layerSets(mesh.colorSets),
  }));
}

export function prepareFbxSceneForExport(
  scene: FbxSceneData,
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

export async function exportToObj(
  meshes: PreparedMesh[],
  settings: Pick<ExportSettings, 'includeNormals' | 'includeUvs'>,
) {
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

export async function exportToPly(
  meshes: PreparedMesh[],
  settings: Pick<ExportSettings, 'includeNormals' | 'includeUvs'>,
) {
  if (!modules.ply.loaded) {
    return { success: false, error: 'PLY module not loaded' };
  }
  // PLY only supports single mesh, merge if multiple
  const merged = mergeMeshes(meshes);
  const options = {
    include_normals: settings.includeNormals,
    include_uvs: settings.includeUvs,
    include_colors: true,
    precision: 6,
    format: 'ascii',
  };
  return modules.ply.module.create_ply(merged, options);
}

/**
 * STL, always the binary container.
 *
 * The writer can emit ASCII and the module exposes it, but nothing in the panel
 * asks for it: ASCII STL is five times the size for the same triangles, and the
 * cases that want readable output are not the ones that go through a converter.
 */
export async function exportToStl(meshes: PreparedMesh[]) {
  if (!modules.stl.loaded) {
    return { success: false, error: 'STL module not loaded' };
  }
  const merged = mergeMeshes(meshes);
  return modules.stl.module.create_stl(
    {
      positions: new Float32Array(merged.positions),
      indices: new Uint32Array(merged.indices),
    },
    { format: 'binary', name: merged.name },
  );
}

/**
 * A standalone Draco payload — the `.drc` container, not glTF's extension.
 *
 * The quantization controls are the same ones the glTF route uses, because they
 * mean the same thing to the same encoder: this route just addresses the
 * attributes directly instead of through a primitive.
 */
export async function exportToDrc(meshes: PreparedMesh[], settings: ExportSettings) {
  if (!modules.drc.loaded) {
    return { success: false, error: 'DRC module not loaded' };
  }
  const merged = mergeMeshes(meshes);
  return modules.drc.module.create_drc(merged, {
    encoding_speed: settings.encodingSpeed,
    position_bits: settings.positionBits,
    normal_bits: settings.normalBits,
    texcoord_bits: settings.texcoordBits,
    include_normals: settings.includeNormals,
    include_uvs: settings.includeUvs,
    include_colors: true,
  });
}

/**
 * glTF out of a mesh list, through the portable document.
 *
 * A flat source has no document to re-serialize, which is why this used to
 * refuse outright: OBJ, PLY, STL and `.drc` could reach every target except the
 * one most people convert to. Building the SceneDocument first puts them on the
 * same writer FBX already uses, Draco pass included.
 *
 * JSON glTF stays out of reach, for the reason it is out of reach for a
 * compressed document too: it needs a companion `.bin` beside it, and the panel
 * downloads one file. GLB is that file.
 */
export function exportMeshesAsGltf(
  meshes: LoadedMesh[],
  settings: ExportSettings,
  options: MeshDocumentOptions = {},
): ExportOutcome {
  const document = buildSceneDocumentFromMeshes(meshes, options);
  if (document.meshes.length === 0) {
    return {
      result: { success: false, error: 'The file contains no triangle geometry to export' },
      warnings: [],
    };
  }
  const outcome = asGltfIfAsked(exportSceneDocumentToGlb(document, settings), settings.format);
  return { ...outcome, warnings: [...document.warnings, ...outcome.warnings] };
}

/**
 * The same export as JSON, when JSON is what was asked for.
 *
 * Every route that can produce glTF produces a GLB first, because that is what
 * the writer emits and what the Draco pass operates on. Turning it into JSON is
 * a container change and belongs in one place rather than in each route.
 */
function asGltfIfAsked(outcome: ExportOutcome, format: string): ExportOutcome {
  if (format !== 'gltf' || !outcome.result.success || !outcome.result.binary_data) return outcome;
  return {
    ...outcome,
    result: {
      ...outcome.result,
      binary_data: undefined,
      json_data: glbToEmbeddedGltf(new Uint8Array(outcome.result.binary_data as ArrayLike<number>)),
      message: 'Exported as JSON glTF with its binary embedded',
    },
  };
}

/**
 * FBX from a bare mesh list.
 *
 * The flat entry point writes each mesh as its own root and has no hierarchy to
 * place them with, so the geometry goes out in glTF's own coordinates. That is
 * only correct if the file says so, which is what the declaration is for: it
 * used to fall back to the writer's defaults, and those described a space this
 * path never wrote.
 */
export async function exportToFbx(meshes: PreparedMesh[], compression = false) {
  if (!modules.fbx.loaded) {
    return { success: false, error: 'FBX module not loaded' };
  }
  return modules.fbx.module.create_fbx(meshes, {
    version: 7500,
    globalSettings: FBX_METERS_Y_UP,
    compression,
  });
}

export async function exportToFbxScene(
  scene: FbxSceneData,
  legacyCompatibility = false,
  compression = false,
) {
  if (!modules.fbx.loaded) {
    return { success: false, error: 'FBX module not loaded' };
  }
  return modules.fbx.module.create_fbx_scene(scene, {
    version: 7500,
    legacyCompatibility,
    compression,
  });
}

/** Merge multiple meshes into one. */
export function mergeMeshes(meshes: PreparedMesh[]) {
  if (meshes.length === 1) return meshes[0];

  const merged = {
    name: 'merged',
    positions: [] as number[],
    indices: [] as number[],
    normals: [] as number[],
    uvs: [] as number[],
    colors: [] as number[],
    extras: [] as OpaqueAttribute[],
  };

  let vertexOffset = 0;
  // Uninterpreted attributes merge by their own description rather than by
  // position: two meshes carrying the same id and layout are one attribute of
  // the merged mesh, and two that differ stay separate. Zero is the only
  // filler available for something nobody here understands.
  const extraValues = new Map<string, number[]>();
  const describeExtra = (extra: OpaqueAttribute) =>
    [extra.type, extra.components, extra.dataType, extra.uniqueId, extra.normalized].join('/');

  // Appended one element at a time on purpose. `push(...values)` passes the
  // whole array as arguments and blows the call stack somewhere past a
  // hundred thousand of them, which a mesh reaches at roughly 40k vertices.
  const append = (into: number[], values: ArrayLike<number>) => {
    for (let index = 0; index < values.length; index += 1) into.push(values[index]);
  };
  /**
   * Append a per-vertex channel, filling in for meshes that lack it.
   *
   * A channel is addressed by vertex index, so one mesh without normals and a
   * later one with them do not concatenate: every value after the gap would
   * belong to the wrong vertex. The filler is what the format means by absent —
   * a zero normal, an origin UV, opaque white.
   */
  const appendChannel = (
    into: number[],
    values: ArrayLike<number> | null | undefined,
    stride: number,
    filler: number[],
  ) => {
    if (!values) return;
    while (into.length < vertexOffset * stride) append(into, filler);
    append(into, values);
  };

  for (const mesh of meshes) {
    const vertices = mesh.positions.length / 3;
    append(merged.positions, mesh.positions);
    if (mesh.indices) {
      for (let index = 0; index < mesh.indices.length; index += 1) {
        merged.indices.push(mesh.indices[index] + vertexOffset);
      }
    }
    appendChannel(merged.normals, mesh.normals, 3, [0, 0, 0]);
    appendChannel(merged.uvs, mesh.uvs, 2, [0, 0]);
    appendChannel(merged.colors, mesh.colors, 4, [255, 255, 255, 255]);
    for (const extra of mesh.extras || []) {
      const key = describeExtra(extra);
      let values = extraValues.get(key);
      if (!values) {
        values = [];
        extraValues.set(key, values);
        merged.extras.push({ ...extra, values });
      }
      appendChannel(values, extra.values, extra.components, Array(extra.components).fill(0));
    }
    vertexOffset += vertices;
  }

  // A channel no mesh supplied stays empty; one some meshes supplied is filled
  // out to the end so its length still matches the vertex count.
  for (const [channel, stride, filler] of [
    [merged.normals, 3, [0, 0, 0]],
    [merged.uvs, 2, [0, 0]],
    [merged.colors, 4, [255, 255, 255, 255]],
  ] as [number[], number, number[]][]) {
    if (channel.length === 0) continue;
    while (channel.length < vertexOffset * stride) append(channel, filler);
  }
  for (const extra of merged.extras) {
    const values = extra.values as number[];
    while (values.length < vertexOffset * extra.components) {
      append(values, Array(extra.components).fill(0));
    }
  }

  return merged;
}
