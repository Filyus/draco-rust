/**
 * What was handed to the shell, and what of it has to be read.
 *
 * A selection used to be read whole: every file in it went through
 * `arrayBuffer()` and landed in the resource map under its bare name. That was
 * affordable while a selection was a handful of files a person picked by hand,
 * and it stops being affordable the moment a folder can be dropped — the point
 * of a folder is that nobody counted what is in it.
 *
 * So the order is inverted here. The model is read first, it says which URIs it
 * needs, and only those are fetched. A `File` is a handle rather than bytes, so
 * a folder of ten thousand costs ten thousand names and the size of the model.
 *
 * Keyed by the URI exactly as the document wrote it, which is also the fix for
 * a second thing: the resolver on the Rust side looks the URI up as-is, so a
 * document naming `textures/wood.png` or `../glTF/mesh.bin` never found it
 * under a bare filename however the file was supplied.
 */

/** The subset of `File` this module needs, so a test can stand in for one. */
export interface IntakeFile {
  name: string;
  size: number;
  arrayBuffer(): Promise<ArrayBuffer>;
}

/** One supplied file and where it sits relative to the root of the selection. */
export interface IntakeEntry {
  /** Slash-separated and root-relative; a bare name when there is no folder. */
  path: string;
  file: IntakeFile;
}

/** What the shell can open as a scene, as opposed to carry as a companion. */
export const MODEL_EXTENSIONS = ['gltf', 'glb', 'obj', 'ply', 'fbx'] as const;

export interface IntakeResult {
  data: Uint8Array;
  resources: Record<string, Uint8Array>;
  /** URIs the document named and the selection did not contain. */
  missing: string[];
}

function extensionOf(path: string): string {
  return path.split('.').pop()!.toLowerCase();
}

function directoryOf(path: string): string {
  const slash = path.lastIndexOf('/');
  return slash < 0 ? '' : path.slice(0, slash);
}

/**
 * Where a URI written inside a document at `directory` actually points.
 *
 * `..` is followed rather than refused: a model in `glTF-instancing/` naming
 * `../glTF/mesh.bin` is ordinary, and the only reason it ever looked suspicious
 * is that a file picker could not supply the sibling folder. Climbing past the
 * root is a different matter — nothing above it was offered, so there is
 * nothing to find and the answer is that the resource is missing.
 */
export function resolveUriPath(directory: string, uri: string): string | null {
  const segments = directory ? directory.split('/') : [];
  for (const segment of decodeURIComponent(uri).split('/')) {
    if (segment === '' || segment === '.') continue;
    if (segment !== '..') segments.push(segment);
    else if (segments.length > 0) segments.pop();
    else return null;
  }
  return segments.join('/');
}

/** The JSON of a `.gltf`, or of a GLB's first chunk. */
function documentJson(data: Uint8Array): Record<string, unknown> | null {
  let json = data;
  if (data.length >= 20 && String.fromCharCode(data[0], data[1], data[2], data[3]) === 'glTF') {
    const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    json = data.subarray(20, 20 + view.getUint32(12, true));
  }
  try {
    return JSON.parse(new TextDecoder().decode(json));
  } catch {
    return null;
  }
}

/**
 * A `map_*` filename, past the flags the format allows in front of it.
 *
 * The filename may itself contain spaces, so the flags are skipped by name and
 * arity rather than by taking the last token.
 */
export function mtlMapPath(values: string[]): string {
  const optionValues = {
    '-blendu': 1, '-blendv': 1, '-cc': 1, '-clamp': 1, '-texres': 1,
    '-bm': 1, '-imfchan': 1, '-type': 1, '-mm': 2, '-o': 3, '-s': 3, '-t': 3,
  };
  let index = 0;
  while (index < values.length && values[index].startsWith('-')) {
    index += 1 + (optionValues[values[index].toLowerCase() as keyof typeof optionValues] ?? 0);
  }
  return values.slice(index).join(' ').trim();
}

