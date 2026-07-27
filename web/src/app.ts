import { formatFileSize } from './app/format.ts';
/**
 * Draco 3D Format Converter — application entry point.
 *
 * What remains here is the wiring: start-up, the event listeners that connect
 * the page to each panel, and the file intake that decides which reader runs.
 * The panels themselves live in ./app/ — module loading, parsers, the scene
 * report, export, preview and the animation bar — each addressing the shared
 * state holder rather than each other.
 */

import { Viewer } from './viewer.ts';
import type { SceneDocument } from './scene-document.ts';
import type { ResourceMap } from './scene-resources.ts';
import type { FbxSceneProvenance } from './fbx-scene-provenance.ts';
import { buildFbxSceneFromGltf, buildFlatMeshesFromGltf, buildSceneFromGltf } from './gltf-loader.ts';
import { buildSceneDocumentWithGltfProvenance } from './gltf-scene-document.ts';
import { buildSceneFromFbx, buildSceneFromMeshes } from './mesh-loader.ts';
import { buildSceneDocumentWithFbxProvenance } from './fbx-scene-document.ts';
import { buildFbxSceneFromDocument } from './fbx-scene-document-writer.ts';
import { serializeSceneDocumentToGlb } from './scene-document-gltf.ts';
import { assertValidSceneDocument, summarizeSceneDocumentGeometry } from './scene-document.ts';
import { basename } from './scene-resources.ts';
import {
  animClipMenu,
  animClipSelect,
  animClipTrigger,
  animLoopCheckbox,
  animPlayBtn,
  animScrub,
  animSpeed,
  animSpeedValue,
  clearFileBtn,
  dracoSettings,
  dropZone,
  element,
  encodingMethod,
  encodingSpeed,
  exportBtn,
  exportFormat,
  exportSection,
  exportSidebar,
  fileInfo,
  fileInput,
  fileName,
  fileSize,
  normalBits,
  positionBits,
  previewSection,
  sceneInfo,
  scenePanel,
  sceneSection,
  texcoordBits,
  useDraco,
  viewerAutoRotateBtn,
  viewerResetBtn,
  viewerSection,
  workspace,
} from './app/dom.ts';
import { debugLog, errorMessage, log } from './app/log.ts';
import { modules, state } from './app/state.ts';
import {
  clearWarningPanel,
  setWarningSource,
} from './app/warnings.ts';
import { loadAllModules, updateDracoEncoderAvailability } from './app/modules.ts';
import { parseFbxFile, parseGltfFile, parseObjFile, parsePlyFile } from './app/parsers.ts';
import {
  describeSceneCapabilities,
  displayMeshInfo,
  renderSceneDocumentSummary,
} from './app/scene-report.ts';
import {
  animationTick,
  closeAnimationClipMenu,
  handleAnimationClipMenuKeydown,
  handleAnimationClipTriggerKeydown,
  handlePlaybackShortcut,
  openAnimationClipMenu,
  rebuildAnimationClipMenu,
  resetAnimationUi,
  selectAnimationClipAt,
  syncAnimationClipSelection,
  toggleAnimationPlayback,
  updateAnimationPlayButton,
  updateAnimationScrub,
  updateAnimationUi,
} from './app/animation-ui.ts';
import { exportFile, updateExportOptions } from './app/export.ts';
import {
  ensureViewer,
  installVariantPicker,
  installViewerToggles,
  loadPreview,
  setViewerControlsEnabled,
  syncAutoRotateButton,
  syncViewerToolbar,
} from './app/preview.ts';




