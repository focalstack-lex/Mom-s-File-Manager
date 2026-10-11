// Shapes returned by the Rust commands (src-tauri/src/commands.rs). Keep in
// step with the serde structs; the renderer never invents fields.

export type Kind = "word" | "pdf" | "excel" | "powerpoint" | "images" | "videos" | "other";

export interface FileRow {
  id: number;
  path: string;
  parent: string;
  name: string;
  ext: string;
  kind: Kind;
  size: number;
  modified: number;
  originLabel: string | null;
  downloadedAt: number | null;
  groupKey: string;
  snippet: string | null;
  pinned: boolean;
}

export interface Group {
  latest: FileRow;
  others: FileRow[];
}

export interface Filters {
  from?: number;
  to?: number;
  kinds: Kind[];
  sources: string[];
}

export interface IndexStatus {
  scanning: boolean;
  scanned: number;
  total: number;
  files: number;
}

export interface DateRange {
  name: string;
  start: string;
  end: string;
}

export interface Settings {
  startAtLogin: boolean;
  notifications: boolean;
  tourDone: boolean;
  schoolYear: DateRange | null;
  quarters: DateRange[];
  extraFolders: string[];
  excludedFolders: string[];
}

export interface AppInfo {
  version: string;
  roots: string[];
  removableRoots: string[];
  testMode: boolean;
  schoolFiles: string;
  downloads: string | null;
  settings: Settings;
  index: IndexStatus;
  sortedThisWeek: number;
  recovered: boolean;
}

export interface FindResult {
  groups: Group[];
  pinned: FileRow[];
  orphanPins: string[];
  recent: FileRow[];
  index: IndexStatus;
}

export interface Preview {
  kind: "text" | "image" | "details";
  text: string | null;
  name: string;
  size: number;
  modified: number;
  parent: string;
}

export interface Destination {
  path: string;
  label: string;
  recent: boolean;
}

export interface OpRow {
  id: number;
  kind: "autoSort" | "move" | "rename" | "newFolder" | "recycle" | "copy";
  src: string;
  dst: string | null;
  status: "done" | "failed" | "undone";
  message: string | null;
  automatic: boolean;
  createdAt: number;
}

export interface Suggestion {
  file: FileRow;
  suggestedDir: string;
}

export interface CleanupReport {
  duplicates: Group[];
  desktop: Suggestion[];
  oldDownloads: Suggestion[];
}

export interface Template {
  id: number;
  path: string;
  name: string;
  missing: boolean;
}

export interface DeskError {
  code:
    | "notFound"
    | "permissionDenied"
    | "alreadyExists"
    | "invalidPath"
    | "invalidName"
    | "outsideRoots"
    | "io"
    | "db"
    | "validation"
    | "unsupported";
  message: string;
  path?: string;
}

export interface SortedEvent {
  name: string;
  folder: string;
  path: string;
}
