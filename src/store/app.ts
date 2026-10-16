// App-wide state: which view is open, app info from the native side, toasts.

import { create } from "zustand";
import { ipc } from "../lib/ipc";
import { describeError } from "../lib/errors";
import type { AppInfo, BrowserStatus, Settings, UpdateInfo } from "../lib/types";

export type View = "find" | "todo" | "cleanup" | "activity" | "settings";

export interface Toast {
  id: number;
  text: string;
  kind: "info" | "error";
  /** Activity entries the toast's Undo button reverses. */
  undoIds?: number[];
  /** An Organize run the toast's Undo reverses as a whole. */
  undoBatch?: string;
  /** A task the toast's Undo puts back: ticked off (reopen) or deleted (restore). */
  undoTask?: { action: "reopen" | "restore"; id: number };
}

interface AppStore {
  view: View;
  info: AppInfo | null;
  /** The first load of app info failed; the shell shows this sentence with Try again. */
  infoError: string | null;
  toasts: Toast[];
  /** Folder choices changed; the watchers pick them up after a restart. */
  foldersChanged: boolean;
  /** Browser download settings; null until the first check returns. */
  browsers: BrowserStatus[] | null;
  /** A newer Tote, once the daily check finds one. */
  update: UpdateInfo | null;
  /** She chose Later; the banner returns the next time Tote starts. */
  updateDismissed: boolean;
  updating: boolean;
  setView(view: View): void;
  markFoldersChanged(): void;
  checkBrowsers(): Promise<void>;
  checkUpdate(): Promise<void>;
  installUpdate(): Promise<void>;
  dismissUpdate(): void;
  loadInfo(): Promise<void>;
  /** False when the native side could not save; the caller shows the old value again. */
  saveSettings(settings: Settings): Promise<boolean>;
  toast(text: string, kind?: Toast["kind"], undoIds?: number[], undoBatch?: string, undoTask?: Toast["undoTask"]): void;
  fail(error: unknown): void;
  dismissToast(id: number): void;
}

let nextToast = 1;

export const useApp = create<AppStore>((set, get) => ({
  view: "find",
  info: null,
  infoError: null,
  toasts: [],
  foldersChanged: false,
  browsers: null,
  update: null,
  updateDismissed: false,
  updating: false,
  setView: (view) => set({ view }),
  markFoldersChanged: () => set({ foldersChanged: true }),
  checkBrowsers: async () => {
    try {
      set({ browsers: await ipc.browserCheck() });
    } catch (error) {
      // The check is advice only; a failure is logged and the notice stays hidden.
      console.warn("Tote: browser check failed", error);
      set({ browsers: [] });
    }
  },
  checkUpdate: async () => {
    try {
      set({ update: await ipc.checkUpdate() });
    } catch (error) {
      // Offline or the site is down: nothing to offer today. Logged, not shown;
      // the native side tries again on the next day's check.
      console.warn("Tote: update check failed", error);
    }
  },
  installUpdate: async () => {
    set({ updating: true });
    try {
      // On success Windows closes Tote and opens the new version, so this
      // only returns on failure (or in a dry run).
      await ipc.installUpdate();
    } catch (error) {
      get().fail(error);
    } finally {
      set({ updating: false });
    }
  },
  dismissUpdate: () => set({ updateDismissed: true }),
  loadInfo: async () => {
    try {
      const info = await ipc.appInfo();
      set({ info, infoError: null });
    } catch (error) {
      // Before the first answer there is no screen to put a toast on: the
      // shell shows the sentence in place with Try again. Later refreshes toast.
      if (get().info === null) {
        console.warn("Tote:", error);
        set({ infoError: describeError(error) });
      } else get().fail(error);
    }
  },
  saveSettings: async (settings) => {
    try {
      const saved = await ipc.setSettings(settings);
      const info = get().info;
      if (info) set({ info: { ...info, settings: saved } });
      return true;
    } catch (error) {
      get().fail(error);
      return false;
    }
  },
  toast: (text, kind = "info", undoIds, undoBatch, undoTask) => {
    const id = nextToast++;
    set((state) => ({ toasts: [...state.toasts.slice(-2), { id, text, kind, undoIds, undoBatch, undoTask }] }));
    // A toast that can be undone stays long enough to reach the button.
    window.setTimeout(() => get().dismissToast(id), kind === "error" ? 9000 : undoIds?.length || undoBatch || undoTask ? 10000 : 6000);
  },
  fail: (error) => {
    console.warn("Tote:", error);
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
    console.warn("Tote: could not read Activity for Undo", error);
    return [];
  }
}