function setupChoiceControl(select: HTMLSelectElement) {
  const control = document.querySelector(`[data-choice-for="${select.id}"]`);
  if (!control) return;
  const buttons = Array.from(control.querySelectorAll<HTMLButtonElement>('button[data-value]'));
  const sync = () => {
    for (const button of buttons) {
      const selected = button.dataset.value === select.value;
      button.classList.toggle('selected', selected);
      button.setAttribute('aria-pressed', String(selected));
    }
  };
  for (const button of buttons) {
    button.addEventListener('click', () => {
      if (button.dataset.value === select.value) return;
      select.value = button.dataset.value!;
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
  }
  select.addEventListener('change', sync);
  sync();
}

setupChoiceControl(exportFormat);
setupChoiceControl(encodingMethod);

// Initialize application
async function init() {
  log('Initializing Draco 3D Format Converter...', 'info');
  
  // Load all WASM modules
  await loadAllModules();
  
  // Setup event listeners
  setupEventListeners();
  
  log('Ready to convert 3D files!', 'success');
}



// Setup event listeners
function setupEventListeners() {
  // Drag and drop
  dropZone.addEventListener('dragover', (e) => {
    e.preventDefault();
    dropZone.classList.add('drag-over');
  });
  
  dropZone.addEventListener('dragleave', () => {
    dropZone.classList.remove('drag-over');
  });
  
  dropZone.addEventListener('drop', (e) => {
    e.preventDefault();
    dropZone.classList.remove('drag-over');
    
    const files = e.dataTransfer!.files;
    if (files.length > 0) {
      handleFiles(files);
    }
  });
  
  // File input
  fileInput.addEventListener('change', () => {
    if (fileInput.files && fileInput.files.length > 0) {
      handleFiles(fileInput.files);
    }
  });
  
  // Clear file
  clearFileBtn.addEventListener('click', clearFile);
  
  // Export format change
  exportFormat.addEventListener('change', updateExportOptions);
  
  // Draco checkbox
  useDraco.addEventListener('change', () => {
    dracoSettings.style.display = useDraco.checked ? 'grid' : 'none';
  });
  
  // Quantization sliders
  encodingSpeed.addEventListener('input', () => {
    element('speed-value').textContent = encodingSpeed.value;
  });
  positionBits.addEventListener('input', () => {
    element('position-bits-value').textContent = positionBits.value;
  });
  normalBits.addEventListener('input', () => {
    element('normal-bits-value').textContent = normalBits.value;
  });
  texcoordBits.addEventListener('input', () => {
    element('texcoord-bits-value').textContent = texcoordBits.value;
  });
  
  // Export button
  exportBtn.addEventListener('click', exportFile);

  // 3D preview toolbar
  viewerResetBtn.addEventListener('click', () => {
    if (!state.viewer) return;
    state.viewer.resetView();
  });
  viewerAutoRotateBtn.addEventListener('click', () => {
    if (!state.viewer) return;
    state.viewer.setAutoRotate(!state.viewer.autoRotate);
  });
  installViewerToggles();
  installVariantPicker();

  // Animation controls
  animPlayBtn.addEventListener('click', toggleAnimationPlayback);
  animClipSelect.addEventListener('change', () => {
    if (!state.viewer) return;
    const idx = Number(animClipSelect.value);
    state.viewer.animation.clipIndex = idx;
    state.viewer.seekAnimation(0);
    syncAnimationClipSelection();
    updateAnimationScrub();
  });
  animClipTrigger.addEventListener('click', (event) => {
    event.stopPropagation();
    const open = animClipTrigger.getAttribute('aria-expanded') !== 'true';
    if (open) openAnimationClipMenu();
    else closeAnimationClipMenu();
  });
  animClipMenu.addEventListener('click', (event) => event.stopPropagation());
  animClipTrigger.addEventListener('keydown', handleAnimationClipTriggerKeydown);
  animClipMenu.addEventListener('keydown', handleAnimationClipMenuKeydown);
  // Wrapped: passing the listener directly would hand the MouseEvent in as a
  // truthy `restoreFocus`, so every click in the page focused the trigger.
  document.addEventListener('click', () => closeAnimationClipMenu());
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') closeAnimationClipMenu();
  });
  document.addEventListener('keydown', handlePlaybackShortcut);
  animLoopCheckbox.addEventListener('change', () => {
    if (state.viewer) state.viewer.animation.loop = animLoopCheckbox.checked;
  });
  animScrub.addEventListener('input', () => {
    if (!state.viewer || !state.viewer.scene?.animations?.length) return;
    const clip = state.viewer.scene.animations[state.viewer.animation.clipIndex];
    if (!clip) return;
    state.viewer.animation.playing = false;
    updateAnimationPlayButton();
    const t = (Number(animScrub.value) / 1000) * clip.duration;
    state.viewer.seekAnimation(t);
    updateAnimationScrub();
  });
  animSpeed.addEventListener('input', () => {
    const v = Number(animSpeed.value) / 100;
    if (state.viewer) state.viewer.animation.speed = v;
    animSpeedValue.textContent = `${v.toFixed(2)}×`;
  });
}

// Handle file selection
async function handleFiles(fileList: FileList) {
  const files = Array.from(fileList);
  const supportedMain = files.filter((file) =>
    ['obj', 'ply', 'gltf', 'glb', 'fbx'].includes(file.name.split('.').pop()!.toLowerCase())
  );
  if (supportedMain.length === 0) {
    log('No supported main 3D file found in the selection', 'error');
    return;
  }
  const gltfMain = supportedMain.filter((file) => /\.(gltf|glb)$/i.test(file.name));
  const file = gltfMain.length === 1 ? gltfMain[0] : supportedMain[0];
  if (supportedMain.length > 1) {
    log('Select exactly one main model file; additional files are only companions for glTF', 'error');
    return;
  }
  await handleFile(file, files.filter((candidate) => candidate !== file));
}


