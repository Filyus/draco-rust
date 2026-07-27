/**
 * What the file asked for, and what each route can do with it.
 *
 * Every part of this was already known and each part was said somewhere else:
 * the preview warns about what it cannot shade, the document warns about what
 * the portable form could not take, and each exporter warns about what its
 * format cannot express. Read one at a time they answer "is something wrong
 * here"; read together they answer the question a user actually has, which is
 * what happens to *this* file.
 *
 * So this states it once, per extension, in the order the answers get worse:
 * carried everywhere, shown but not exportable, read but not shown, not
 * understood at all.
 */

import { GLTF_INTERPRETED_EXTENSIONS, GLTF_READER_RESOLVED_EXTENSIONS } from '../gltf-interpretation.ts';
import { MATERIAL_EXTENSION_NAMES } from '../material-extensions.ts';
import type { GltfSceneProvenance } from '../gltf-scene-provenance.ts';

/** How far one extension got, worst last. */
export type ExtensionReach =
  /** Read, shown, and written back out by the glTF exporters. */
  | 'carried'
  /** Read and shown, but no exporter other than glTF can state it. */
  | 'gltf-only'
  /** Read into the document, but the preview does not shade it. */
  | 'not-shown'
  /** Not acted on anywhere; the file's own claim is all that is left of it. */
  | 'ignored';

export interface ExtensionOutcome {
  name: string;
  reach: ExtensionReach;
  /** Whether the file said a reader may not skip it. */
  required: boolean;
}

/**
 * Extensions the reader resolves before anything downstream sees them.
 *
 * A decoded Draco payload is ordinary geometry by the time it reaches the
 * document, so the answer for these is "carried" whatever any consumer
 * believes about the name.
 */
const RESOLVED = GLTF_READER_RESOLVED_EXTENSIONS;

/**
 * What only glTF can carry back out.
 *
 * The layered material extensions and the two scene ones are read and shown,
 * but the OBJ, PLY and FBX writers have no way to state them, so a user
 * exporting to those formats loses exactly this list.
 */
const GLTF_ONLY = new Set<string>([
  ...MATERIAL_EXTENSION_NAMES,
  'KHR_texture_transform',
  'KHR_lights_punctual',
  'KHR_materials_variants',
  'EXT_mesh_gpu_instancing',
]);

/**
 * Judge every extension the file declared.
 *
 * The provenance is the file's own claim, which is the only place the full
 * list survives: the document deliberately keeps no record of what it dropped
 * beyond a warning, and the preview never sees the manifest at all.
 */
export function reportExtensionReach(provenance: Partial<GltfSceneProvenance> | null): ExtensionOutcome[] {
  const required = new Set(provenance?.extensionsRequired ?? []);
  return (provenance?.extensionsUsed ?? []).map((name) => ({
    name,
    required: required.has(name),
    reach: reachOf(name),
  }));
}

function reachOf(name: string): ExtensionReach {
  if (RESOLVED.has(name)) return 'carried';
  if (GLTF_ONLY.has(name)) return 'gltf-only';
  // Interpreted but neither resolved nor in the glTF-only list means something
  // reads it without the preview showing it - the case worth naming, because
  // it is the one a user cannot see for themselves.
  if (GLTF_INTERPRETED_EXTENSIONS.has(name)) return 'not-shown';
  return 'ignored';
}

/** One line per outcome, in the order a reader cares about them. */
const REACH_ORDER: ExtensionReach[] = ['ignored', 'not-shown', 'gltf-only', 'carried'];

const REACH_WORDING: Record<ExtensionReach, string> = {
  ignored: 'not understood: neither shown nor exported',
  'not-shown': 'read and exported, but not shown in the preview',
  'gltf-only': 'shown and exported to glTF; OBJ, PLY and FBX cannot state it',
  carried: 'carried through every route',
};

/**
 * The report as sentences, worst first.
 *
 * Empty when the file claimed nothing, which is most files: a summary that
 * says "nothing to report" about a plain glTF is noise.
 */
export function describeExtensionReach(outcomes: ExtensionOutcome[]): string[] {
  return REACH_ORDER.flatMap((reach) => {
    const named = outcomes.filter((outcome) => outcome.reach === reach);
    if (named.length === 0) return [];
    const names = named
      .map((outcome) => (outcome.required ? `${outcome.name} (required)` : outcome.name))
      .join(', ');
    return [`${names} — ${REACH_WORDING[reach]}`];
  });
}
