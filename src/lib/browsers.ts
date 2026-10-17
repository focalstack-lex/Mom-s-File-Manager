// Plain-words summaries of the browser download check (spec 5.1.2).

import type { BrowserStatus } from "./types";

/** "Chrome" or "Chrome (Mom)" when a browser has more than one profile. */
export function browserLabel(status: BrowserStatus, all: BrowserStatus[]): string {
  const sameBrowser = all.filter((s) => s.browser === status.browser).length;
  return sameBrowser > 1 ? `${status.browser} (${status.profile})` : status.browser;
}

/** The one-line notice for the Find screen, or null when every browser is fine. */
export function browserNotice(all: BrowserStatus[] | null): string | null {
  if (!all) return null;
  const needing = all.filter((s) => !s.ok);
  if (needing.length === 0) return null;
  const names = [...new Set(needing.map((s) => s.browser))];
  const list = names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
  const verb = names.length === 1 ? "saves" : "save";
  return `${list} ${verb} some downloads outside Downloads, so Recent downloads may miss them.`;
}
