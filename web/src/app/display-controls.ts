/**
 * What the frame is shown through: the tone curve, the exposure, how much light
 * the environment gives, and how much of that environment stands behind the
 * model.
 *
 * None of the four changes the file — they are the viewing conditions, and the
 * first three are the ones the reference viewer puts in its Display panel — so
 * they belong to the viewport rather than to the document, and they survive
 * loading another model. Exposure and intensity run in log units, which is how
 * a stop and a factor of ten are spaced evenly and how the middle of each track
 * lands exactly on 1; the backdrop is a plain fraction of the room, because
 * that is what it is.
 */
import {
  displayBackdrop,
  displayBackdropValue,
  displayExposure,
  displayExposureValue,
  displayIbl,
  displayIblValue,
  displayReset,
  displayToneMapLabel,
  displayToneMapMenu,
  displayToneMapSelect,
  displayToneMapTrigger,
  viewerDisplayBtn,
  viewerDisplayPanel,
} from './dom.ts';
import { createMenuPicker } from './menu-picker.ts';
import { state } from './state.ts';
import { TONE_MAP_ACES, TONE_MAP_NEUTRAL, TONE_MAP_NONE } from '../viewer/shaders.ts';
import { DEFAULT_BACKDROP_LEVEL } from '../viewer/renderer.ts';

/**
 * The curves offered, neutral first because it is the default.
 *
 * Neutral leaves the lower two thirds of the range alone, so a base colour
 * arrives on screen as the value the asset stores and only highlights roll
 * off. The filmic curve is the photographic look, which lifts and desaturates
 * everything it touches. Clamping is neither, and is there for reading values
 * off the frame rather than for looking at it.
 */
const TONE_MAPS: { mode: number; label: string }[] = [
  { mode: TONE_MAP_NEUTRAL, label: 'Khronos PBR Neutral' },
  { mode: TONE_MAP_ACES, label: 'ACES filmic' },
  { mode: TONE_MAP_NONE, label: 'None (clamped)' },
];

const picker = createMenuPicker({
  select: displayToneMapSelect,
  trigger: displayToneMapTrigger,
  label: displayToneMapLabel,
  menu: displayToneMapMenu,
  placeholder: 'Tone map',
  optionId: 'display-tone-map-option',
});

/** The viewing conditions, kept here so a new model is shown under the last ones. */
const settings = {
  toneMap: TONE_MAP_NEUTRAL,
  exposureStops: 0,
  iblDecades: 0,
  backdrop: DEFAULT_BACKDROP_LEVEL,
};

const exposureOf = (stops: number) => 2 ** stops;
const intensityOf = (decades: number) => 10 ** decades;

function showValues() {
  displayExposureValue.textContent = `${exposureOf(settings.exposureStops).toFixed(2)}×`;
  displayIblValue.textContent = `${intensityOf(settings.iblDecades).toFixed(2)}×`;
  displayBackdropValue.textContent = `${Math.round(settings.backdrop * 100)}%`;
}

/** Push the settings at the viewer, which is only there once a model has loaded. */
export function applyDisplaySettings() {
  const viewer = state.viewer;
  if (!viewer) return;
  viewer.toneMap = settings.toneMap;
  viewer.exposure = exposureOf(settings.exposureStops);
  viewer.iblIntensity = intensityOf(settings.iblDecades);
  viewer.backdropLevel = settings.backdrop;
}

function syncControls() {
  displayToneMapSelect.value = String(settings.toneMap);
  picker.sync();
  displayExposure.value = String(settings.exposureStops);
  displayIbl.value = String(settings.iblDecades);
  displayBackdrop.value = String(settings.backdrop);
  showValues();
}

/** Open and close the panel, and say which state the button is in. */
function setPanelOpen(open: boolean) {
  viewerDisplayPanel.hidden = !open;
  viewerDisplayBtn.classList.toggle('active', open);
  viewerDisplayBtn.setAttribute('aria-expanded', String(open));
}

export function installDisplayControls() {
  displayToneMapSelect.replaceChildren(...TONE_MAPS.map(({ mode, label }) => {
    const option = document.createElement('option');
    option.value = String(mode);
    option.textContent = label;
    return option;
  }));
  picker.install();
  picker.rebuild();
  syncControls();

  viewerDisplayBtn.addEventListener('click', () => setPanelOpen(viewerDisplayPanel.hidden !== false));
  // A click anywhere else closes it, the way the pickers close: the panel is a
  // transient overlay on the viewport, not a second sidebar.
  document.addEventListener('pointerdown', (event) => {
    if (viewerDisplayPanel.hidden) return;
    const target = event.target as Node;
    if (viewerDisplayPanel.contains(target) || viewerDisplayBtn.contains(target)) return;
    setPanelOpen(false);
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !viewerDisplayPanel.hidden) setPanelOpen(false);
  });

  displayToneMapSelect.addEventListener('change', () => {
    settings.toneMap = Number(displayToneMapSelect.value);
    picker.sync();
    applyDisplaySettings();
  });
  displayExposure.addEventListener('input', () => {
    settings.exposureStops = Number(displayExposure.value);
    showValues();
    applyDisplaySettings();
  });
  displayIbl.addEventListener('input', () => {
    settings.iblDecades = Number(displayIbl.value);
    showValues();
    applyDisplaySettings();
  });
  displayBackdrop.addEventListener('input', () => {
    settings.backdrop = Number(displayBackdrop.value);
    showValues();
    applyDisplaySettings();
  });
  displayReset.addEventListener('click', () => {
    settings.toneMap = TONE_MAP_NEUTRAL;
    settings.exposureStops = 0;
    settings.iblDecades = 0;
    settings.backdrop = DEFAULT_BACKDROP_LEVEL;
    syncControls();
    applyDisplaySettings();
  });
}

/** Close the panel when the viewport controls go away with the model. */
export function closeDisplayPanel() {
  setPanelOpen(false);
}
