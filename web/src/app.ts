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
  animClipSelect,
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
  fbxCompressionLevel,
  fbxCompressionLevelRow,
  useFbxCompression,
  useFbxLegacy,
  exportBtn,
  exportFormat,
  exportSection,
  exportSidebar,
  fileInfo,
  fileInput,
  folderInput,
  fileName,
  fileSize,
  normalBits,
  positionBits,
  previewSection,
  sceneInfo,
  scenePanel,
  sceneSection,
  texcoordBits,
  colorBits,
  genericBits,
  includeNormals,
  includeUvs,
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
import { parseDrcFile, parseFbxFile, parseGltfFile, parseObjFile, parsePlyFile, parseStlFile } from './app/parsers.ts';
import {
  describeSceneCapabilities,
  displayMeshInfo,
  renderSceneDocumentSummary,
} from './app/scene-report.ts';
import {
  animationTick,
  handlePlaybackShortcut,
  installAnimationClipPicker,
  rebuildAnimationClipMenu,
  resetAnimationUi,
  selectAnimationClipAt,
  syncAnimationClipSelection,
  toggleAnimationPlayback,
  updateAnimationPlayButton,
  updateAnimationScrub,
  updateAnimationUi,
} from './app/animation-ui.ts';
import { clearExportStats, exportFile, updateExportOptions } from './app/export.ts';
import {
  ensureViewer,
  installViewerToggles,
  loadPreview,
  setViewerControlsEnabled,
  syncAutoRotateButton,
  syncViewerToolbar,
} from './app/preview.ts';
import { installVariantPicker } from './app/variant-picker.ts';
import { entriesFromDataTransfer } from './app/dropped-entries.ts';
import { findModels, readModel } from './app/model-intake.ts';
import type { IntakeEntry } from './app/model-intake.ts';
import { installModelPicker, syncModelPicker } from './app/model-picker.ts';




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



/**
 * What each region of the encoding-speed scale actually costs.
 *
 * Draco's speed is not one dial trading size for time evenly: it switches four
 * separate things at four thresholds, and two of them move the opposite way
 * from what the label suggests. The figures come from encoding and decoding
 * real meshes with this project's own coders -- a 3k-face sphere, a 16k-face
 * head and a 26k-face torus straight out of Blender, plus the same models
 * welded the way an OBJ import leaves them.
 *
 * The two counter-intuitive parts are worth stating plainly, because a user
 * dragging toward "best" is otherwise misled:
 *
 * - Below speed 4 the encoder switches to predictors that guess one attribute
 *   from another. Those need vertices shared between faces to have anything to
 *   reuse; glTF geometry has none, so the file stops shrinking while the decode
 *   keeps getting slower.
 * - At speed 6 the encoder splits the mesh along attribute seams. For glTF that
 *   is free, because the mesh arrives already split -- same bytes, noticeably
 *   faster decode. For a welded mesh it duplicates every seam vertex and the
 *   file can more than triple.
 */
const ENCODING_SPEED_NOTES: { upTo: number; zone: string; text: string }[] = [
  {
    upTo: 3,
    zone: 'deep',
    text: 'Smaller only where faces share vertices, as in OBJ and PLY — up to 30%. glTF gains nothing below 4 and decodes ~1.5× slower.',
  },
  {
    upTo: 4,
    zone: 'best',
    text: 'Recommended. Smallest or near-smallest on every mesh measured, and the setting Blender’s glTF exporter uses.',
  },
  {
    upTo: 5,
    zone: 'best',
    text: 'Balanced, and Draco’s own default — about 2% larger than 4 on most meshes. Either is safe on any input.',
  },
  {
    upTo: 7,
    zone: 'fast',
    text: 'Same size as 5 on glTF geometry and roughly 40% faster to decode — but a mesh with UV or normal seams can grow several times over.',
  },
  {
    upTo: 10,
    zone: 'bulky',
    text: 'Larger for little gain: 4–11% at 8 and 9, and 1.5–3× at 10, where connectivity is stored without compression. Preview only.',
  },
];

/**
 * Matches the CSS gradient stops on `.zoned-range`/`.quant-range`, so the
 * thumb reads as sitting inside the band it currently occupies instead of as
 * a fixed accent dot unrelated to the track underneath it.
 */
const ZONE_COLORS: Record<string, string> = {
  deep: '#43618c',
  best: 'var(--success)',
  fast: 'var(--warning)',
  bulky: 'var(--error)',
  low: 'var(--warning)',
  good: 'var(--success)',
  high: '#43618c',
};

/** Mirrors the encoding-speed slider into its readout, its explanation, and its thumb color. */
function updateEncodingSpeedNote() {
  const speed = Number(encodingSpeed.value);
  element('speed-value').textContent = encodingSpeed.value;
  const note = ENCODING_SPEED_NOTES.find((entry) => speed <= entry.upTo)
    ?? ENCODING_SPEED_NOTES[ENCODING_SPEED_NOTES.length - 1]!;
  const noteElement = element('speed-note');
  noteElement.textContent = note.text;
  noteElement.dataset.zone = note.zone;
  encodingSpeed.style.setProperty('--thumb-color', ZONE_COLORS[note.zone] ?? 'var(--accent)');
}

