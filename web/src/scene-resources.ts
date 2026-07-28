/** Shared resource and accessor helpers for the scene-document builders.
 *
 * The FBX and glTF importers each grew their own copies of these, and the
 * copies drifted: only the glTF side knew about KTX2 and `data:` URIs, and
 * only the scene-document builders stripped the directory before looking at a
 * file extension. This module is the union of those behaviours, so a fix lands
 * once instead of two or three times.
 */

/**
 * External files an importer may hand over alongside the model: the raw bytes
 * keyed by URI, by basename, or both.
 */
export type ResourceMap = Record<string, Uint8Array | ArrayBuffer | null | undefined>;

/** Final path component, for both POSIX and Windows separators. */
export function basename(path: string | null | undefined): string {
  if (!path) return '';
  const slash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return slash >= 0 ? path.slice(slash + 1) : path;
}

const MIME_BY_EXTENSION: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  webp: 'image/webp',
  avif: 'image/avif',
  ktx2: 'image/ktx2',
};

/** Reads four bytes as ASCII, for the container tags that are spelled out. */
function tagAt(bytes: Uint8Array, offset: number): string {
  if (bytes.length < offset + 4) return '';
  return String.fromCharCode(bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]);
}

/** Guesses an image MIME type from a URI's file extension. */
export function mimeFromUri(uri: string | null | undefined): string | null {
  // Strip the directory first: a path like `assets.v2/texture` would
  // otherwise take `v2/texture` as its extension.
  const extension = basename(uri || '').split('.').pop()?.toLowerCase();
  return (extension ? MIME_BY_EXTENSION[extension] : null) || null;
}

/** Identifies an image MIME type from its magic bytes. */
export function sniffMime(bytes: Uint8Array | null | undefined): string | null {
  if (!bytes || bytes.length < 2) return null;
  if (bytes.length >= 4 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return 'image/png';
  if (bytes[0] === 0xff && bytes[1] === 0xd8) return 'image/jpeg';
  // RIFF alone is the container WAV and AVI also use, so the form tag decides.
  if (tagAt(bytes, 0) === 'RIFF' && tagAt(bytes, 8) === 'WEBP') return 'image/webp';
  // AVIF is ISOBMFF: a box whose type is `ftyp`, then the brand. HEIC is the
  // same container with a different brand, which is why the brand is read
  // rather than the box type taken as an answer. `avis` is a sequence; the
  // extension permits one and a decoder shows its first frame.
  if (tagAt(bytes, 4) === 'ftyp' && ['avif', 'avis'].includes(tagAt(bytes, 8))) return 'image/avif';
  if (bytes.length >= 4 && bytes[0] === 0xab && bytes[1] === 0x4b && bytes[2] === 0x54 && bytes[3] === 0x58) return 'image/ktx2';
  return null;
}

/** Decodes a `data:` URI payload, base64 or percent-encoded. */
export function decodeDataUri(uri: string): Uint8Array | null {
  const comma = uri.indexOf(',');
  if (comma < 0) return null;
  const meta = uri.substring(0, comma);
  const payload = uri.substring(comma + 1);
  try {
    if (meta.includes(';base64')) {
      const decoded = atob(payload);
      return Uint8Array.from(decoded, (character) => character.charCodeAt(0));
    }
    return new TextEncoder().encode(decodeURIComponent(payload));
  } catch {
    return null;
  }
}

/**
 * Resolves a URI to bytes from an embedded `data:` payload or a supplied
 * resource map, matching either the full URI or its basename.
 */
export function resolveResource(
  uri: string | null | undefined,
  resources?: ResourceMap | null,
): Uint8Array | null {
  if (!uri) return null;
  if (uri.startsWith('data:')) return decodeDataUri(uri);
  const value = resources?.[uri] || resources?.[basename(uri)];
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  return value || null;
}

export function bytesFromF32(values: ArrayLike<number>): Uint8Array {
  return new Uint8Array(Float32Array.from(values).buffer);
}

export function bytesFromU16(values: ArrayLike<number>): Uint8Array {
  return new Uint8Array(Uint16Array.from(values).buffer);
}

export function bytesFromU32(values: ArrayLike<number>): Uint8Array {
  return new Uint8Array(Uint32Array.from(values).buffer);
}

/** Appends an accessor to a SceneDocument and returns its index. */
export function appendAccessor<T>(document: { accessors: T[] }, accessor: T): number {
  const index = document.accessors.length;
  document.accessors.push(accessor);
  return index;
}
