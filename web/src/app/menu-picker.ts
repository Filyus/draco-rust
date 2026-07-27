/**
 * A dropdown that looks like the rest of the viewport, over a real `<select>`.
 *
 * A native select cannot be styled where it matters: the popup is drawn by the
 * platform, so it arrives in the OS palette on top of a dark viewport. The
 * animation bar solved that by keeping the select for its state and drawing the
 * list itself. The variant picker, added later, did not — it sat in the toolbar
 * as a bare native control with a floating label, in a different palette and a
 * different height from every button beside it.
 *
 * So the mechanics live here rather than in either panel. The select stays the
 * single source of truth — it holds the value, it fires `change`, and a test or
 * a screen reader still sees an ordinary form control — and this draws a listbox
 * over it and keeps the two in step.
 */

export interface MenuPicker {
  /** Rebuild the option buttons from the select's current options. */
  rebuild(): void;
  /** Reflect the select's value into the trigger label and the option list. */
  sync(): void;
  open(): void;
  close(restoreFocus?: boolean): void;
  /** Move the selection by absolute index, wrapping at both ends. */
  selectAt(index: number): void;
  handleTriggerKeydown(event: KeyboardEvent): void;
  handleMenuKeydown(event: KeyboardEvent): void;
  /** Attach the listeners that make the control work, including the outside-click close. */
  install(): void;
}

export interface MenuPickerElements {
  select: HTMLSelectElement;
  trigger: HTMLButtonElement;
  label: HTMLElement;
  menu: HTMLElement;
  /** Shown on the trigger before anything is selected. */
  placeholder: string;
  /**
   * Names the control on the trigger, ahead of the chosen value.
   *
   * For a control whose values do not say what they are: on its own,
   * "Default" reads as a placeholder rather than as the variant a file
   * actually named, and gives no clue what it is the default of.
   */
  prefix?: string;
  /** Prefix for option element ids, which `aria-activedescendant` points at. */
  optionId: string;
}

/** Gap between the control and its list, matching the CSS default. */
const MENU_GAP = 6;

/**
 * The nearest ancestor that clips its overflow, or the viewport.
 *
 * An absolutely positioned menu is clipped by a scrolling ancestor just as
 * surely as by the window — the sidebar scrolls, so a list long enough to pass
 * its bottom edge simply loses its last rows.
 */
function clippingBounds(element: HTMLElement) {
  for (let node = element.parentElement; node; node = node.parentElement) {
    const { overflow, overflowY } = getComputedStyle(node);
    if (/auto|scroll|hidden/.test(overflow) || /auto|scroll|hidden/.test(overflowY)) {
      const rect = node.getBoundingClientRect();
      return { top: rect.top, bottom: rect.bottom };
    }
  }
  return { top: 0, bottom: window.innerHeight };
}

/**
 * Put the list where it fits: below the control, above it when below is too
 * tight, and never taller than the room it has.
 *
 * A fixed max-height cannot know the room — the same control sits near the top
 * of a scrolling sidebar in one panel and at the bottom of the viewport in
 * another. Measured on open, both cases fall out of one rule, and the playback
 * bar no longer needs a rule of its own saying "this one opens upward".
 */
function placeMenu(trigger: HTMLElement, menu: HTMLElement) {
  const anchor = trigger.getBoundingClientRect();
  const bounds = clippingBounds(menu);
  const below = bounds.bottom - anchor.bottom - MENU_GAP;
  const above = anchor.top - bounds.top - MENU_GAP;
  // Its natural height, before any cap from a previous opening.
  menu.style.maxHeight = 'none';
  const wanted = menu.getBoundingClientRect().height;
  const openUp = below < Math.min(wanted, above);
  menu.style.top = openUp ? 'auto' : `calc(100% + ${MENU_GAP}px)`;
  menu.style.bottom = openUp ? `calc(100% + ${MENU_GAP}px)` : 'auto';
  menu.style.maxHeight = `${Math.max(80, Math.floor(openUp ? above : below))}px`;
}

