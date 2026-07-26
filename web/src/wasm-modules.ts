/**
 * Hand-written surface of the wasm-pack modules.
 *
 * wasm-pack generates its own .d.ts next to each module, but `www/pkg` is
 * build output and is not tracked, so a clone that has not built the WASM yet
 * would fail to typecheck against it. These declarations therefore cover only
 * what the front-end actually calls, and are widened as more of it converts.
 */

/** One accessor materialized into tightly packed little-endian bytes. */
export interface PackedAccessor {
    free(): void;
    bytes(): Uint8Array;
    componentType(): number;
    components(): number;
    count(): number;
    normalized(): boolean;
}

/** One primitive read back with Draco compression already resolved. */
export interface PackedGeometry {
    free(): void;
    mode(): number;
    attributeCount(): number;
    attributeSemantic(index: number): string;
    attributeBytes(index: number): Uint8Array;
    attributeComponentType(index: number): number;
    attributeComponents(index: number): number;
    attributeElementCount(index: number): number;
    attributeNormalized(index: number): boolean;
    hasIndices(): boolean;
    indexBytes(): Uint8Array;
    indexComponentType(): number;
    indexCount(): number;
}

export interface GltfAsset {
    free(): void;
    validate(validationProfile: string): void;
    /** Serializes the document as a GLB container of the given version. */
    glb(version: number): Uint8Array;
    json(): Uint8Array;
    minifiedJson(): Uint8Array;
    meshCount(): number;
    primitiveCount(mesh: number): number;
    bufferViewBytes(index: number): Uint8Array;
    readAccessor(index: number): PackedAccessor;
    readPrimitive(mesh: number, primitive: number): PackedGeometry;
}

export interface GltfModule {
    GltfAsset: {
        /** Opens a document with an explicit URI-to-bytes resource map. */
        withResources(
            json: Uint8Array,
            resources: Record<string, Uint8Array>,
            validationProfile: string,
        ): GltfAsset;
    };
}
