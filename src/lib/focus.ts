// Keyboard focus helpers shared by the dialog, the walkthrough and the lists.

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])';

/** Keeps Tab and Shift+Tab inside `container`, wrapping at both ends. */
export function trapTab(event: KeyboardEvent, container: HTMLElement): void {
  const items = [...container.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null);
  if (items.length === 0) {
    event.preventDefault();
    return;
  }
  const first = items[0];
  const last = items[items.length - 1];
  const active = document.activeElement;
  if (event.shiftKey && (active === first || !container.contains(active))) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && (active === last || !container.contains(active))) {
    event.preventDefault();
    first.focus();
  }
}

/** Moves focus to the next or previous file row on the screen (arrow keys). */
export function focusSiblingRow(from: HTMLElement, step: 1 | -1): void {
  const rows = [...document.querySelectorAll<HTMLElement>('[data-testid="main"] .row-body:not(:disabled)')];
  const at = rows.indexOf(from);
  const next = rows[at + step];
  if (next) {
    next.focus();
    next.scrollIntoView({ block: "nearest" });
  }
}