/**
 * Colors a quantization slider's thumb to match the band its value sits in.
 *
 * `--good-from`/`--good-to` are already set per slider as percentages of its
 * own 0..16 range to paint the track (see the quant-range rule in
 * style.css), so the same two numbers decide the thumb color rather than a
 * second copy of the thresholds that could drift from the track they describe.
 */
function updateQuantThumbColor(input: HTMLInputElement) {
  const min = Number(input.min || '0');
  const max = Number(input.max || '100');
  const goodFrom = parseFloat(input.style.getPropertyValue('--good-from')) || 0;
  const goodTo = parseFloat(input.style.getPropertyValue('--good-to')) || 100;
  const percent = max === min ? 0 : ((Number(input.value) - min) / (max - min)) * 100;
  const color = percent < goodFrom
    ? ZONE_COLORS.low
    : percent < goodTo
      ? ZONE_COLORS.good
      : ZONE_COLORS.high;
  input.style.setProperty('--thumb-color', color);
}

/**
 * Restates the collapsed quantization group's current bits.
 *
 * The group is folded by default because five sliders push the export button
 * past the bottom of a zoomed-in panel, so the summary has to carry enough for
 * a glance to confirm the defaults are still in place.
 */
function updateQuantizationSummary() {
  const bits = [positionBits, normalBits, texcoordBits, colorBits, genericBits]
    .map((input) => (Number(input.value) === 0 ? 'off' : input.value));
  element('quantization-summary').textContent = `${bits.join('/')} bits`;
}

const ENCODING_METHOD_LABELS: Record<string, string> = {
  0: 'Auto',
  1: 'Sequential',
  2: 'Edgebreaker',
};

