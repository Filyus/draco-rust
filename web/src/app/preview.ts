import { Viewer } from '../viewer.ts';
import { buildSceneFromFbx, buildSceneFromMeshes } from '../mesh-loader.ts';
import { buildSceneFromGltf } from '../gltf-loader.ts';
import { errorMessage, log } from './log.ts';
import { modules, state } from './state.ts';
import { renderSceneDocumentSummary } from './scene-report.ts';
import { setWarningSource } from './warnings.ts';
import { updateAnimationPlayButton, updateAnimationUi } from './animation-ui.ts';
import { viewerAutoRotateBtn, viewerBaseColorBtn, viewerCanvas, viewerControls, viewerGridBtn, viewerSection, viewerSmoothNormalsBtn, viewerWireframeBtn } from './dom.ts';

/**
 * The 3D preview: creating the viewer on demand, loading a scene into it, and
 * keeping the viewport toolbar in step with the viewer's own display flags.
 */

export function ensureViewer() {
    if (state.viewer) return state.viewer;
    try {
        state.viewer = new Viewer(viewerCanvas, {
            onLog: (msg: string, type: string) => log(msg, type),
            onSceneLoaded: (scene: any) => {
                if (scene) updateAnimationUi(scene);
            },
            onAnimationEnded: () => updateAnimationPlayButton(),
            onAutoRotateChange: syncAutoRotateButton,
        });
        syncViewerToolbar();
    } catch (error) {
        log(`Preview unavailable: ${errorMessage(error)}`, 'error');
        state.viewer = null;
    }
    return state.viewer;
}

export async function loadPreview(extension: string) {
    viewerSection.classList.add('loaded');
    setViewerControlsEnabled(false);

    // Yield to the browser so the section layout settles before measuring the canvas.
    await new Promise((resolve) => requestAnimationFrame(resolve));

    if (!ensureViewer()) {
        log('Preview unavailable', 'error');
        return;
    }

    try {
        let scene: any;
        if (extension === 'gltf' || extension === 'glb') {
            if (!modules.gltf.loaded) throw new Error('glTF module is not loaded');
            scene = await buildSceneFromGltf(
                state.currentSourceData!,
                state.currentSourceResources,
                modules.gltf.module,
                { onLog: (msg: string, type: string) => log(msg, type) },
            );
        } else if (extension === 'fbx' && state.currentMeshData?.scene) {
            scene = await buildSceneFromFbx(
                state.currentMeshData,
                state.currentSourceResources,
                { onLog: (msg: string, type: string) => log(msg, type) },
            );
        } else if (state.currentMeshData?.meshes) {
            scene = await buildSceneFromMeshes(
                state.currentMeshData,
                state.currentSourceResources,
                { onLog: (msg: string, type: string) => log(msg, type) },
            );
        } else {
            throw new Error('No geometry available to preview');
        }

        for (const warning of scene.warnings || []) {
            log(warning, 'warning');
        }

        state.viewer!.setScene(scene);
        renderSceneDocumentSummary(state.currentSceneDocument!);
        setWarningSource('preview', scene.warnings || []);
        setViewerControlsEnabled(true);
        syncViewerToolbar();
        log('Preview ready', 'success');
    } catch (error) {
        state.viewer!.clear();
        setViewerControlsEnabled(false);
        log(`Preview failed: ${errorMessage(error)}`, 'error');
    }
}

export function setViewerControlsEnabled(enabled: boolean) {
    for (const control of viewerControls) control.disabled = !enabled;
}

export function syncViewerToolbar() {
    if (!state.viewer) return;
    syncAutoRotateButton(state.viewer.autoRotate);
    const toggles: [HTMLButtonElement, boolean][] = [
        [viewerWireframeBtn, state.viewer.wireframe],
        [viewerBaseColorBtn, state.viewer.baseColorOnly],
        [viewerSmoothNormalsBtn, state.viewer.smoothNormals],
        [viewerGridBtn, state.viewer.showGrid],
    ];
    for (const [button, enabled] of toggles) {
        button.classList.toggle('active', enabled);
        button.setAttribute('aria-pressed', String(enabled));
    }
}

export function syncAutoRotateButton(enabled: boolean) {
    viewerAutoRotateBtn.classList.toggle('active', enabled);
    viewerAutoRotateBtn.setAttribute('aria-pressed', String(enabled));
}
