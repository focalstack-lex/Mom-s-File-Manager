// Show a little, offer more (spec 5.10). Long lists open short and grow on
// request, so a full laptop never greets her with a wall of files. The data
// is all loaded; only what is drawn is limited.

import { useState } from "react";

/** How many rows each list shows first, and how many each "Show more" adds. */
export const PAGE = {
  recent: 10,
  results: 20,
  cleanup: 10,
  activity: 10,
  todo: 20,
} as const;

/** A Clean up card starts open only when it is this short; longer cards start folded with a count. */
export const OPEN_CARD_MAX = 5;

/** Activity opens with Today and Yesterday, but never fewer or more rows than this. */
export const ACTIVITY_FIRST = { min: 10, max: 30 } as const;

export interface DayGroup<T> {
  title: string;
  items: T[];
}

/** "Today", "Yesterday", "Earlier this week", "Earlier": the order she remembers things in. */
export function dayTitle(unixSeconds: number, now: Date = new Date()): string {
  const start = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((start(now) - start(new Date(unixSeconds * 1000))) / 86_400_000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return "Earlier this week";
  return "Earlier";
}

/** Groups an already newest-first list into consecutive day groups. */
export function groupByDay<T>(items: T[], when: (item: T) => number, now: Date = new Date()): DayGroup<T>[] {
  const groups: DayGroup<T>[] = [];
  for (const item of items) {
    const title = dayTitle(when(item), now);
    const last = groups[groups.length - 1];
    if (last && last.title === title) last.items.push(item);
    else groups.push({ title, items: [item] });
  }
  return groups;
}

/** The first `limit` rows across the groups, keeping each group's heading. */
export function takeRows<T>(groups: DayGroup<T>[], limit: number): DayGroup<T>[] {
  const out: DayGroup<T>[] = [];
  let left = limit;
  for (const group of groups) {
    if (left <= 0) break;
    const items = group.items.slice(0, left);
    out.push({ title: group.title, items });
    left -= items.length;
  }
  return out;
}

/** Activity's first page: everything from Today and Yesterday, kept between the bounds. */
export function activityFirstPage<T>(groups: DayGroup<T>[]): number {
  const recent = groups.filter((g) => g.title === "Today" || g.title === "Yesterday").reduce((n, g) => n + g.items.length, 0);
  return Math.min(ACTIVITY_FIRST.max, Math.max(ACTIVITY_FIRST.min, recent));
}

/** The button text for the next page, or null when everything is shown. */
export function moreLabel(shown: number, total: number, step: number): string | null {
  const left = total - shown;
  if (left <= 0) return null;
  return `Show ${Math.min(step, left)} more`;
}

/**
 * How many rows to draw, growing by `step` on request. Changing `resetKey`
 * (a new search, a different filter) starts again from the first page;
 * background refreshes keep the same key, so what she opened stays open.
 */
export function useShown(first: number, step: number, resetKey = ""): [number, () => void] {
  const [state, setState] = useState({ key: resetKey, shown: first });
  if (state.key !== resetKey) setState({ key: resetKey, shown: first });
  const shown = state.key === resetKey ? state.shown : first;
  return [shown, () => setState((s) => ({ key: s.key, shown: s.shown + step }))];
}
