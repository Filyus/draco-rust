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
export const MODEL_EXTENSIONS = ['gltf', 'glb', 'obj', 'ply', 'stl', 'drc', 'fbx'] as const;

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
 * The file name at the end of a path, under either separator.
 *
 * Selection paths are slash-separated, but an FBX quotes the authoring
 * machine's path verbatim, and that machine was usually Windows.
 */
function baseNameOf(path: string): string {
  const slash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return slash < 0 ? path : path.slice(slash + 1);
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

/** The spellings an FBX uses for the path property of a texture or video. */
const FBX_FILENAME_MARKERS = ['RelativeFilename', 'FileName', 'Filename'];

/**
 * What an FBX file reference has to end in to be worth going looking for.
 *
 * The scan below reads the property wherever it appears, and a `FileName` also
 * hangs off things that are not textures — an animation take carries one. The
 * extension is what separates them, and getting it wrong costs a reported
 * missing file that was never a file.
 */
const FBX_TEXTURE_EXTENSIONS = new Set([
  'png', 'jpg', 'jpeg', 'webp', 'avif', 'ktx2', 'bmp', 'gif', 'tga', 'tif', 'tiff', 'dds', 'exr',
]);

/**
 * Every texture path an FBX names.
 *
 * Scanned rather than parsed. Intake runs before the WASM reader, and these are
 * plain string properties held uncompressed, so they are legible without
 * touching the geometry: a binary node writes its property list straight after
 * its name -- the tag `S`, a little-endian length, then the text -- and an
 * ASCII node writes `FileName: "..."`.
 *
 * The paths are usually the authoring machine's own absolute ones and point
 * nowhere on the machine reading the file, so the caller matches them by file
 * name. That is what every other FBX importer does with them.
 */
function fbxTextureNames(data: Uint8Array): string[] {
  // Latin-1 keeps one character per byte, so offsets found in the string are
  // offsets into the bytes and the search itself is the engine's, not ours.
  const text = new TextDecoder('latin1').decode(data);
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
  const names: string[] = [];
  for (const marker of FBX_FILENAME_MARKERS) {
    for (let at = text.indexOf(marker); at >= 0; at = text.indexOf(marker, at + 1)) {
      const start = at + marker.length;
      let name = '';
      if (text.charCodeAt(start) === 0x53 && start + 5 <= data.length) {
        const length = view.getUint32(start + 1, true);
        if (length > 0 && length <= 4096 && start + 5 + length <= data.length) {
          name = new TextDecoder().decode(data.subarray(start + 5, start + 5 + length));
        }
      } else {
        const open = text.indexOf('"', start);
        const close = open < 0 ? -1 : text.indexOf('"', open + 1);
        // Both quotes have to sit on this property's own line, or the match was
        // a node whose name merely ends in one of the markers.
        if (open >= 0 && close > open && !text.slice(start, open).includes('\n')) {
          name = text.slice(open + 1, close);
        }
      }
      if (FBX_TEXTURE_EXTENSIONS.has(extensionOf(name)) && !names.includes(name)) names.push(name);
    }
  }
  return names;
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
 *
 * FBX names its textures too, but by the authoring machine's absolute path, so
 * they are matched on the file name alone. Still only what the model named.
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
  } else if (extensionOf(model.path) === 'fbx') {
    // An FBX names its textures by a path from the machine that authored it, so
    // the directory is worthless and the file name is all that survives. Match
    // on that, against the selection as supplied.
    const byName = new Map<string, IntakeEntry>();
    for (const entry of entries) {
      const key = baseNameOf(entry.path);
      if (!byName.has(key)) byName.set(key, entry);
    }
    for (const named of fbxTextureNames(data)) {
      if (named in resources) continue;
      const entry = byName.get(baseNameOf(named));
      if (!entry) {
        if (!missing.includes(named)) missing.push(named);
        continue;
      }
      const bytes = new Uint8Array(await entry.file.arrayBuffer());
      // Under both spellings: the reader reports whichever of the property
      // names it met first, and the two need not agree inside one file.
      resources[named] = bytes;
      resources[baseNameOf(named)] = bytes;
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
