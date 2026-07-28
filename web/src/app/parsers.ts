import type { ResourceMap } from '../scene-resources.ts';
import { basename } from '../scene-resources.ts';
import { debugLog } from './log.ts';
import { mtlMapPath } from './model-intake.ts';
import { modules, state } from './state.ts';

/**
 * Reading a dropped file into whatever its format's module returns.
 *
 * Each parser owns its format's quirks — MTL lookup for OBJ, container and
 * companion resolution for glTF, semantic scene extraction for FBX — and hands
 * back the parse result the rest of the shell carries in state.
 */

// Parse OBJ file
export async function parseObjFile(data: Uint8Array, resources = Object.create(null)) {
  if (!modules.obj.loaded) {
    return { success: false, error: 'OBJ module not loaded' };
  }

  const textContent = new TextDecoder().decode(data);
  const result = modules.obj.module.parse_obj(textContent);
  if (result?.success) {
    result.materials = parseObjMaterials(textContent, resources, result.warnings || (result.warnings = []));
  }
  return result;
}

export function parseObjMaterials(objText: string, resources: ResourceMap, warnings: string[]) {
  const materials = Object.create(null);
  const libraries = [];
  for (const line of objText.split(/\r\n|[\r\n]/)) {
    const match = line.trim().match(/^mtllib\s+(.+)$/i);
    if (match) libraries.push(match[1].trim());
  }
  for (const library of libraries) {
    const bytes = resources[library] || resources[basename(library)];
    if (!bytes) {
      warnings.push(`OBJ material library not selected: ${library}`);
      continue;
    }
    let current = null;
    const text = new TextDecoder().decode(bytes);
    for (const line of text.split(/\r\n|[\r\n]/)) {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith('#')) continue;
      const [directive, ...values] = trimmed.split(/\s+/);
      if (directive === 'newmtl') {
        current = values.join(' ');
        if (current) materials[current] ||= { diffuse: [1, 1, 1], alpha: 1 };
      } else if (directive === 'Kd' && current && values.length >= 3) {
        const diffuse = values.slice(0, 3).map(Number);
        if (diffuse.every(Number.isFinite)) materials[current].diffuse = diffuse;
      } else if ((directive === 'd' || directive === 'Tr') && current && values.length >= 1) {
        const value = Number(values[0]);
        if (Number.isFinite(value)) materials[current].alpha = directive === 'Tr' ? 1 - value : value;
      } else if (directive.toLowerCase() === 'map_kd' && current) {
        const texture = mtlMapPath(values);
        if (texture) materials[current].baseColorTextureUri = texture;
      }
    }
  }
  return materials;
}

// One implementation, in the module that also decides which textures have to be
// read: a second copy here would be the same rule written twice, and the two
// would answer differently the first time either was corrected.
export { mtlMapPath } from './model-intake.ts';

// Parse PLY file
export async function parsePlyFile(data: Uint8Array) {
  if (!modules.ply.loaded) {
    return { success: false, error: 'PLY module not loaded' };
  }

  const result = modules.ply.module.parse_ply_bytes(data);
  debugLog('PLY parse result:', result);
  if (result.meshes) {
    for (const mesh of result.meshes) {
      debugLog('PLY mesh: positions=', mesh.positions?.length,
        ', indices=', mesh.indices?.length,
        ', normals=', mesh.normals?.length);
    }
  }
  return result;
}

/**
 * Parse a glTF/GLB file for the summary panel.
 *
 * Only the container-level summary comes from here. The geometry figures are
 * counted from the SceneDocument by `summarizeSceneDocumentGeometry`, because
 * that document is built from the same bytes moments later anyway and counting
 * them here meant opening — and fully walking — the asset a second time for
 * numbers already in hand.
 */
export async function parseGltfFile(data: Uint8Array, extension: string) {
  if (!modules.gltf.loaded) {
    return { success: false, error: 'glTF module not loaded' };
  }

  const summary = modules.gltf.module.inspect_gltf(data);
  return { ...summary, document: true, format: extension };
}

// Parse FBX file
export async function parseFbxFile(data: Uint8Array) {
  // ASCII and binary both go through the WASM reader now; it produces the
  // same node tree from either container, so the regex fallback that used to
  // scrape ASCII geometry -- without transforms, materials, skins or
  // animation -- is gone.
  if (!modules.fbx.loaded) {
    return { success: false, error: 'FBX module not loaded' };
  }

  return modules.fbx.module.parse_fbx(data);
}
