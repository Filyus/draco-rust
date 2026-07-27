import { viewerVariantLabel, viewerVariantMenu, viewerVariantPicker, viewerVariantSelect, viewerVariantTrigger } from './dom.ts';
import { createMenuPicker } from './menu-picker.ts';
import { loadPreview } from './preview.ts';
import type { SceneDocument } from '../scene-document.ts';
import { state } from './state.ts';

/**
 * Which material set the scene is shown with.
 *
 * The list is exactly what the file declares, and one of them is always in
 * force. There is no entry for "no variant", because `KHR_materials_variants`
 * does not define one: the `material` on a primitive is the fallback for
 * readers that cannot follow the extension, not a choice offered to readers
 * that can. Listing it made a fourth row in a list of three variants, under a
 * name we had to invent — and since authoring tools write the fallback as one
 * of the variants' own materials, that row usually rendered identically to the
 * first real one. A picker whose first two entries draw the same image is
 * telling the user something untrue about the file.
 *
 * Driven by the document rather than by the built preview, and rendered with
 * the rest of the document's summary: read off the finished scene, the control
 * appeared one paint after the node tree it sits above and pushed it down.
 */

const picker = createMenuPicker({
  select: viewerVariantSelect,
  trigger: viewerVariantTrigger,
  label: viewerVariantLabel,
  menu: viewerVariantMenu,
  placeholder: 'Variant',
  optionId: 'viewer-variant-option',
});

/** Offer the document's material variants, or hide the picker when it has none. */
export function syncVariantPicker(sceneDocument: SceneDocument | null) {
  const variants = sceneDocument?.variants ?? [];
  viewerVariantPicker.hidden = variants.length === 0;
  if (variants.length === 0) {
    state.currentVariant = null;
    viewerVariantSelect.replaceChildren();
    picker.rebuild();
    return;
  }
  // Rebuilt only when the list itself changed. Replacing the options resets
  // the control's value on the way, and doing that on every render means doing
  // it in response to the choice the user just made - which is a loop: the
  // picker reloads the preview, the preview re-renders the summary.
  const current = [...viewerVariantSelect.options].map((option) => option.textContent);
  if (current.length !== variants.length || variants.some((name, index) => current[index] !== name)) {
    viewerVariantSelect.replaceChildren(...variants.map((name, index) => {
      const option = document.createElement('option');
      option.value = String(index);
      option.textContent = name;
      return option;
    }));
  }
  // A file that declares variants is meant to be seen through one of them, so
  // the first stands in until the user picks another.
  if (state.currentVariant === null || state.currentVariant >= variants.length) state.currentVariant = 0;
  const selected = String(state.currentVariant);
  if (viewerVariantSelect.value !== selected) viewerVariantSelect.value = selected;
  picker.rebuild();
}

/** Re-read the document under the chosen variant; only materials change. */
export function installVariantPicker() {
  picker.install();
  viewerVariantSelect.addEventListener('change', () => {
    state.currentVariant = Number(viewerVariantSelect.value);
    picker.sync();
    // A variant swaps the materials and nothing else, so the camera and the
    // clip that were on screen stay there. Re-framing here showed the user the
    // same model from somewhere other than where they had put it.
    if (state.currentFileType) void loadPreview(state.currentFileType, { keepView: true });
  });
}
