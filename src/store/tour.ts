// The first-run walkthrough (spec 5.9): five steps, each pointing at a real
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
    body: "A file name, a student, or a word inside the file. Desk looks inside your documents too.",
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

/** Where the card sits relative to its target so it never covers it. */
export function placeCard(
  target: { top: number; left: number; width: number; height: number },
  card: { width: number; height: number },
  viewport: { width: number; height: number },
  gap = 24,
): { top: number; left: number; side: "below" | "above" | "right" | "left" } {
  const fitsBelow = target.top + target.height + gap + card.height <= viewport.height;
  const fitsRight = target.left + target.width + gap + card.width <= viewport.width;
  const clamp = (value: number, max: number) => Math.max(16, Math.min(value, max - 16));
  if (fitsBelow && target.width >= card.width / 2) {
    return { top: target.top + target.height + gap, left: clamp(target.left, viewport.width - card.width), side: "below" };
  }
  if (fitsRight) {
    return { top: clamp(target.top, viewport.height - card.height), left: target.left + target.width + gap, side: "right" };
  }
  if (target.top - gap - card.height >= 0) {
    return { top: target.top - gap - card.height, left: clamp(target.left, viewport.width - card.width), side: "above" };
  }
  return { top: clamp(target.top, viewport.height - card.height), left: clamp(target.left - gap - card.width, viewport.width - card.width), side: "left" };
}