async function handleFile(file: File, companionFiles: File[] = []) {
  const extension = file.name.split('.').pop()!.toLowerCase();
  
  if (!['obj', 'ply', 'gltf', 'glb', 'fbx'].includes(extension)) {
    log(`Unsupported file format: .${extension}`, 'error');
    return;
  }
  
  log(`Loading ${file.name}...`, 'info');
  
  // Show file info
  fileName.textContent = companionFiles.length > 0
    ? `${file.name} (+${companionFiles.length} resources)`
    : file.name;
  const totalSize = companionFiles.reduce((sum, companion) => sum + companion.size, file.size);
  fileSize.textContent = formatFileSize(totalSize);
  fileInfo.style.display = 'flex';
  dropZone.style.display = 'none';
  
  state.currentFileType = extension;
  
  try {
    const arrayBuffer = await file.arrayBuffer();
    const data = new Uint8Array(arrayBuffer);
    state.currentSourceData = new Uint8Array(data);
    state.currentSourceResources = Object.create(null);
    state.currentSceneDocument = null;
    state.currentFbxProvenance = null;
    state.currentGltfProvenance = null;
    state.currentVariant = null;
    clearWarningPanel();
    for (const companion of companionFiles) {
      if (Object.prototype.hasOwnProperty.call(state.currentSourceResources, companion.name)) {
        throw new Error(`Duplicate companion resource name: ${companion.name}`);
      }
      state.currentSourceResources[companion.name] = new Uint8Array(await companion.arrayBuffer());
    }
    
    // Parse file based on extension
    let result;
    switch (extension) {
      case 'obj':
        result = await parseObjFile(data, state.currentSourceResources);
        break;
      case 'ply':
        result = await parsePlyFile(data);
        break;
      case 'gltf':
      case 'glb':
        result = await parseGltfFile(data, extension);
        if (result?.success && result.document) {
          try {
            const adapted = buildSceneDocumentWithGltfProvenance(data, state.currentSourceResources as Record<string, Uint8Array>, modules.gltf.module);
            state.currentSceneDocument = adapted.document;
            state.currentGltfProvenance = adapted.provenance;
            // The geometry figures come from the document rather than from a
            // second walk of the same asset; when it could not be built there
            // is nothing to count, and the panel says so below.
            Object.assign(result, summarizeSceneDocumentGeometry(state.currentSceneDocument));
          } catch (error) {
            // A document that merely fails the portability rules still
            // previews, so that stays a warning. Missing companion files are
            // different: nothing can be read at all, and the outer handler is
            // the one that tells the user which files to select.
            if (errorMessage(error).includes('External resource denied:')) throw error;
            log(`Scene details unavailable: ${errorMessage(error)}`, 'warning');
          }
        }
        break;
      case 'fbx':
        result = await parseFbxFile(data);
        if (result?.success && result.scene) {
          const adapted = buildSceneDocumentWithFbxProvenance(result, state.currentSourceResources);
          state.currentSceneDocument = adapted.document;
          state.currentFbxProvenance = adapted.provenance;
        }
        break;
    }
    
    if (result && result.success) {
      state.currentMeshData = result;
      displayMeshInfo(result);
      renderSceneDocumentSummary(state.currentSceneDocument!);
      previewSection.style.display = 'block';
      exportSection.style.display = 'flex';
      exportSidebar.style.display = 'flex';
      workspace.classList.add('export-loaded');
      setWarningSource('export', []);
      log(`Successfully parsed ${file.name}`, 'success');
      await loadPreview(extension);
    } else {
      log(`Failed to parse file: ${result?.error || 'Unknown error'}`, 'error');
    }
  } catch (error) {
    const message = errorMessage(error);
    const resourceHint = extension === 'gltf' && message.includes('External resource denied:')
      ? ' Select the .gltf together with all referenced .bin and image files.'
      : '';
    log(`Error reading file: ${message}.${resourceHint}`, 'error');
  }
}






























// === 3D preview integration ===















requestAnimationFrame(animationTick);


// Clear loaded file
function clearFile() {
  state.currentMeshData = null;
  state.currentFileType = null;
  state.currentSourceData = null;
  state.currentSourceResources = Object.create(null);
  state.currentSceneDocument = null;
  state.currentFbxProvenance = null;
  state.currentGltfProvenance = null;
  state.currentVariant = null;
  
  fileInfo.style.display = 'none';
  dropZone.style.display = 'grid';
  previewSection.style.display = 'none';
  exportSection.style.display = 'none';
  viewerSection.classList.remove('loaded');
  state.viewer?.clear();
  setViewerControlsEnabled(false);
  resetAnimationUi();
  scenePanel.hidden = true;
  sceneInfo.hidden = true;
  clearWarningPanel();
  sceneSection.style.display = 'none';
  exportSidebar.style.display = 'none';
  workspace.classList.remove('export-loaded');
  workspace.classList.remove('scene-loaded');

  fileInput.value = '';

  log('File cleared', 'info');
}

// Format file size

// Initialize on load
init();
