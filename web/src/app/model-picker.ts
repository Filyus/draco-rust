/**
 * Which model in the selection is open.
 *
 * Hidden whenever there is one, which is every ordinary drop of a single file:
 * the import panel then looks exactly as it always has. A folder is where it
 * earns its place — Khronos ships each model as `glTF/`, `glTF-Binary/`,
 * `glTF-Draco/` and `glTF-KTX-BasisU/`, and the one folder anybody would drop
 * holds four files that are the same scene.
 *
 * A picker rather than a list to choose from up front, for two reasons: it does
 * not stand between the drop and the model, and it lets the choice be changed
 * afterwards, which is the interesting operation. Comparing the Draco variant
 * against the plain one is a thing someone converting assets actually wants.
 */
import { fileModelCaption, fileModelLabel, fileModelMenu, fileModelPicker, fileModelSelect, fileModelTrigger } from './dom.ts';
import { createMenuPicker } from './menu-picker.ts';
import type { IntakeEntry } from './model-intake.ts';

const picker = createMenuPicker({
  select: fileModelSelect,
  trigger: fileModelTrigger,
  label: fileModelLabel,
  menu: fileModelMenu,
  placeholder: 'Model',
  optionId: 'file-model-option',
});

/**
 * The directory every candidate shares, so the list shows what differs.
 *
 * Dropping `DamagedHelmet` makes every path start with it, and repeating the
 * folder name on each row says nothing: what tells the two apart is `glTF/`
 * against `glTF-instancing/`.
 */
export function commonPrefix(paths: string[]): string {
  if (paths.length < 2) return paths[0]?.replace(/[^/]*$/, '') ?? '';
  const segments = paths[0].split('/').slice(0, -1);
  let shared = segments.length;
  for (const path of paths.slice(1)) {
    const other = path.split('/');
    let index = 0;
    while (index < shared && index < other.length - 1 && segments[index] === other[index]) index += 1;
    shared = index;
  }
  return shared === 0 ? '' : `${segments.slice(0, shared).join('/')}/`;
}

/** Offer the models the selection held, or hide the picker when it held one. */
export function syncModelPicker(models: IntakeEntry[]) {
  fileModelPicker.hidden = models.length < 2;
  if (models.length < 2) {
    fileModelSelect.replaceChildren();
    picker.rebuild();
    return;
  }
  const prefix = commonPrefix(models.map((model) => model.path));
  fileModelCaption.textContent = prefix ? `Model in ${prefix.slice(0, -1)}` : 'Model in this folder';
  const wanted = models.map((model) => model.path);
  const current = [...fileModelSelect.options].map((option) => option.value);
  if (current.length !== wanted.length || wanted.some((path, index) => current[index] !== path)) {
    fileModelSelect.replaceChildren(...models.map((model) => {
      const option = document.createElement('option');
      option.value = model.path;
      option.textContent = model.path.slice(prefix.length);
      return option;
    }));
  }
  picker.rebuild();
}

/**
 * Open another model out of the same selection, reading only what it names.
 *
 * The path identifies it; the shell holds the selection and finds the entry.
 * Carrying the entry through the option value would mean the DOM owning a file
 * handle, and the option value is a string.
 */
export function installModelPicker(open: (path: string) => void) {
  picker.install();
  fileModelSelect.addEventListener('change', () => {
    picker.sync();
    open(fileModelSelect.value);
  });
}
