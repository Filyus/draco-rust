/**
 * Optional, FBX-only provenance kept beside a portable SceneDocument.
 *
 * SceneDocument itself remains transferable and source-neutral.  This sidecar
 * preserves the semantic FBX scene and the exact evaluator convention that
 * produced it, so an FBX export boundary can make an explicit choice between
 * source preservation and a canonical baked FBX profile.
 */

export const FBX_SCENE_PROVENANCE_VERSION = 1;

/** Build a serializable sidecar for an FBX semantic parse result. */
export function createFbxSceneProvenance(parsed) {
    if (!parsed?.scene?.rootNodes?.length) throw new Error('FBX provenance requires a semantic scene');
    const settings = parsed.scene.globalSettings || null;
    return {
        version: FBX_SCENE_PROVENANCE_VERSION,
        format: 'fbx',
        coordinateSpace: {
            // fbx-wasm materializes Model/Cluster values in authored semantic
            // FBX axes; retain decoded GlobalSettings without applying a
            // guessed conversion to the portable document.
            axes: settings ? 'fbx-global-settings' : 'semantic-fbx-native',
            unitScaleFactor: settings?.unitScaleFactor ?? null,
            sourceUnit: settings?.unitScaleFactor === 1 || settings?.unitScaleFactor === 100
                ? 'centimeter' : 'decoded',
            sceneDocumentMetersPerSourceUnit: 0.01,
        },
        ...(settings ? { globalSettings: structuredClone(settings) } : {}),
        animation: {
            // The source scene contains raw Lcl curves. The active FBX
            // evaluator additionally applies bind/rest and pre/post policies
            // before it exposes canonical SceneDocument channels.
            rawChannels: 'semantic-fbx-scene',
            evaluator: 'fbx-viewer-bind-rest-v1',
            canonicalBake: 'fbx-local-trs-sampled-v1',
        },
        sourceScene: structuredClone(parsed.scene),
    };
}

/** Return a detached semantic scene suitable for the typed FBX writer. */
export function cloneFbxSemanticScene(provenance) {
    assertFbxProvenance(provenance);
    return structuredClone(provenance.sourceScene);
}

export function assertFbxProvenance(provenance) {
    if (!provenance || provenance.version !== FBX_SCENE_PROVENANCE_VERSION || provenance.format !== 'fbx'
        || !provenance.sourceScene?.rootNodes?.length) {
        throw new Error('Invalid FBX SceneDocument provenance');
    }
}
