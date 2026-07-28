/**
 * Turning a drop into a list of paths, whether a file or a folder was dropped.
 *
 * `dataTransfer.files` holds nothing for a dropped folder — not the files
 * inside it, not the folder itself — so a browser that offers
 * `webkitGetAsEntry` is asked for the filesystem entry instead and the tree is
 * walked. Only names and handles come back; nothing is read here.
 *
 * The one-jesture rule this exists for: drop a file and a file opens, drop a
 * folder and its contents become the selection. There is no mode to choose,
 * which is why the folder path is worth this much code rather than a second
 * button.
 */
import type { IntakeEntry } from './model-intake.ts';

/** The `FileSystemEntry` surface used here, which TypeScript's DOM lib omits. */
interface DirectoryEntry {
  isFile: boolean;
  isDirectory: boolean;
  name: string;
  file(onSuccess: (file: File) => void, onError: (error: unknown) => void): void;
  createReader(): {
    readEntries(
      onSuccess: (entries: DirectoryEntry[]) => void,
      onError: (error: unknown) => void,
    ): void;
  };
}

/**
 * Above this many files a dropped folder is refused rather than walked.
 *
 * Not a memory limit — nothing is read — but a limit on how long the walk may
 * take and how long a picker may get. Someone who drops their home directory
 * should be told so rather than watched.
 */
const MAX_ENTRIES = 20_000;

/**
 * One `readEntries` call, promisified.
 *
 * The reader hands back a batch at a time and signals the end with an empty
 * one, so a directory of a thousand needs several calls. Reading it once — the
 * obvious mistake — silently truncates to whatever the first batch held, which
 * in Chrome is a hundred.
 */
function readBatch(reader: ReturnType<DirectoryEntry['createReader']>): Promise<DirectoryEntry[]> {
  return new Promise((resolve) => reader.readEntries(resolve, () => resolve([])));
}

function readFileOf(entry: DirectoryEntry): Promise<File | null> {
  return new Promise((resolve) => entry.file(resolve, () => resolve(null)));
}

async function walk(entry: DirectoryEntry, prefix: string, collected: IntakeEntry[]): Promise<void> {
  if (collected.length >= MAX_ENTRIES) return;
  const path = prefix ? `${prefix}/${entry.name}` : entry.name;
  if (entry.isFile) {
    const file = await readFileOf(entry);
    if (file) collected.push({ path, file });
    return;
  }
  if (!entry.isDirectory) return;
  const reader = entry.createReader();
  for (;;) {
    const batch = await readBatch(reader);
    if (batch.length === 0) return;
    for (const child of batch) await walk(child, path, collected);
    if (collected.length >= MAX_ENTRIES) return;
  }
}

/**
 * What was dropped, as root-relative paths.
 *
 * A drop of several items — two folders, or a file beside a folder — keeps
 * each item's own name as the first path segment, so two files called
 * `scene.bin` under different roots stay distinct.
 */
export async function entriesFromDataTransfer(transfer: DataTransfer): Promise<IntakeEntry[]> {
  // Asked for before anything is awaited: the item list is emptied as soon as
  // the drop handler yields, so a `webkitGetAsEntry` after the first `await`
  // returns null for everything.
  const roots: DirectoryEntry[] = [];
  for (const item of Array.from(transfer.items ?? [])) {
    const entry = item.webkitGetAsEntry?.();
    if (entry) roots.push(entry as unknown as DirectoryEntry);
  }

  if (roots.length === 0) {
    // No filesystem entries on offer: whatever `files` holds is all there is,
    // and for a plain file drop that is the whole answer.
    return Array.from(transfer.files ?? [], (file) => ({ path: file.name, file }));
  }

  const collected: IntakeEntry[] = [];
  // Each root keeps its own name as the first segment, so a dropped file is
  // just its filename and a dropped folder carries the folder in front of
  // everything inside it.
  for (const root of roots) await walk(root, '', collected);
  return collected;
}

export { MAX_ENTRIES };
