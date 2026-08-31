import {
  sceneWarningCount,
  sceneWarningList,
  sceneWarnings,
  sceneWarningsSection,
} from './dom.ts';

/**
 * The single warnings card.
 *
 * Warnings arrive from independent stages — scene import, mesh preparation,
 * preview, export — so each owns one slot and the panel always shows their
 * union. No stage can wipe another's.
 */

const WARNING_PREVIEW_COUNT = 8;

// Renders a numbered list; the overflow row is a real button that reveals the rest.
export function setWarningList(list: HTMLElement, warnings: string[], limit = WARNING_PREVIEW_COUNT) {
  list.replaceChildren();
  const unique = uniqueWarnings(warnings);
  const shown = Number.isFinite(limit) ? unique.slice(0, limit) : unique;
  shown.forEach((warning, index) => {
    const item = document.createElement('li');
    item.className = 'scene-warning-item';
    const marker = document.createElement('span');
    marker.className = 'scene-warning-index';
    marker.setAttribute('aria-hidden', 'true');
    marker.textContent = String(index + 1);
    const text = document.createElement('span');
    text.className = 'scene-warning-text';
    text.textContent = warning;
    item.append(marker, text);
    list.appendChild(item);
  });
  const hidden = unique.length - shown.length;
  if (hidden > 0) {
    const item = document.createElement('li');
    item.className = 'scene-warning-item scene-warning-more';
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'scene-warning-more-button';
    button.textContent = `Show ${hidden} more`;
    button.addEventListener('click', () => setWarningList(list, unique, Number.POSITIVE_INFINITY));
    item.appendChild(button);
    list.appendChild(item);
  }
  return unique;
}

/**
 * The warnings worth showing, one line per thing that is wrong.
 *
 * Identical text has always collapsed. What did not was the same notice raised
 * against a different subject, and a stage that walks a document raises exactly
 * that: a point cloud of 797 meshes filled the card with 797 lines saying its
 * primitives would need triangulating, and a skin whose joints went missing
 * reported each of them by number, 1116 times. The card is then a wall, and the
 * one notice that is not a repeat is lost in it.
 *
 * So the subject is what a line is allowed to differ by, and a subject is a
 * number that names one: an index in brackets, or a bare figure like a joint's
 * ordinal. A number written after `=` is not a subject but a stated value --
 * `mode=0` and `mode=5` are two different facts and stay two lines. Warnings
 * that agree once their subjects are blanked are one notice: the first is
 * shown, the rest become a count after it.
 */
export function uniqueWarnings(warnings: string[]) {
  const groups = new Map<string, { text: string; count: number }>();
  for (const warning of warnings) {
    if (typeof warning !== 'string' || !warning.trim()) continue;
    const key = warning.replace(/\[\d+\]/g, '[]').replace(/(?<![=\d])\d+/g, '#');
    const group = groups.get(key);
    if (group) group.count += 1;
    else groups.set(key, { text: warning, count: 1 });
  }
  return [...groups.values()].map(({ text, count }) => (count > 1 ? `${text} (x${count})` : text));
}

// Warnings arrive from independent stages, so each stage owns one slot and the
// panel always shows their union — no stage can wipe another's warnings.
const warningSources: Record<'scene' | 'mesh' | 'preview' | 'export', string[]> =
  { scene: [], mesh: [], preview: [], export: [] };

type WarningSource = keyof typeof warningSources;

export function setWarningSource(source: WarningSource, warnings: string[]) {
  warningSources[source] = uniqueWarnings(warnings || []);
  setWarningPanel([
    ...warningSources.scene,
    ...warningSources.mesh,
    ...warningSources.preview,
    ...warningSources.export,
  ]);
}

export function clearWarningPanel() {
  for (const source of Object.keys(warningSources) as WarningSource[]) warningSources[source] = [];
  setWarningPanel([]);
}

// The warnings live in their own collapsible panel so the scene tree stays readable.
// Collapsed state still advertises how many warnings are waiting.
export function setWarningPanel(warnings: string[]) {
  if (!sceneWarnings) return;
  const unique = setWarningList(sceneWarningList, warnings);
  const total = unique.length;
  sceneWarningCount.textContent = String(total);
  if (sceneWarningsSection) sceneWarningsSection.hidden = total === 0;
  if (total === 0) sceneWarnings.open = false;
}
