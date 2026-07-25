/** Shared resource and accessor helpers for the scene-document builders.
 *
 * The FBX and glTF importers each grew their own copies of these, and the
 * copies drifted: only the glTF side knew about KTX2 and `data:` URIs, and
 * only the scene-document builders stripped the directory before looking at a
 * file extension. This module is the union of those behaviours, so a fix lands
 * once instead of two or three times.
 */

/** Final path component, for both POSIX and Windows separators. */
export function basename(path) {
    if (!path) return '';
    const slash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    return slash >= 0 ? path.slice(slash + 1) : path;
}

const MIME_BY_EXTENSION = {
    png: 'image/png',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    webp: 'image/webp',
    ktx2: 'image/ktx2',
};

/** Guesses an image MIME type from a URI's file extension. */
export function mimeFromUri(uri) {
    // Strip the directory first: a path like `assets.v2/texture` would
    // otherwise take `v2/texture` as its extension.
    const extension = basename(uri || '').split('.').pop()?.toLowerCase();
    return MIME_BY_EXTENSION[extension] || null;
}

/** Identifies an image MIME type from its magic bytes. */
export function sniffMime(bytes) {
    if (!bytes || bytes.length < 2) return null;
    if (bytes.length >= 4 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return 'image/png';
    if (bytes[0] === 0xff && bytes[1] === 0xd8) return 'image/jpeg';
    if (bytes.length >= 4 && bytes[0] === 0x52 && bytes[1] === 0x49 && bytes[2] === 0x46 && bytes[3] === 0x46) return 'image/webp';
    if (bytes.length >= 4 && bytes[0] === 0xab && bytes[1] === 0x4b && bytes[2] === 0x54 && bytes[3] === 0x58) return 'image/ktx2';
    return null;
}

/** Decodes a `data:` URI payload, base64 or percent-encoded. */
export function decodeDataUri(uri) {
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
export function resolveResource(uri, resources) {
    if (!uri) return null;
    if (uri.startsWith('data:')) return decodeDataUri(uri);
    const value = resources?.[uri] || resources?.[basename(uri)];
    if (value instanceof Uint8Array) return value;
    if (value instanceof ArrayBuffer) return new Uint8Array(value);
    return value || null;
}

export function bytesFromF32(values) {
    return new Uint8Array(Float32Array.from(values).buffer);
}

export function bytesFromU16(values) {
    return new Uint8Array(Uint16Array.from(values).buffer);
}

export function bytesFromU32(values) {
    return new Uint8Array(Uint32Array.from(values).buffer);
}

/** Appends an accessor to a SceneDocument and returns its index. */
export function appendAccessor(document, accessor) {
    const index = document.accessors.length;
    document.accessors.push(accessor);
    return index;
}
