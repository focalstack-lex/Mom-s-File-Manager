// The only door to the native side. In the Tauri shell every call is a
// validated Rust command; in a plain browser (dev harness, UI drive) the
// mock backend answers instead.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { AppInfo, BatchUndo, BrowserStatus, Completed, OrganizePlan, OrganizeResult, Repeat, SubjectRule, CleanupReport, Destination, FindResult, Filters, MoveRequest, MoveResult, OpRow, Preview, Settings, SortedEvent, Task, Template, TemplateCopy, UpdateInfo } from "./types";

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
/** The mock's stand-in for the window's file drag: true while files are over it. */
export const MOCK_DRAG_EVENT = "tote://mock-drag";

/**
 * Whether files from File Explorer are being dragged over the window. The
 * drop itself is filed by the native side (lib.rs), like Move to Tote; the
 * screen only shows where to let go.
 */
export async function onFileDrag(handler: (over: boolean) => void): Promise<UnlistenFn> {
  if (window.__toteMock) return window.__toteMock.listen(MOCK_DRAG_EVENT, (over) => handler(over === true));
  // "enter" carries the paths: a drag of text or a link (no files) shows nothing.
  return getCurrentWebview().onDragDropEvent((e) => {
    if (e.payload.type === "enter") handler(e.payload.paths.length > 0);
    else if (e.payload.type !== "over") handler(false);
  });
}
export type { SortedEvent };

/**
 * Sends one line to tote.log (spec 6.5), so a screen that failed leaves a
 * trace after its toast is gone. A log line that cannot be sent stays a
 * console line: it must never raise a toast of its own.
 */
export function logRenderer(level: "error" | "warn" | "info", message: string): void {
  void invoke<void>("renderer_log", { level, message }).catch((error) => console.warn("Tote: the log line was not sent", error));
}

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
  /** Several files, each to its own folder, as one batch: Activity shows one row with Undo all; files that cannot move are named in `failed`. */
  moveEach: (moves: MoveRequest[]) => invoke<MoveResult>("move_each", { moves }),
  /** With `pickFree`, a taken name becomes the first free "Name (2)"; without it a clash is refused as `alreadyExists`. */
  renameFile: (path: string, newStem: string, pickFree = false) => invoke<string>("rename_file", { path, newStem, pickFree }),
  createFolder: (parent: string, name: string) => invoke<string>("create_folder", { parent, name }),
  recycleFiles: (paths: string[]) => invoke<OpRow[]>("recycle_files", { paths }),
  destinations: () => invoke<Destination[]>("list_destinations"),
  activity: () => invoke<OpRow[]>("list_activity"),
  undo: (id: number) => invoke<void>("undo", { id }),
  cleanup: () => invoke<CleanupReport>("cleanup_report"),
  templates: () => invoke<Template[]>("list_templates"),
  addTemplate: (path: string) => invoke<Template>("add_template", { path }),
  removeTemplate: (id: number) => invoke<void>("remove_template", { id }),
  /** Makes the dated copy; `opened` is false when Windows could not open it, so the toast can say so. */
  useTemplate: (id: number) => invoke<TemplateCopy>("use_template", { id }),
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
  /** The newer version to offer, if any; goes online at most once a day. */
  checkUpdate: () => invoke<UpdateInfo | null>("check_update"),
  /** Downloads, verifies and installs it; Tote closes and reopens. */
  installUpdate: () => invoke<void>("install_update"),
  tasks: () => invoke<Task[]>("list_tasks"),
  addTask: (title: string, due: string | null) => invoke<Task>("add_task", { title, due }),
  updateTask: (id: number, title: string, due: string | null, repeat: Repeat) => invoke<Task>("update_task", { id, title, due, repeat }),
  completeTask: (id: number) => invoke<Completed>("complete_task", { id }),
  reopenTask: (id: number) => invoke<void>("reopen_task", { id }),
  deleteTask: (id: number) => invoke<void>("delete_task", { id }),
  restoreTask: (id: number) => invoke<void>("restore_task", { id }),
  attachFile: (id: number, path: string) => invoke<Task>("attach_file", { id, path }),
  detachFile: (id: number, path: string) => invoke<Task>("detach_file", { id, path }),
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
