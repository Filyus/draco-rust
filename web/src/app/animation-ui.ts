import { animClipLabel, animClipMenu, animClipSelect, animClipTrigger, animPlayBtn, animScrub, animSpeed, animSpeedValue, animTimeLabel, viewerAnimation } from './dom.ts';
import { createMenuPicker } from './menu-picker.ts';
import type { ViewerScene } from '../viewer-scene.ts';
import { state } from './state.ts';

/**
 * The animation bar: clip menu, playback, scrub and their keyboard handling.
 *
 * The viewer owns playback state; this only drives it and reflects it back, so
 * the bar and the viewport can never disagree about what is playing.
 */

/**
 * The clip list, drawn as a listbox over the real select.
 *
 * The select keeps the value and fires `change`; only the popup is ours,
 * because the platform draws a native one in the platform's palette.
 */
const clipPicker = createMenuPicker({
  select: animClipSelect,
  trigger: animClipTrigger,
  label: animClipLabel,
  menu: animClipMenu,
  placeholder: 'Animation',
  optionId: 'anim-clip-option',
});

export function updateAnimationUi(scene: ViewerScene) {
  const clips = state.currentSceneDocument?.animations?.length
    ? state.currentSceneDocument.animations
    : (scene.animations || []);
  resetAnimationUi();
  if (clips.length === 0) return;
  viewerAnimation.style.display = 'flex';
  for (let i = 0; i < clips.length; i++) {
    const option = document.createElement('option');
    option.value = String(i);
    option.textContent = clips[i].name || `Clip ${i + 1}`;
    animClipSelect.appendChild(option);
  }
  animClipSelect.value = String(state.viewer!.animation.clipIndex);
  rebuildAnimationClipMenu();
  updateAnimationPlayButton();
  updateAnimationScrub();
}

export function resetAnimationUi() {
  lastTimeLabel = '';
  lastScrubValue = '';
  viewerAnimation.style.display = 'none';
  animClipSelect.innerHTML = '';
  animClipMenu.replaceChildren();
  animClipLabel.textContent = 'Animation';
  closeAnimationClipMenu();
  animTimeLabel.textContent = '0.00s';
  animScrub.value = '0';
  animSpeedValue.textContent = '1.00×';
  animSpeed.value = '100';
  animPlayBtn.classList.remove('active');
  animPlayBtn.title = 'Play';
  animPlayBtn.setAttribute('aria-label', 'Play animation');
}

export const rebuildAnimationClipMenu = () => clipPicker.rebuild();
export const syncAnimationClipSelection = () => clipPicker.sync();
export const closeAnimationClipMenu = () => clipPicker.close();
export const selectAnimationClipAt = (index: number) => clipPicker.selectAt(index);
export const installAnimationClipPicker = () => clipPicker.install();

export function toggleAnimationPlayback() {
  if (!state.viewer || !state.viewer.scene?.animations?.length) return false;
  state.viewer.animation.playing = !state.viewer.animation.playing;
  if (state.viewer.animation.playing && state.viewer.animation.time >= state.viewer.scene.animations[state.viewer.animation.clipIndex].duration) {
    state.viewer.seekAnimation(0);
  }
  updateAnimationPlayButton();
  return true;
}

/** Space plays and pauses, unless it belongs to the focused control. */
export function handlePlaybackShortcut(event: KeyboardEvent) {
  if (event.code !== 'Space' || event.repeat) return;
  if (event.ctrlKey || event.altKey || event.metaKey) return;
  const target = event.target;
  if (target instanceof HTMLElement) {
    if (target.isContentEditable) return;
    // Space is the activation key for buttons, checkboxes and text fields,
    // and the clip listbox picks a clip with it.
    if (/^(BUTTON|INPUT|SELECT|TEXTAREA)$/.test(target.tagName)) return;
    if (!animClipMenu.hidden && animClipMenu.contains(target)) return;
  }
  if (toggleAnimationPlayback()) event.preventDefault();
}

export function updateAnimationPlayButton() {
  if (!state.viewer || !state.viewer.scene?.animations?.length) return;
  const playing = state.viewer.animation.playing;
  animPlayBtn.classList.toggle('active', playing);
  animPlayBtn.title = playing ? 'Pause' : 'Play';
  animPlayBtn.setAttribute('aria-label', playing ? 'Pause animation' : 'Play animation');
}

/**
 * Follows playback while it is running.
 *
 * Only while it is running: a paused clip's time does not change, and the
 * paths that move it — the scrub handle, clip selection, the play button —
 * refresh the bar themselves. Idle frames therefore touch no DOM at all.
 */
export function animationTick() {
  if (state.viewer?.animation.playing) updateAnimationScrub();
  requestAnimationFrame(animationTick);
}

let lastTimeLabel = '';
let lastScrubValue = '';

export function updateAnimationScrub() {
  if (!state.viewer || !state.viewer.scene?.animations?.length) return;
  const clip = state.viewer.scene.animations[state.viewer.animation.clipIndex];
  if (!clip) return;
  // Written only on change: at 60 Hz and above most frames land on the same
  // hundredth of a second, and assigning an input's value moves the caret
  // and invalidates layout whether or not the text differs.
  const label = `${state.viewer.animation.time.toFixed(2)}s / ${clip.duration.toFixed(2)}s`;
  if (label !== lastTimeLabel) {
    animTimeLabel.textContent = label;
    lastTimeLabel = label;
  }
  const scrub = String(Math.round((state.viewer.animation.time / Math.max(clip.duration, 0.0001)) * 1000));
  if (scrub !== lastScrubValue) {
    animScrub.value = scrub;
    lastScrubValue = scrub;
  }
}
