// The only door to the native side. In the Tauri shell every call is a
// validated Rust command; in a plain browser (dev harness, UI drive) the
// mock backend answers instead.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type {
  AppInfo,
  BatchUndo,
  BrowserStatus,
  OrganizePlan,
  OrganizeResult,
  SubjectRule,
  CleanupReport,
  Destination,
  FindResult,
  Filters,
  OpRow,
  Preview,
  Settings,
  SortedEvent,
  Template,
} from "./types";

export interface MockBackend {
  invoke(command: string, args: Record<string, unknown>): Promise<unknown>;
  listen(event: string, handler: (payload: unknown) => void): () => void;
}

declare global {
  interface Window {
    __toteMock?: MockBackend;
  }
}

function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  if (window.__toteMock) return window.__toteMock.invoke(command, args) as Promise<T>;
  return tauriInvoke<T>(command, args);
}

export async function listen<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  if (window.__toteMock) return window.__toteMock.listen(event, handler as (payload: unknown) => void);
  return tauriListen<T>(event, (e) => handler(e.payload));
}

export const CHANGED_EVENT = "tote://changed";
export const SORTED_EVENT = "tote://sorted";
export type { SortedEvent };

export const ipc = {
  appInfo: () => invoke<AppInfo>("get_app_info"),
  find: (query: string, filters: Filters) => invoke<FindResult>("find", { query, filters }),
  setPinned: (path: string, pinned: boolean) => invoke<void>("set_pinned", { path, pinned }),
  forgetPin: (path: string) => invoke<void>("forget_pin", { path }),
  open: (path: string) => invoke<void>("open_path", { path }),
  reveal: (path: string) => invoke<void>("reveal_path", { path }),
  openSchoolFiles: () => invoke<void>("open_school_files"),
  copyFiles: (paths: string[]) => invoke<void>("copy_files", { paths }),
  preview: (path: string) => invoke<Preview>("preview", { path }),
  moveFiles: (paths: string[], destination: string) => invoke<string[]>("move_files", { paths, destination }),
  renameFile: (path: string, newStem: string) => invoke<string>("rename_file", { path, newStem }),
  createFolder: (parent: string, name: string) => invoke<string>("create_folder", { parent, name }),
  recycleFiles: (paths: string[]) => invoke<OpRow[]>("recycle_files", { paths }),
  destinations: () => invoke<Destination[]>("list_destinations"),
  activity: () => invoke<OpRow[]>("list_activity"),
  undo: (id: number) => invoke<void>("undo", { id }),
  cleanup: () => invoke<CleanupReport>("cleanup_report"),
  templates: () => invoke<Template[]>("list_templates"),
  addTemplate: (path: string) => invoke<Template>("add_template", { path }),
  removeTemplate: (id: number) => invoke<void>("remove_template", { id }),
  useTemplate: (id: number) => invoke<string>("use_template", { id }),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<Settings>("set_settings", { settings }),
  addFolder: (path: string) => invoke<Settings>("add_folder", { path }),
  removeFolder: (path: string) => invoke<Settings>("remove_folder", { path }),
  restart: () => invoke<void>("restart_app"),
  browserCheck: () => invoke<BrowserStatus[]>("browser_check"),
  organizePlan: () => invoke<OrganizePlan>("organize_plan"),
  organizeRun: (paths: string[]) => invoke<OrganizeResult>("organize_run", { paths }),
  undoBatch: (batch: string) => invoke<BatchUndo>("undo_batch", { batch }),
  defaultSubjectRules: () => invoke<SubjectRule[]>("default_subject_rules"),
  /** Native folder picker; null when she cancels. */
  pickFolder: async (): Promise<string | null> => {
    if (window.__toteMock) return (await window.__toteMock.invoke("pick_folder", {})) as string | null;
    const picked = await openDialog({ directory: true, multiple: false, title: "Choose a folder for Tote to look in" });
    return typeof picked === "string" ? picked : null;
  },
  /** Native file picker for templates; null when she cancels. */
  pickFile: async (): Promise<string | null> => {
    if (window.__toteMock) return (await window.__toteMock.invoke("pick_file", {})) as string | null;
    const picked = await openDialog({
      multiple: false,
      title: "Choose a master file for the template shelf",
      filters: [{ name: "Documents", extensions: ["docx", "doc", "xlsx", "xls", "pptx", "ppt", "pdf", "odt", "ods"] }],
    });
    return typeof picked === "string" ? picked : null;
  },
};
