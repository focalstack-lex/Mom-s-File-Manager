// The first-run walkthrough (spec 5.9): six steps, each pointing at a real
// control. Pure state here; the overlay draws it.

import { create } from "zustand";

export interface TourStep {
  id: string;
  target: string;
  title: string;
  body: string;
  view: "find" | "cleanup" | "activity" | "settings";
}

export const TOUR_STEPS: TourStep[] = [
  {
    id: "search",
    target: "search",
    title: "Type what you remember",
    body: "A file name, a student, or a word inside the file. Tote looks inside your documents too.",
    view: "find",
  },
  {
    id: "recent",
    target: "recent",
    title: "Your downloads land here",
    body: "Everything you download shows up in this list, newest first. It is already filed for you.",
    view: "find",
  },
  {
    id: "open",
    target: "open",
    title: "Open it",
    body: "One click opens the file. Show in folder takes you to where it lives.",
    view: "find",
  },
  {
    id: "todo",
    target: "nav-todo",
    title: "Your to-do list",
    body: "Write what needs doing. On the morning it is due, Tote reminds you.",
    view: "find",
  },
  {
    id: "cleanup",
    target: "nav-cleanup",
    title: "Tidy up when you have time",
    body: "Clean up gathers duplicate copies and loose files for you. Nothing moves until you click.",
    view: "find",
  },
  {
    id: "activity",
    target: "nav-activity",
    title: "Nothing is ever lost",
    body: "Every change is listed in Activity. Undo puts it back where it was.",
    view: "find",
  },
];

interface TourStore {
  active: boolean;
  index: number;
  start(): void;
  next(): void;
  back(): void;
  finish(): void;
}

export const useTour = create<TourStore>((set, get) => ({
  active: false,
  index: 0,
  start: () => set({ active: true, index: 0 }),
  next: () => {
    if (get().index >= TOUR_STEPS.length - 1) get().finish();
    else set({ index: get().index + 1 });
  },
  back: () => set({ index: Math.max(0, get().index - 1) }),
  finish: () => set({ active: false, index: 0 }),
}));

export interface Box {
  top: number;
  left: number;
  width: number;
  height: number;
}

export type Side = "below" | "above" | "right" | "left";

const MARGIN = 16;

/** How much of `a` lies over `b`, in square pixels. */
function overlapArea(a: Box, b: Box): number {
  const x = Math.min(a.left + a.width, b.left + b.width) - Math.max(a.left, b.left);
  const y = Math.min(a.top + a.height, b.top + b.height) - Math.max(a.top, b.top);
  return x > 0 && y > 0 ? x * y : 0;
}

/**
 * Where the card sits beside its target (the ring's box) so it never covers
 * it: the first side, in order, where the whole card fits inside the window
 * below `minTop` (the title bar) and clear of the ring. A wide target tries
 * below first; a narrow one (a menu item, a button) tries its right first.
 * `fits` is false when no side has room; the walkthrough then scrolls the
 * target to the other end of the screen and asks again, and if nothing
 * helps the card goes where it covers the least.
 */
export function placeCard(target: Box, card: { width: number; height: number }, viewport: { width: number; height: number }, gap = 24, minTop = MARGIN): { top: number; left: number; side: Side; fits: boolean } {
  const top = Math.max(MARGIN, minTop);
  const clampX = (x: number) => Math.max(MARGIN, Math.min(x, viewport.width - MARGIN - card.width));
  const clampY = (y: number) => Math.max(top, Math.min(y, viewport.height - MARGIN - card.height));
  const at: Record<Side, { top: number; left: number }> = {
    below: { top: target.top + target.height + gap, left: clampX(target.left) },
    above: { top: target.top - gap - card.height, left: clampX(target.left) },
    right: { top: clampY(target.top), left: target.left + target.width + gap },
    left: { top: clampY(target.top), left: target.left - gap - card.width },
  };
  const order: Side[] = target.width >= card.width / 2 ? ["below", "right", "above", "left"] : ["right", "above", "left", "below"];
  const inside = (p: { top: number; left: number }) => p.top >= top && p.left >= MARGIN && p.top + card.height <= viewport.height - MARGIN && p.left + card.width <= viewport.width - MARGIN;
  const cover = (p: { top: number; left: number }) => overlapArea({ ...p, ...card }, target);
  const free = order.find((side) => inside(at[side]) && cover(at[side]) === 0);
  if (free) return { ...at[free], side: free, fits: true };
  // No side has room: keep the card on screen where it covers the least.
  const kept = order.map((side) => ({ side, top: clampY(at[side].top), left: clampX(at[side].left) }));
  const best = kept.reduce((a, b) => (cover(b) < cover(a) ? b : a));
  return { ...best, fits: false };
}
