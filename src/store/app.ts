// App-wide state: which view is open, app info from the native side, toasts.

import { create } from "zustand";
import { ipc } from "../lib/ipc";
import { describeError } from "../lib/errors";
import type { AppInfo, Settings } from "../lib/types";

export type View = "find" | "cleanup" | "activity" | "settings";

export interface Toast {
  id: number;
  text: string;
  kind: "info" | "error";
  undoId?: number;
}

interface AppStore {
  view: View;
  info: AppInfo | null;
  toasts: Toast[];
  /** Folder choices changed; the watchers pick them up after a restart. */
  foldersChanged: boolean;
  setView(view: View): void;
  markFoldersChanged(): void;
  loadInfo(): Promise<void>;
  saveSettings(settings: Settings): Promise<void>;
  toast(text: string, kind?: Toast["kind"], undoId?: number): void;
  fail(error: unknown): void;
  dismissToast(id: number): void;
}

let nextToast = 1;

export const useApp = create<AppStore>((set, get) => ({
  view: "find",
  info: null,
  toasts: [],
  foldersChanged: false,
  setView: (view) => set({ view }),
  markFoldersChanged: () => set({ foldersChanged: true }),
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
  toast: (text, kind = "info", undoId) => {
    const id = nextToast++;
    set((state) => ({ toasts: [...state.toasts.slice(-2), { id, text, kind, undoId }] }));
    window.setTimeout(() => get().dismissToast(id), kind === "error" ? 9000 : 6000);
  },
  fail: (error) => {
    console.warn("Desk:", error);
    get().toast(describeError(error), "error");
  },
  dismissToast: (id) => set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) })),
}));
