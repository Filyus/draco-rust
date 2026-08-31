/**
 * One line per thing that is wrong, not one per subject it is wrong about.
 *
 * A stage that walks a document raises its notice against every element it
 * walks. A point cloud of 797 meshes filled the warnings card with 797 lines
 * saying its primitives would need triangulating, and a skin missing its tail
 * reported each absent joint by number, 1116 times. Whatever else was wrong
 * with the file was somewhere in that wall.
 */
import assert from 'node:assert/strict';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

// The module reaches the panel through dom.ts, which looks its elements up at
// load. Nothing below touches them; this is only so the import resolves.
(globalThis as any).document = { getElementById: () => null, querySelector: () => null };
const { uniqueWarnings } = await import(
  pathToFileURL(resolve(here, '..', 'src', 'app', 'warnings.ts')).href
) as { uniqueWarnings(warnings: string[]): string[] };

// ---- the same notice against many subjects is one line ---------------------
{
  const walked = Array.from(
    { length: 797 },
    (_, mesh) => `meshes[${mesh}].primitives[0].mode=0 will require triangulation before glTF export`,
  );
  const shown = uniqueWarnings(walked);
  assert.deepEqual(
    shown,
    ['meshes[0].primitives[0].mode=0 will require triangulation before glTF export (x797)'],
    'the walk collapses to its first line and a count',
  );
}

// A bare ordinal names a subject the same way an index does.
{
  const clusters = [259, 260, 261].map(
    (joint) => `FBX skin cluster targets missing joint ${joint} and was omitted`,
  );
  assert.deepEqual(
    uniqueWarnings(clusters),
    ['FBX skin cluster targets missing joint 259 and was omitted (x3)'],
  );
}

// ---- a stated value is not a subject ---------------------------------------
// Two primitive modes are two different facts about the document, however many
// meshes hold each, and collapsing them would report only the first.
{
  const mixed = [
    'meshes[0].primitives[0].mode=0 will require triangulation before glTF export',
    'meshes[1].primitives[0].mode=5 will require triangulation before glTF export',
    'meshes[2].primitives[0].mode=0 will require triangulation before glTF export',
  ];
  const shown = uniqueWarnings(mixed);
  assert.equal(shown.length, 2, `modes must stay apart: ${JSON.stringify(shown)}`);
  assert.ok(shown.some((line) => line.includes('mode=0') && line.endsWith('(x2)')));
  assert.ok(shown.some((line) => line.includes('mode=5') && !line.includes('(x')));
}

// ---- what differs in words stays its own line ------------------------------
{
  const distinct = [
    'Skin hair has 340 joints, over the 256 the preview holds at once',
    'FBX model uses unsupported InheritType',
    '',
    '   ',
  ];
  assert.deepEqual(uniqueWarnings(distinct), [distinct[0], distinct[1]], 'and blank lines are dropped');
}

// ---- collapsing twice does not annotate twice ------------------------------
// The card re-runs this on its own output when the reader expands it.
{
  const once = uniqueWarnings(['a[0] fell over', 'a[1] fell over']);
  assert.deepEqual(uniqueWarnings(once), once, 'expanding the card must not restate the count');
}

console.log('warning-collapse: OK');
