/**
 * What the summary tells a user about the extensions their file declared.
 *
 * Every part of this was already knowable and each part was said somewhere
 * else: the preview warns about what it cannot shade, the document about what
 * the portable form could not take, each exporter about what its format cannot
 * state. Read one at a time they answer "is something wrong"; the question a
 * user has is what happens to *this* file, and that answer had no home.
 *
 * The gate is here rather than in the browser because the judgement is not a
 * rendering one: it reads the three lists the code already keeps and sorts a
 * declared name into one of four outcomes.
 */
import assert from 'node:assert/strict';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import type {
  describeExtensionReach as DescribeExtensionReach,
  ExtensionOutcome,
  reportExtensionReach as ReportExtensionReach,
} from '../src/app/extension-report.ts';
import type {
  GLTF_INTERPRETED_EXTENSIONS as InterpretedExtensions,
  GLTF_READER_RESOLVED_EXTENSIONS as ReaderResolvedExtensions,
} from '../src/gltf-interpretation.ts';

const here = dirname(fileURLToPath(import.meta.url));
const source = (...parts: string[]) => pathToFileURL(resolve(here, '..', 'src', ...parts)).href;

const { describeExtensionReach, reportExtensionReach } = await import(source('app', 'extension-report.ts')) as {
  describeExtensionReach: typeof DescribeExtensionReach;
  reportExtensionReach: typeof ReportExtensionReach;
};
const { GLTF_INTERPRETED_EXTENSIONS, GLTF_READER_RESOLVED_EXTENSIONS } = await import(
  source('gltf-interpretation.ts')
) as {
  GLTF_INTERPRETED_EXTENSIONS: typeof InterpretedExtensions;
  GLTF_READER_RESOLVED_EXTENSIONS: typeof ReaderResolvedExtensions;
};

const reachOf = (outcomes: ExtensionOutcome[], name: string) => outcomes.find((outcome) => outcome.name === name)?.reach;

const outcomes = reportExtensionReach({
  extensionsUsed: [
    'KHR_draco_mesh_compression',
    'KHR_materials_clearcoat',
    'KHR_lights_punctual',
    'KHR_materials_pbrSpecularGlossiness',
  ],
  extensionsRequired: ['KHR_draco_mesh_compression'],
});

// Decoded before anything downstream sees it, so nothing is lost anywhere.
assert.equal(reachOf(outcomes, 'KHR_draco_mesh_compression'), 'carried');
// Read and shown, but only glTF can state it again.
assert.equal(reachOf(outcomes, 'KHR_materials_clearcoat'), 'gltf-only');
assert.equal(reachOf(outcomes, 'KHR_lights_punctual'), 'gltf-only');
// Nothing here interprets it — and it is still exported, because the glTF
// route rewrites the asset in place rather than rebuilding it from what was
// understood. Calling that "ignored" was the report's one false statement.
assert.equal(reachOf(outcomes, 'KHR_materials_pbrSpecularGlossiness'), 'gltf-verbatim');

// What the file said a reader may not skip is carried through, because that is
// the difference between "this export is poorer" and "this export is wrong".
assert.equal(outcomes.find((outcome) => outcome.name === 'KHR_draco_mesh_compression')!.required, true);
assert.equal(outcomes.find((outcome) => outcome.name === 'KHR_materials_clearcoat')!.required, false);

// Every interpreted extension has to land somewhere better, or the report
// contradicts the list the readers act on.
for (const name of GLTF_INTERPRETED_EXTENSIONS) {
  const [outcome] = reportExtensionReach({ extensionsUsed: [name] });
  assert.notEqual(outcome.reach, 'gltf-verbatim', `${name} is interpreted, so the report must not call it un-understood`);
}
for (const name of GLTF_READER_RESOLVED_EXTENSIONS) {
  const [outcome] = reportExtensionReach({ extensionsUsed: [name] });
  assert.equal(outcome.reach, 'carried', `${name} is resolved by the reader, so nothing downstream loses it`);
}

// Worst first: a user reading one line should read the one that matters.
const lines = describeExtensionReach(outcomes);
assert.equal(lines.length, 3, 'one line per outcome that occurred, not per extension');
assert.match(lines[0], /pbrSpecularGlossiness/);
assert.match(lines[0], /not understood/);
// The half that was wrong before: the file's own claim is not all that is left
// of it, and the line has to say where it does survive.
assert.match(lines[0], /copied unchanged into exported glTF/);
assert.doesNotMatch(lines[0], /neither shown nor exported/);
assert.match(lines[1], /clearcoat.*lights_punctual|lights_punctual.*clearcoat/);
assert.match(lines[2], /draco.*\(required\)/);

// The alternate image codecs are read and shown, so the report may not say
// they are not understood. That was true of KHR_texture_basisu until the
// transcoder existed, and saying it now would send a user looking for a
// problem that is no longer there.
for (const name of ['KHR_texture_basisu', 'EXT_texture_webp', 'EXT_texture_avif']) {
  const [outcome] = reportExtensionReach({ extensionsUsed: [name] });
  assert.equal(
    outcome.reach,
    'gltf-only',
    `${name} names an image codec this converter decodes and shows, and which only glTF can carry back out`,
  );
}

// A file that claimed nothing gets no section at all: "nothing to report" about
// a plain glTF is noise.
assert.deepEqual(describeExtensionReach(reportExtensionReach(null)), []);
assert.deepEqual(describeExtensionReach(reportExtensionReach({ extensionsUsed: [] })), []);

console.log('extension-reach: OK');
