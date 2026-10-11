// App-wide state: which view is open, app info from the native side, toasts.

import { create } from "zustand";
import { ipc } from "../lib/ipc";
import { describeError } from "../lib/errors";
import type { AppInfo, BrowserStatus, Settings } from "../lib/types";

export type View = "find" | "cleanup" | "activity" | "settings";

export interface Toast {
  id: number;
  text: string;
  kind: "info" | "error";
  /** Activity entries the toast's Undo button reverses. */
  undoIds?: number[];
}

interface AppStore {
  view: View;
  info: AppInfo | null;
  toasts: Toast[];
  /** Folder choices changed; the watchers pick them up after a restart. */
  foldersChanged: boolean;
  /** Browser download settings; null until the first check returns. */
  browsers: BrowserStatus[] | null;
  setView(view: View): void;
  markFoldersChanged(): void;
  checkBrowsers(): Promise<void>;
  loadInfo(): Promise<void>;
  saveSettings(settings: Settings): Promise<void>;
  toast(text: string, kind?: Toast["kind"], undoIds?: number[]): void;
  fail(error: unknown): void;
  dismissToast(id: number): void;
}

let nextToast = 1;

export const useApp = create<AppStore>((set, get) => ({
  view: "find",
  info: null,
  toasts: [],
  foldersChanged: false,
  browsers: null,
  setView: (view) => set({ view }),
  markFoldersChanged: () => set({ foldersChanged: true }),
  checkBrowsers: async () => {
    try {
      set({ browsers: await ipc.browserCheck() });
    } catch (error) {
      // The check is advice only; a failure is logged and the notice stays hidden.
      console.warn("Desk: browser check failed", error);
      set({ browsers: [] });
    }
  },
  loadInfo: async () => {
    try {
      const info = await ipc.appInfo();
      set({ info });
    } catch (error) {
      get().fail(error);
    }
  },
  saveSettings: async (settings) => {
    try {
      const saved = await ipc.setSettings(settings);
      const info = get().info;
      if (info) set({ info: { ...info, settings: saved } });
    } catch (error) {
      get().fail(error);
    }
  },
  toast: (text, kind = "info", undoIds) => {
    const id = nextToast++;
    set((state) => ({ toasts: [...state.toasts.slice(-2), { id, text, kind, undoIds }] }));
    // A toast that can be undone stays long enough to reach the button.
    window.setTimeout(() => get().dismissToast(id), kind === "error" ? 9000 : undoIds?.length ? 10000 : 6000);
  },
  fail: (error) => {
    console.warn("Desk:", error);
    get().toast(describeError(error), "error");
  },
  dismissToast: (id) => set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) })),
}));

/**
 * The Activity ids of the moves that just put files at `paths`, so a toast can
 * offer Undo. Moves return new paths, not ids; the log is the source of truth.
 * Returns an empty list when the log cannot be read; the toast then simply has
 * no Undo button and Activity still has it.
 */
export async function moveOpIds(paths: string[]): Promise<number[]> {
  try {
    const wanted = new Set(paths);
    const ops = await ipc.activity();
    return ops.filter((op) => op.kind === "move" && op.status === "done" && op.dst !== null && wanted.has(op.dst)).map((op) => op.id);
  } catch (error) {
    console.warn("Desk: could not read Activity for Undo", error);
    return [];
  }
}
