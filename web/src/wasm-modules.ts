/**
 * Hand-written surface of the wasm-pack modules.
 *
 * wasm-pack generates its own .d.ts next to each module, but `www/pkg` is
 * build output and is not tracked, so a clone that has not built the WASM yet
 * would fail to typecheck against it. These declarations therefore cover only
 * what the front-end actually calls, and are widened as more of it converts.
 */

export interface GltfAsset {
    free(): void;
    validate(validationProfile: string): void;
    /** Serializes the document as a GLB container of the given version. */
    glb(version: number): Uint8Array;
    json(): Uint8Array;
    minifiedJson(): Uint8Array;
    meshCount(): number;
    primitiveCount(mesh: number): number;
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