/** Every `mtllib` an OBJ names. */
function objLibraries(text: string): string[] {
  const libraries: string[] = [];
  for (const line of text.split(/\r\n|[\r\n]/)) {
    const match = line.trim().match(/^mtllib\s+(.+)$/i);
    if (match) libraries.push(match[1].trim());
  }
  return libraries;
}

/** Every texture an MTL names, under any map directive. */
function mtlTextures(text: string): string[] {
  const textures: string[] = [];
  for (const line of text.split(/\r\n|[\r\n]/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;
    const [directive, ...values] = trimmed.split(/\s+/);
    if (!/^(map_\w+|bump|disp|decal|refl)$/i.test(directive)) continue;
    const texture = mtlMapPath(values);
    if (texture) textures.push(texture);
  }
  return textures;
}

/**
 * The models in a selection, most likely first.
 *
 * Shallowest path wins, then shortest, then alphabetical. Nothing is opened to
 * decide this — the extension is the whole test — and the ordering is only a
 * default: a folder holding one scene several ways (`glTF/`, `glTF-Binary/`,
 * `glTF-Draco/`) puts the plain one first, which is the reference variant, and
 * the rest stay one click away.
 */
export function findModels(entries: IntakeEntry[]): IntakeEntry[] {
  const depth = (path: string) => path.split('/').length;
  return entries
    .filter((entry) => (MODEL_EXTENSIONS as readonly string[]).includes(extensionOf(entry.path)))
    .sort((left, right) => depth(left.path) - depth(right.path)
      || left.path.length - right.path.length
      || left.path.localeCompare(right.path));
}

/**
 * Read one model and exactly the companions it names.
 *
 * OBJ needs two rounds rather than one: the model names material libraries and
 * the libraries name textures, so nothing about the images is knowable until
 * the `.mtl` files themselves have been read. They are a few kilobytes, and
 * they are the only files opened speculatively.
 */
export async function readModel(model: IntakeEntry, entries: IntakeEntry[]): Promise<IntakeResult> {
  const byPath = new Map(entries.map((entry) => [entry.path, entry]));
  const directory = directoryOf(model.path);
  const data = new Uint8Array(await model.file.arrayBuffer());
  const resources: Record<string, Uint8Array> = Object.create(null);
  const missing: string[] = [];

  /** @returns The bytes, or null once the URI has been recorded as missing. */
  const take = async (uri: string): Promise<Uint8Array | null> => {
    if (!uri || uri.startsWith('data:')) return null;
    if (uri in resources) return resources[uri];
    const path = resolveUriPath(directory, uri);
    const entry = path === null ? undefined : byPath.get(path);
    if (!entry) {
      if (!missing.includes(uri)) missing.push(uri);
      return null;
    }
    const bytes = new Uint8Array(await entry.file.arrayBuffer());
    resources[uri] = bytes;
    return bytes;
  };

  if (extensionOf(model.path) === 'obj') {
    const text = new TextDecoder().decode(data);
    for (const library of objLibraries(text)) {
      const bytes = await take(library);
      if (!bytes) continue;
      // Texture paths in an MTL are written relative to the MTL, which is
      // usually but not always the directory the OBJ is in.
      const base = directoryOf(resolveUriPath(directory, library) ?? '');
      for (const texture of mtlTextures(new TextDecoder().decode(bytes))) {
        const path = resolveUriPath(base, texture);
        const entry = path === null ? undefined : byPath.get(path);
        if (entry) resources[texture] = new Uint8Array(await entry.file.arrayBuffer());
        else if (!missing.includes(texture)) missing.push(texture);
      }
    }
  } else {
    const document = documentJson(data);
    const named = [...(document?.buffers as { uri?: string }[] ?? []),
      ...(document?.images as { uri?: string }[] ?? [])];
    for (const entry of named) {
      if (typeof entry?.uri === 'string') await take(entry.uri);
    }
  }

  return { data, resources, missing };
}
