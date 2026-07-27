import type { ViewerScene } from '../viewer-scene.ts';
import { Viewer } from '../viewer.ts';
import { buildSceneFromFbx, buildSceneFromMeshes } from '../mesh-loader.ts';
import { buildSceneFromGltf, extensionWarnings } from '../gltf-loader.ts';
import { buildViewerSceneFromDocument } from '../scene-document-viewer.ts';
import { hydrateSceneTextures, honoredTextureSources } from '../scene-document-textures.ts';
import { chooseCompressedTarget } from '../viewer/compressed-formats.ts';
import type { SceneDocument } from '../scene-document.ts';
import { errorMessage, log } from './log.ts';
import { loadKtx2Module } from './modules.ts';
import { modules, state } from './state.ts';
import { renderSceneDocumentSummary } from './scene-report.ts';
import { setWarningSource } from './warnings.ts';
import { updateAnimationPlayButton, updateAnimationUi } from './animation-ui.ts';
import { viewerAutoRotateBtn, viewerBaseColorBtn, viewerCanvas, viewerControls, viewerGridBtn, viewerSection, viewerSmoothNormalsBtn, viewerWireframeBtn } from './dom.ts';

/**
 * The 3D preview: creating the viewer on demand, loading a scene into it, and
 * keeping the viewport toolbar in step with the viewer's own display flags.
 */

/** The viewer display flags a viewport button can toggle. */
type ViewerToggle = 'wireframe' | 'baseColorOnly' | 'smoothNormals' | 'showGrid';

export function ensureViewer() {
  if (state.viewer) return state.viewer;
  try {
    state.viewer = new Viewer(viewerCanvas, {
      onLog: (msg: string, type: string) => log(msg, type),
      onSceneLoaded: (scene: ViewerScene) => {
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

/**
 * Which preview run is the current one.
 *
 * Loading is asynchronous - a frame is yielded for layout, textures are
 * decoded - so two runs can overlap: opening a file while one is in flight, or
 * picking a material variant before the first finished. Without this the last
 * one to *finish* wins, which is not the last one asked for.
 */
let previewGeneration = 0;

export async function loadPreview(extension: string, { keepView = false } = {}) {
  const generation = ++previewGeneration;
  viewerSection.classList.add('loaded');
  setViewerControlsEnabled(false);

  // Yield so the section layout settles before the canvas is measured. A
  // background tab is served no animation frames at all, so give up waiting
  // after a moment rather than leaving the file parsed but never previewed;
  // the resize observer corrects the canvas once the tab is shown again.
  await new Promise<void>((resolve) => {
    const proceed = () => {
      clearTimeout(timer);
      resolve();
    };
    const timer = setTimeout(proceed, 100);
    requestAnimationFrame(proceed);
  });

  if (!ensureViewer()) {
    log('Preview unavailable', 'error');
    return;
  }

  try {
    let scene: ViewerScene;
    if (extension === 'gltf' || extension === 'glb') {
      if (!modules.gltf.loaded) throw new Error('glTF module is not loaded');
      scene = state.currentSceneDocument
        ? await previewFromDocument(state.currentSceneDocument)
        : await previewFromLoader();
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

    // A newer run has taken over; showing this one would put back what the
    // user just moved away from.
    if (generation !== previewGeneration) return;

    for (const warning of scene.warnings || []) {
      log(warning, 'warning');
    }

    state.viewer!.setScene(scene, { keepView });
    renderSceneDocumentSummary(state.currentSceneDocument!);
    setWarningSource('preview', scene.warnings || []);
    setViewerControlsEnabled(true);
    syncViewerToolbar();
    log('Preview ready', 'success');
  } catch (error) {
    if (generation !== previewGeneration) return;
    state.viewer!.clear();
    setViewerControlsEnabled(false);
    log(`Preview failed: ${errorMessage(error)}`, 'error');
  }
}

/**
 * Build the preview from the document the summary and every export already
 * read.
 *
 * The document is built once when the file is opened; before this, the preview
 * opened the same bytes again with a second reader, and the two could disagree
 * about what the file contained without anything saying so. What the renderer
 * cannot show comes from the adapter; what the file claimed and this browser
 * could not honor is stated here, because only after hydration is the answer
 * known.
 */
async function previewFromDocument(document: SceneDocument): Promise<ViewerScene> {
  const scene = await hydrateSceneTextures(
    buildViewerSceneFromDocument(document, state.currentVariant) as ViewerScene,
    {
      load: loadKtx2Module,
      // The viewer's context decides, because it is the one that will be
      // handed the result: what a machine can sample is not a property of
      // the file.
      target: (codec, hasAlpha) => chooseCompressedTarget(
        state.viewer?.compressedTextureExtensions ?? [],
        codec,
        hasAlpha,
      ),
    },
  );
  scene.warnings.push(...extensionWarnings(
    state.currentGltfProvenance ?? {},
    honoredTextureSources(scene),
  ));
  return scene;
}

/**
 * The reader that used to be the only path, kept for the files the portable
 * document cannot be built from.
 *
 * Every glTF in `testdata` survives the document route, so this is rare rather
 * than routine — but a user's file is not bounded by that corpus, and showing
 * nothing is worse than showing it through the older reader and saying so.
 */
async function previewFromLoader(): Promise<ViewerScene> {
  log('Scene document unavailable; previewing through the direct glTF reader instead', 'warning');
  return buildSceneFromGltf(
    state.currentSourceData!,
    state.currentSourceResources,
    modules.gltf.module,
    { onLog: (msg: string, type: string) => log(msg, type) },
  );
}

export function setViewerControlsEnabled(enabled: boolean) {
  for (const control of viewerControls) control.disabled = !enabled;
}

/** Which viewport button drives which viewer display flag. */
const VIEWER_TOGGLES: [HTMLButtonElement, ViewerToggle][] = [
  [viewerWireframeBtn, 'wireframe'],
  [viewerBaseColorBtn, 'baseColorOnly'],
  [viewerSmoothNormalsBtn, 'smoothNormals'],
  [viewerGridBtn, 'showGrid'],
];

/**
 * Wire the viewport toggles to their flags.
 *
 * The handlers only flip the flag; how a button then looks is
 * syncViewerToolbar's business, so the button-to-flag mapping is stated once
 * and the two cannot drift apart.
 */
export function installViewerToggles() {
  for (const [button, flag] of VIEWER_TOGGLES) {
    button.addEventListener('click', () => {
      if (!state.viewer) return;
      state.viewer[flag] = !state.viewer[flag];
      syncViewerToolbar();
    });
  }
}

export function syncViewerToolbar() {
  if (!state.viewer) return;
  syncAutoRotateButton(state.viewer.autoRotate);
  for (const [button, flag] of VIEWER_TOGGLES) setPressed(button, state.viewer[flag]);
}

export function syncAutoRotateButton(enabled: boolean) {
  setPressed(viewerAutoRotateBtn, enabled);
}

/** A toggle button reports its state to both the stylesheet and the reader. */
function setPressed(button: HTMLElement, active: boolean) {
  button.classList.toggle('active', active);
  button.setAttribute('aria-pressed', String(active));
}
