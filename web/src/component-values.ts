/**
 * Reading glTF component types out of raw bytes.
 *
 * Both the importers and the exporters walk accessor payloads, and every one
 * of them needs the same three facts about a component type: how wide it is,
 * how to read it, and how a normalized integer maps back to its float range.
 * Keeping one copy here means a new width is added in one place.
 */

const COMPONENT_BYTES = new Map<number, number>([
    [5120, 1], [5121, 1], [5122, 2], [5123, 2], [5125, 4], [5126, 4],
]);

/** Width in bytes, or undefined for a type outside the supported set. */
export function componentByteWidth(componentType: number): number | undefined {
    return COMPONENT_BYTES.get(componentType);
}

/** Width in bytes, rejecting anything outside the supported set. */
export function componentByteSize(componentType: number): number {
    const width = COMPONENT_BYTES.get(componentType);
    if (width === undefined) throw new Error(`Unsupported component type ${componentType}`);
    return width;
}

/** Read one little-endian component at a byte offset. */
export function readComponent(view: DataView, offset: number, componentType: number): number {
    switch (componentType) {
        case 5120: return view.getInt8(offset);
        case 5121: return view.getUint8(offset);
        case 5122: return view.getInt16(offset, true);
        case 5123: return view.getUint16(offset, true);
        case 5125: return view.getUint32(offset, true);
        case 5126: return view.getFloat32(offset, true);
        default: throw new Error(`Unsupported component type ${componentType}`);
    }
}

/**
 * Map a normalized integer component onto its float range.
 *
 * Signed types clamp at -1 because the most negative value of a two's
 * complement integer has no positive counterpart, which glTF resolves by
 * mapping both it and the next value to -1.
 */
export function normalizeComponent(value: number, componentType: number): number {
    switch (componentType) {
        case 5120: return Math.max(value / 127, -1);
        case 5121: return value / 255;
        case 5122: return Math.max(value / 32767, -1);
        case 5123: return value / 65535;
        default: return value;
    }
}