/** Restates the collapsed method group's current pick, folded for the same reason as quantization. */
function updateMethodSummary() {
  element('method-summary').textContent = ENCODING_METHOD_LABELS[encodingMethod.value] ?? 'Auto';
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
  
  dropZone.addEventListener('drop', async (e) => {
    e.preventDefault();
    dropZone.classList.remove('drag-over');

    // A dropped folder has no entry in `dataTransfer.files` at all, so the
    // items are asked for their filesystem entries first and the file list is
    // only the fallback for a browser that does not offer them.
    const entries = await entriesFromDataTransfer(e.dataTransfer!);
    if (entries.length > 0) await openSelection(entries);
  });

  // File input, and the folder one beside it. `webkitdirectory` fills
  // `webkitRelativePath`, so a folder chosen through the button arrives with
  // the same paths a dropped one does and takes the same route from here.
  for (const input of [fileInput, folderInput]) {
    input.addEventListener('change', () => {
      if (input.files && input.files.length > 0) {
        openSelection(entriesFromFileList(input.files));
      }
    });
  }
  
  // Clear file
  clearFileBtn.addEventListener('click', clearFile);
  
  // Any export-control change invalidates the report for the previous file.
  // It describes the bytes from one exact set of options, so leaving it up
  // while editing the next export would make the numbers look current when
  // they are not.
  const clearExportReport = () => clearExportStats();
  exportFormat.addEventListener('change', () => {
    clearExportReport();
    updateExportOptions();
  });
  encodingMethod.addEventListener('change', () => {
    clearExportReport();
    updateMethodSummary();
  });
  updateMethodSummary();
  includeNormals.addEventListener('change', clearExportReport);
  includeUvs.addEventListener('change', clearExportReport);
  useFbxCompression.addEventListener('change', () => {
    clearExportReport();
    fbxCompressionLevelRow.style.display = useFbxCompression.checked ? 'block' : 'none';
  });
  fbxCompressionLevelRow.style.display = useFbxCompression.checked ? 'block' : 'none';
  fbxCompressionLevel.addEventListener('input', () => {
    clearExportReport();
    element('fbx-compression-level-value').textContent = fbxCompressionLevel.value;
    updateQuantThumbColor(fbxCompressionLevel);
  });
  updateQuantThumbColor(fbxCompressionLevel);
  useFbxLegacy.addEventListener('change', clearExportReport);

  // Draco checkbox
  useDraco.addEventListener('change', () => {
    clearExportReport();
    dracoSettings.style.display = useDraco.checked ? 'grid' : 'none';
  });
  
  // Quantization sliders
  encodingSpeed.addEventListener('input', () => {
    clearExportReport();
    updateEncodingSpeedNote();
  });
  updateEncodingSpeedNote();
  positionBits.addEventListener('input', () => {
    clearExportReport();
    element('position-bits-value').textContent = positionBits.value;
    updateQuantizationSummary();
    updateQuantThumbColor(positionBits);
  });
  normalBits.addEventListener('input', () => {
    clearExportReport();
    element('normal-bits-value').textContent = normalBits.value;
    updateQuantizationSummary();
    updateQuantThumbColor(normalBits);
  });
  texcoordBits.addEventListener('input', () => {
    clearExportReport();
    element('texcoord-bits-value').textContent = texcoordBits.value;
    updateQuantizationSummary();
    updateQuantThumbColor(texcoordBits);
  });
  colorBits.addEventListener('input', () => {
    clearExportReport();
    element('color-bits-value').textContent = colorBits.value;
    updateQuantizationSummary();
    updateQuantThumbColor(colorBits);
  });
  genericBits.addEventListener('input', () => {
    clearExportReport();
    element('generic-bits-value').textContent = genericBits.value;
    updateQuantizationSummary();
    updateQuantThumbColor(genericBits);
  });
  updateQuantizationSummary();
  for (const input of [positionBits, normalBits, texcoordBits, colorBits, genericBits]) {
    updateQuantThumbColor(input);
  }

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
  installModelPicker((path) => {
    const model = state.currentSelection.find((entry) => entry.path === path);
    if (model) void handleModel(model, state.currentSelection);
  });

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
  installAnimationClipPicker();
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
/**
 * Open whatever was handed over: a file, several, or a whole folder.
 *
 * The selection is not read here. It is a list of names, and only the model
 * chosen out of it — plus the companions that model names — is ever opened.
 * That is what makes a dropped folder affordable: a thousand files cost a
 * thousand names.
 */
async function openSelection(entries: IntakeEntry[]) {
  const models = findModels(entries);
  if (models.length === 0) {
    log('No supported 3D file found in the selection', 'error');
    return;
  }
  state.currentSelection = entries;
  syncModelPicker(models);
  await handleModel(models[0], entries);
}

/** Every supplied file, with the path it sat at inside the selection. */
function entriesFromFileList(fileList: FileList): IntakeEntry[] {
  return Array.from(fileList, (file) => ({
    path: (file as File & { webkitRelativePath?: string }).webkitRelativePath || file.name,
    file,
  }));
}

async function handleModel(model: IntakeEntry, entries: IntakeEntry[]) {
  const file = model.file as File;
  const extension = model.path.split('.').pop()!.toLowerCase();

  log(`Loading ${file.name}...`, 'info');

  // Grid, and stated here because this inline value is what the stylesheet's
  // own `display` has to agree with: the bar lays the name and the remove
  // button out in a row and gives the model picker the width of both.
  fileInfo.style.display = 'grid';
  dropZone.style.display = 'none';

  state.currentFileType = extension;

  try {
    const intake = await readModel(model, entries);
    const data = intake.data;
    state.currentSourceData = new Uint8Array(data);
    state.currentSourceResources = intake.resources;
    // Set with the bytes rather than before them: the field says which model is
    // open, and until the read returns the answer is still the previous one.
    state.currentModelPath = model.path;
    state.currentSceneDocument = null;
    state.currentFbxProvenance = null;
    state.currentGltfProvenance = null;
    state.currentVariant = null;
    clearWarningPanel();
    clearExportStats();

    // Counted rather than assumed: with a folder the selection is arbitrarily
    // large and the number that means anything is what the model actually
    // pulled in.
    const companions = Object.keys(intake.resources).length;
    fileName.textContent = companions > 0 ? `${file.name} (+${companions} resources)` : file.name;
    fileSize.textContent = formatFileSize(
      Object.values(intake.resources).reduce((sum, bytes) => sum + bytes.length, data.length),
    );
    for (const uri of intake.missing) {
      log(`Referenced file not in the selection: ${uri}`, 'warning');
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
      case 'stl':
        result = await parseStlFile(data);
        break;
      case 'drc':
        result = await parseDrcFile(data);
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
            // the one that tells the user which files to select. Geometry that
            // will not decode is the same kind of thing: the container summary
            // above it comes from the JSON alone and would otherwise report a
            // successful parse of an asset whose meshes are unreadable, which
            // is the answer the same corruption gets in a .drc file.
            const message = errorMessage(error);
            if (message.includes('External resource denied:') || message.includes('Draco decode error:')) {
              throw error;
            }
            log(`Scene details unavailable: ${message}`, 'warning');
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
    // A model whose companions sit in a sibling folder cannot be selected file
    // by file at all, so the advice names the way that always works.
    const resourceHint = extension === 'gltf' && message.includes('External resource denied:')
      ? ' Drop the whole folder instead, or select the .gltf together with every referenced .bin and image.'
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
  state.currentSelection = [];
  state.currentModelPath = null;
  syncModelPicker([]);

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
  clearExportStats();
  sceneSection.style.display = 'none';
  exportSidebar.style.display = 'none';
  workspace.classList.remove('export-loaded');
  workspace.classList.remove('scene-loaded');

  // Both, or choosing the same folder twice in a row fires no change event the
  // second time and the panel sits empty.
  fileInput.value = '';
  folderInput.value = '';

  log('File cleared', 'info');
}

// Format file size

// Initialize on load
init();
