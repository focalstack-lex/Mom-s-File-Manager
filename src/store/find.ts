// The Find screen: what she typed, the chips, the answer, what is selected
// and what is previewed. One request in flight at a time; stale answers are
// dropped.

import { create } from "zustand";
import { ipc } from "../lib/ipc";
import { spanFor, type DateChip } from "../lib/dates";
import type { FileRow, FindResult, Kind, Settings, Template } from "../lib/types";
import { useApp } from "./app";

interface FindStore {
  query: string;
  dateChip: DateChip;
  kinds: Kind[];
  sources: string[];
  moreFilters: boolean;
  result: FindResult | null;
  templates: Template[];
  selected: string[];
  previewPath: string | null;
  loading: boolean;
  setQuery(query: string): void;
  setDateChip(chip: DateChip): void;
  toggleKind(kind: Kind): void;
  toggleSource(source: string): void;
  setMoreFilters(open: boolean): void;
  clearFilters(): void;
  refresh(): Promise<void>;
  loadTemplates(): Promise<void>;
  toggleSelected(path: string): void;
  clearSelection(): void;
  setPreview(path: string | null): void;
  isSearching(): boolean;
}

let requestId = 0;
let debounce: number | undefined;

function settingsNow(): Settings {
  return useApp.getState().info?.settings ?? { startAtLogin: true, notifications: true, tourDone: false, schoolYear: null, quarters: [] };
}

export const useFind = create<FindStore>((set, get) => ({
  query: "",
  dateChip: "all",
  kinds: [],
  sources: [],
  moreFilters: false,
  result: null,
  templates: [],
  selected: [],
  previewPath: null,
  loading: false,
  setQuery: (query) => {
    set({ query });
    window.clearTimeout(debounce);
    debounce = window.setTimeout(() => void get().refresh(), 180);
  },
  setDateChip: (dateChip) => {
    set({ dateChip });
    void get().refresh();
  },
  toggleKind: (kind) => {
    const kinds = get().kinds.includes(kind) ? get().kinds.filter((k) => k !== kind) : [...get().kinds, kind];
    set({ kinds });
    void get().refresh();
  },
  toggleSource: (source) => {
    const sources = get().sources.includes(source) ? get().sources.filter((s) => s !== source) : [...get().sources, source];
    set({ sources });
    void get().refresh();
  },
  setMoreFilters: (moreFilters) => set({ moreFilters }),
  clearFilters: () => {
    set({ dateChip: "all", kinds: [], sources: [] });
    void get().refresh();
  },
  refresh: async () => {
    const id = ++requestId;
    const { query, dateChip, kinds, sources } = get();
    const span = spanFor(dateChip, settingsNow());
    set({ loading: true });
    try {
      const result = await ipc.find(query, { from: span.from, to: span.to, kinds, sources });
      if (id !== requestId) return;
      const live = new Set(allRows(result).map((r) => r.path));
      set({ result, loading: false, selected: get().selected.filter((p) => live.has(p)) });
      const preview = get().previewPath;
      if (preview && !live.has(preview)) set({ previewPath: null });
    } catch (error) {
      if (id !== requestId) return;
      set({ loading: false });
      useApp.getState().fail(error);
    }
  },
  loadTemplates: async () => {
    try {
      set({ templates: await ipc.templates() });
    } catch (error) {
      useApp.getState().fail(error);
    }
  },
  toggleSelected: (path) =>
    set((state) => ({ selected: state.selected.includes(path) ? state.selected.filter((p) => p !== path) : [...state.selected, path] })),
  clearSelection: () => set({ selected: [] }),
  setPreview: (previewPath) => set({ previewPath }),
  isSearching: () => {
    const { query, dateChip, kinds, sources } = get();
    return query.trim() !== "" || dateChip !== "all" || kinds.length > 0 || sources.length > 0;
  },
}));

/** Every row the current answer shows, flattened. */
export function allRows(result: FindResult | null): FileRow[] {
  if (!result) return [];
  const rows: FileRow[] = [...result.pinned, ...result.recent];
  for (const group of result.groups) rows.push(group.latest, ...group.others);
  return rows;
}
