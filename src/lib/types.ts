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
  /** Copies of the latest: the same size, or a name with a copy mark such as "(2)" or "final". */
  others: FileRow[];
  /** Files that only share the name and are different files; never offered for the Recycle Bin. */
  sameName: FileRow[];
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
  /** Empty means the standard DepEd list. */
  subjectRules: SubjectRule[];
  /** Once a day, ask codewithlex.site whether a newer Tote exists. */
  checkUpdates: boolean;
  /** She has opened To do once; the sidebar's New marker goes. */
  todoSeen: boolean;
}

/** A newer version offered by the update check. */
export interface UpdateInfo {
  version: string;
  notes: string | null;
}

export interface SubjectRule {
  folder: string;
  words: string[];
}

export interface PlannedMove {
  path: string;
  name: string;
  ext: string;
  kind: Kind;
  modified: number;
  destDir: string;
  reason: string;
}

export interface FolderCount {
  dir: string;
  count: number;
  exists: boolean;
}

export interface OrganizePlan {
  moves: PlannedMove[];
  folders: FolderCount[];
  skipped: number;
}

export interface OrganizeResult {
  batch: string;
  moved: number;
  folders: number;
  failed: string[];
}

export interface BatchUndo {
  undone: number;
  /** A file that could not go back, with the sentence for why. */
  failed: { name: string; message: string }[];
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
  /** tote.sqlite was rebuilt on this start; kept says whether her settings, pins, templates and Activity came along. */
  rebuilt: { kept: boolean } | null;
  /** A damaged to-do list was replaced on this start. */
  tasksRestored: "snapshot" | "empty" | null;
  /** The To do store could not be opened on this start. */
  tasksUnavailable: boolean;
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
  /** Set when the change was part of an Organize run. */
  batch: string | null;
  /** The file came from the web and landed on a drive that cannot keep that mark (a FAT32 or exFAT flash drive). */
  stampLost: boolean;
}

/** One file for `move_each`: where it is and the folder it goes to. */
export interface MoveRequest {
  path: string;
  destination: string;
}

/** A file `move_each` left where it was, with the plain reason. */
export interface MoveFailure {
  name: string;
  message: string;
}

/** What `move_each` did: one batch for several moves, the new paths, and the files that stayed. */
export interface MoveResult {
  batch: string | null;
  moved: string[];
  failed: MoveFailure[];
}

export interface Suggestion {
  file: FileRow;
  suggestedDir: string;
}

export interface CleanupReport {
  duplicates: Group[];
  /** Files that share a name but are different files: shown, never suggested for the Recycle Bin. */
  sameName: Group[];
  desktop: Suggestion[];
  oldDownloads: Suggestion[];
}

export interface Template {
  id: number;
  path: string;
  name: string;
  missing: boolean;
}

/** A dated copy made from a template; `opened` says whether Windows opened it. */
export interface TemplateCopy {
  path: string;
  opened: boolean;
}

export interface ToteError {
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
    | "network"
    | "unsupported"
    | "inUse"
    | "notInRecycleBin"
    | "noRecycleBin"
    | "updateSignature"
    | "updateInstall";
  message: string;
  path?: string;
}

export interface BrowserStatus {
  browser: string;
  profile: string;
  ok: boolean;
  asks: boolean;
  folder: string | null;
  advice: string;
}

/** What Move to Tote filed: the first file, its folder, and how many went. */
export interface SortedEvent {
  name: string;
  folder: string;
  path: string;
  count: number;
}

export type Repeat = "never" | "week" | "month" | "quarter";

/** A file attached to a task; `missing` when it is no longer where it was. */
export interface TaskFile {
  path: string;
  name: string;
  ext: string;
  kind: Kind;
  missing: boolean;
}

export interface Task {
  id: number;
  title: string;
  /** YYYY-MM-DD, or null for no date. */
  due: string | null;
  repeat: Repeat;
  doneAt: number | null;
  createdAt: number;
  files: TaskFile[];
}

export interface Completed {
  task: Task;
  next: Task | null;
  /** A quarterly task in the last quarter Settings knows about. */
  noNextQuarter: boolean;
}
