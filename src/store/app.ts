// App-wide state: which view is open, app info from the native side, toasts.

import { create } from "zustand";
import { ipc, logRenderer } from "../lib/ipc";
import { describeError } from "../lib/errors";
import type { AppInfo, BrowserStatus, OpRow, Settings, UpdateInfo } from "../lib/types";

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
  /** Browser download settings; null until the first check returns, or when the last check could not run. */
  browsers: BrowserStatus[] | null;
  /** The last browser check could not run; Settings shows this sentence with Check again. */
  browsersError: string | null;
  /** A newer Tote, once the daily check finds one. */
  update: UpdateInfo | null;
  /** She chose Later; the banner returns the next time Tote starts, and Settings keeps a quiet Update meanwhile. */
  updateDismissed: boolean;
  updating: boolean;
  /** The last install failed: the banner and Settings show the same sentence as the toast, with Try again. */
  updateError: string | null;
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
  /** The pointer is over the toast or focus is in it: its time stops until `releaseToast`. */
  holdToast(id: number): void;
  releaseToast(id: number): void;
}

let nextToast = 1;

/**
 * How long each toast still has, outside the store so a timer is never part
 * of what the screen draws. A toast under the pointer or holding focus waits
 * (holds counts the pointer and focus separately), so she can always reach
 * its Undo.
 */
const timers = new Map<number, { left: number; started: number; handle: number | undefined; holds: number }>();

function runTimer(id: number, dismiss: (id: number) => void): void {
  const timer = timers.get(id);
  if (!timer || timer.holds > 0 || timer.handle !== undefined) return;
  timer.started = Date.now();
  timer.handle = window.setTimeout(() => dismiss(id), Math.max(0, timer.left));
}

function stopTimer(id: number): void {
  const timer = timers.get(id);
  if (!timer || timer.handle === undefined) return;
  window.clearTimeout(timer.handle);
  timer.handle = undefined;
  timer.left -= Date.now() - timer.started;
}

export const useApp = create<AppStore>((set, get) => ({
  view: "find",
  info: null,
  infoError: null,
  toasts: [],
  foldersChanged: false,
  browsers: null,
  browsersError: null,
  update: null,
  updateDismissed: false,
  updating: false,
  updateError: null,
  setView: (view) => set({ view }),
  markFoldersChanged: () => set({ foldersChanged: true }),
  checkBrowsers: async () => {
    try {
      set({ browsers: await ipc.browserCheck(), browsersError: null });
    } catch (error) {
      // The check is advice only: the Find notice stays hidden, and Settings
      // says the check could not run rather than that no browser was found.
      console.warn("Tote: browser check failed", error);
      logRenderer("warn", describeError(error));
      set({ browsers: null, browsersError: describeError(error) });
    }
  },
  checkUpdate: async () => {
    try {
      set({ update: await ipc.checkUpdate() });
    } catch (error) {
      // Offline or the site is down: nothing to offer today. Logged, not shown;
      // the native side tries again on the next day's check.
      console.warn("Tote: update check failed", error);
      logRenderer("warn", describeError(error));
    }
  },
  installUpdate: async () => {
    set({ updating: true, updateError: null });
    try {
      // On success Windows closes Tote and opens the new version, so this
      // only returns on failure (or in a dry run).
      await ipc.installUpdate();
    } catch (error) {
      // One sentence in three places: the toast, the banner and Settings.
      set({ updateError: describeError(error) });
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
        logRenderer("error", describeError(error));
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
    const shown = [...get().toasts.slice(-2), { id, text, kind, undoIds, undoBatch, undoTask }];
    // The oldest of four goes, timer and all.
    for (const old of get().toasts) if (!shown.some((t) => t.id === old.id)) get().dismissToast(old.id);
    set({ toasts: shown });
    // A toast that can be undone stays long enough to reach the button.
    timers.set(id, { left: kind === "error" ? 9000 : undoIds?.length || undoBatch || undoTask ? 10000 : 6000, started: 0, handle: undefined, holds: 0 });
    runTimer(id, get().dismissToast);
  },
  fail: (error) => {
    console.warn("Tote:", error);
    logRenderer("error", describeError(error));
    get().toast(describeError(error), "error");
  },
  dismissToast: (id) => {
    stopTimer(id);
    timers.delete(id);
    set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) }));
  },
  holdToast: (id) => {
    const timer = timers.get(id);
    if (!timer) return;
    timer.holds += 1;
    stopTimer(id);
  },
  releaseToast: (id) => {
    const timer = timers.get(id);
    if (!timer) return;
    timer.holds = Math.max(0, timer.holds - 1);
    runTimer(id, get().dismissToast);
  },
}));

/**
 * The Activity ids of the moves that just put files at `paths`, so a toast can
 * offer Undo. Moves return new paths, not ids; the log is the source of truth.
 * Returns an empty list when the log cannot be read; the toast then simply has
 * no Undo button and Activity still has it.
 */
export async function moveOpIds(paths: string[]): Promise<number[]> {
  return (await movedOps(paths)).map((op) => op.id);
}

/** The Activity rows of the moves that just put files at `paths`; empty when the log cannot be read. */
export async function movedOps(paths: string[]): Promise<OpRow[]> {
  try {
    const wanted = new Set(paths);
    const ops = await ipc.activity();
    return ops.filter((op) => op.kind === "move" && op.status === "done" && op.dst !== null && wanted.has(op.dst));
  } catch (error) {
    console.warn("Tote: could not read Activity for Undo", error);
    logRenderer("warn", describeError(error));
    return [];
  }
}

/**
 * The sentence a move toast adds when a file from the web landed on a drive
 * that cannot keep that mark, such as a FAT32 flash drive (audit T-43), so
 * Office will open it without its Protected View. Empty when none did.
 */
export function stampNote(ops: OpRow[]): string {
  const lost = ops.filter((op) => op.stampLost).length;
  if (lost === 0) return "";
  if (ops.length === 1) return " Windows will no longer mark it as from the internet.";
  if (lost === ops.length) return " Windows will no longer mark them as from the internet.";
  return ` Windows will no longer mark ${lost} of them as from the internet.`;
}