export function createMenuPicker({
  select, trigger, label, menu, placeholder, optionId, prefix,
}: MenuPickerElements): MenuPicker {
  const options = () => [...menu.querySelectorAll<HTMLElement>('.menu-picker-option')];

  const commit = (value: string) => {
    select.value = value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
  };

  const picker: MenuPicker = {
    rebuild() {
      // Only when the options actually changed. Replacing them unconditionally
      // destroyed the button holding focus, and the caller cannot avoid that:
      // choosing an option reloads the preview, the preview re-renders the
      // panel, and the panel syncs the picker. So a keyboard user pressing
      // Down committed a choice, lost focus to the body, and found the control
      // dead until they clicked it again.
      const wanted = [...select.options].map((option) => option.value);
      const current = options().map((option) => option.dataset.value);
      if (wanted.length === current.length && wanted.every((value, index) => value === current[index])) {
        picker.sync();
        return;
      }
      menu.replaceChildren(...[...select.options].map((option) => {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'menu-picker-option';
        button.dataset.value = option.value;
        button.id = `${optionId}-${option.value}`;
        button.tabIndex = -1;
        button.setAttribute('role', 'option');
        button.textContent = option.textContent;
        // The row truncates rather than wrapping, so the full name has to stay
        // readable somewhere.
        button.title = option.textContent ?? '';
        button.addEventListener('click', () => {
          commit(option.value);
          picker.close(true);
        });
        return button;
      }));
      picker.sync();
    },

    sync() {
      const chosen = select.selectedOptions[0]?.textContent;
      label.textContent = chosen ? (prefix ? `${prefix}: ${chosen}` : chosen) : placeholder;
      for (const option of options()) {
        const active = option.dataset.value === select.value;
        option.classList.toggle('selected', active);
        option.setAttribute('aria-selected', String(active));
        if (active) trigger.setAttribute('aria-activedescendant', option.id);
      }
    },

    open() {
      trigger.setAttribute('aria-expanded', 'true');
      menu.hidden = false;
      placeMenu(trigger, menu);
      (menu.querySelector<HTMLElement>('.menu-picker-option.selected')
        || menu.querySelector<HTMLElement>('.menu-picker-option'))?.focus();
    },

    close(restoreFocus = false) {
      menu.style.removeProperty('top');
      menu.style.removeProperty('bottom');
      menu.style.removeProperty('maxHeight');
      // Hiding the menu while one of its options holds focus would drop focus
      // to the body, so the trigger takes it back — but only then, otherwise
      // closing would steal focus from whatever the user just clicked.
      const hadFocus = menu.contains(document.activeElement);
      trigger.setAttribute('aria-expanded', 'false');
      menu.hidden = true;
      if (restoreFocus || hadFocus) trigger.focus();
    },

    selectAt(index: number) {
      const all = [...select.options];
      if (all.length === 0) return;
      commit(all[(index + all.length) % all.length].value);
    },

    handleTriggerKeydown(event: KeyboardEvent) {
      const all = [...select.options];
      if (all.length === 0) return;
      const current = Math.max(0, all.findIndex((option) => option.value === select.value));
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault();
        picker.selectAt(current + (event.key === 'ArrowDown' ? 1 : -1));
      } else if (event.key === 'Home' || event.key === 'End') {
        event.preventDefault();
        picker.selectAt(event.key === 'Home' ? 0 : all.length - 1);
      } else if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        picker.open();
      }
    },

    handleMenuKeydown(event: KeyboardEvent) {
      const all = options();
      const current = all.indexOf(document.activeElement as HTMLElement);
      if (event.key === 'Escape') {
        event.preventDefault();
        picker.close(true);
        return;
      }
      let next = current;
      if (event.key === 'ArrowDown') next = (current + 1) % all.length;
      else if (event.key === 'ArrowUp') next = (current - 1 + all.length) % all.length;
      else if (event.key === 'Home') next = 0;
      else if (event.key === 'End') next = all.length - 1;
      else return;
      event.preventDefault();
      const target = all[next];
      if (!target) return;
      target.focus();
      commit(target.dataset.value!);
    },

    install() {
      trigger.addEventListener('click', (event) => {
        event.stopPropagation();
        if (trigger.getAttribute('aria-expanded') !== 'true') picker.open();
        else picker.close();
      });
      menu.addEventListener('click', (event) => event.stopPropagation());
      trigger.addEventListener('keydown', picker.handleTriggerKeydown);
      menu.addEventListener('keydown', picker.handleMenuKeydown);
      // Wrapped: passing close directly would hand the event in as a truthy
      // `restoreFocus`, so every click in the page focused the trigger.
      document.addEventListener('click', () => picker.close());
      document.addEventListener('keydown', (event) => {
        if (event.key === 'Escape') picker.close();
      });
    },
  };
  return picker;
}
