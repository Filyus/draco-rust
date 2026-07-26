import { animClipLabel, animClipMenu, animClipSelect, animClipTrigger, animPlayBtn, animScrub, animSpeed, animSpeedValue, animTimeLabel, viewerAnimation } from './dom.ts';
import { state } from './state.ts';

/**
 * The animation bar: clip menu, playback, scrub and their keyboard handling.
 *
 * The viewer owns playback state; this only drives it and reflects it back, so
 * the bar and the viewport can never disagree about what is playing.
 */

export function updateAnimationUi(scene: any) {
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

export function rebuildAnimationClipMenu() {
    animClipMenu.replaceChildren();
    for (const option of animClipSelect.options) {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'anim-clip-option';
        button.dataset.value = option.value;
        button.id = `anim-clip-option-${option.value}`;
        button.tabIndex = -1;
        button.setAttribute('role', 'option');
        button.textContent = option.textContent;
        button.addEventListener('click', () => {
            animClipSelect.value = option.value;
            animClipSelect.dispatchEvent(new Event('change', { bubbles: true }));
            closeAnimationClipMenu(true);
        });
        animClipMenu.appendChild(button);
    }
    syncAnimationClipSelection();
}

export function syncAnimationClipSelection() {
    const selected = animClipSelect.selectedOptions[0];
    animClipLabel.textContent = selected?.textContent || 'Animation';
    for (const option of animClipMenu.querySelectorAll<HTMLElement>('.anim-clip-option')) {
        const active = option.dataset.value === animClipSelect.value;
        option.classList.toggle('selected', active);
        option.setAttribute('aria-selected', String(active));
        if (active) animClipTrigger.setAttribute('aria-activedescendant', option.id);
    }
}

export function openAnimationClipMenu() {
    animClipTrigger.setAttribute('aria-expanded', 'true');
    animClipMenu.hidden = false;
    const selected = animClipMenu.querySelector<HTMLElement>('.anim-clip-option.selected')
        || animClipMenu.querySelector<HTMLElement>('.anim-clip-option');
    selected?.focus();
}

export function closeAnimationClipMenu(restoreFocus = false) {
    // Hiding the menu while one of its options holds focus would drop focus to
    // the body, so the trigger takes it back — but only then, otherwise closing
    // would steal focus from whatever the user just clicked.
    const hadFocus = animClipMenu.contains(document.activeElement);
    animClipTrigger.setAttribute('aria-expanded', 'false');
    animClipMenu.hidden = true;
    if (restoreFocus || hadFocus) animClipTrigger.focus();
}

export function selectAnimationClipAt(index: number) {
    const options = [...animClipSelect.options];
    if (options.length === 0) return;
    const wrapped = (index + options.length) % options.length;
    animClipSelect.value = options[wrapped].value;
    animClipSelect.dispatchEvent(new Event('change', { bubbles: true }));
}

export function handleAnimationClipTriggerKeydown(event: KeyboardEvent) {
    const options = [...animClipSelect.options];
    if (options.length === 0) return;
    const current = Math.max(0, options.findIndex(option => option.value === animClipSelect.value));
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault();
        selectAnimationClipAt(current + (event.key === 'ArrowDown' ? 1 : -1));
    } else if (event.key === 'Home' || event.key === 'End') {
        event.preventDefault();
        selectAnimationClipAt(event.key === 'Home' ? 0 : options.length - 1);
    } else if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        openAnimationClipMenu();
    }
}

export function handleAnimationClipMenuKeydown(event: KeyboardEvent) {
    const options = [...animClipMenu.querySelectorAll<HTMLElement>('.anim-clip-option')];
    const current = options.indexOf(document.activeElement as HTMLElement);
    if (event.key === 'Escape') {
        event.preventDefault();
        closeAnimationClipMenu(true);
        return;
    }
    let next = current;
    if (event.key === 'ArrowDown') next = (current + 1) % options.length;
    else if (event.key === 'ArrowUp') next = (current - 1 + options.length) % options.length;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = options.length - 1;
    else return;
    event.preventDefault();
    options[next]?.focus();
    if (options[next]) {
        animClipSelect.value = options[next].dataset.value!;
        animClipSelect.dispatchEvent(new Event('change', { bubbles: true }));
    }
}

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

// Animation scrub/timeline ticker — bound to the render loop via rAF.
export function animationTick() {
    if (state.viewer && state.viewer.scene?.animations?.length && state.viewer.animation.clipIndex >= 0) {
        updateAnimationScrub();
    }
    requestAnimationFrame(animationTick);
}

export function updateAnimationScrub() {
    if (!state.viewer || !state.viewer.scene?.animations?.length) return;
    const clip = state.viewer.scene.animations[state.viewer.animation.clipIndex];
    if (!clip) return;
    animTimeLabel.textContent = `${state.viewer.animation.time.toFixed(2)}s / ${clip.duration.toFixed(2)}s`;
    animScrub.value = String(Math.round((state.viewer.animation.time / Math.max(clip.duration, 0.0001)) * 1000));
}
