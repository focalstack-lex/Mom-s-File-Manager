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

/** The search box on Find, the last resort after a change took away what had focus. */
export const SEARCH_BOX = '[data-testid="search-input"]';

/** The open view's title, the last resort on the screens without a search box. */
export const VIEW_TITLE = '[data-testid="main"] .view-title';

/** The button of the file row for `path` on the Find screen. */
export function rowTarget(path: string): string {
  return `[data-testid="main"] [data-testid="file-row"][data-path="${CSS.escape(path)}"] .row-body`;
}

/** The paths of the file rows on the Find screen, top to bottom, each once. */
function rowPaths(): string[] {
  const paths = [...document.querySelectorAll<HTMLElement>('[data-testid="main"] [data-testid="file-row"]')].map((row) => row.dataset.path ?? "");
  return [...new Set(paths.filter(Boolean))];
}

/**
 * The row to land on once `gone` leave the list: the first row after them,
 * else the last one before them. Read before the change, while the rows are
 * still on screen.
 */
export function neighbourRows(gone: string[]): string[] {
  const paths = rowPaths();
  const leaving = new Set(gone);
  const at = paths.findIndex((p) => leaving.has(p));
  if (at < 0) return [];
  const after = paths.slice(at).filter((p) => !leaving.has(p));
  const before = paths.slice(0, at).reverse();
  return [...after.slice(0, 1), ...before.slice(0, 1)].map(rowTarget);
}

/**
 * After a change re-drew the screen, focus the first of `targets` (CSS
 * selectors, in order of preference) that is on screen; the search box and
 * then the view's title are the last resorts, so focus never falls back to
 * the page. Waits two frames, so React has drawn the new state.
 */
export function focusSoon(...targets: (string | null | undefined)[]): void {
  const wanted = [...targets.filter((t): t is string => Boolean(t)), SEARCH_BOX, VIEW_TITLE];
  window.requestAnimationFrame(() =>
    window.requestAnimationFrame(() => {
      for (const selector of wanted) {
        const el = document.querySelector<HTMLElement>(selector);
        if (el && el.offsetParent !== null && !el.matches(":disabled")) {
          // A title is not focusable by itself; -1 lets a script focus it while Tab passes it by.
          if (el.tabIndex < 0 && !el.hasAttribute("tabindex")) el.setAttribute("tabindex", "-1");
          el.focus();
          el.scrollIntoView({ block: "nearest" });
          return;
        }
      }
    }),
  );
}

/** Like focusSoon, but only when the change left focus on the page itself (the control she used is gone). */
export function refocusIfLost(...targets: (string | null | undefined)[]): void {
  window.requestAnimationFrame(() =>
    window.requestAnimationFrame(() => {
      const active = document.activeElement;
      if (!active || active === document.body) focusSoon(...targets);
    }),
  );
}

/**
 * After switching views, bring one card of the new view to the top of the
 * screen with focus on its title: How to fix lands on the Browsers card, the
 * quarter note on the School year card.
 */
export function revealCard(testId: string): void {
  window.requestAnimationFrame(() =>
    window.requestAnimationFrame(() => {
      const card = document.querySelector<HTMLElement>(`[data-testid="${testId}"]`);
      if (!card) return;
      const title = card.querySelector<HTMLElement>(".card-title") ?? card;
      if (!title.hasAttribute("tabindex")) title.setAttribute("tabindex", "-1");
      title.focus({ preventScroll: true });
      card.scrollIntoView({ block: "start" });
    }),
  );
}
