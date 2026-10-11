// A fake native side for the browser harness and the UI drive: a small
// messy folder set, in memory, answering every command the app uses.
// Never bundled into the Tauri build (main.tsx imports it only in DEV).

import type { MockBackend } from "../lib/ipc";
import type { CleanupReport, Destination, FileRow, FindResult, Filters, Group, OpRow, Settings, Template } from "../lib/types";

const DOCS = "C:\\Users\\Teacher\\Documents";
const DESKTOP = "C:\\Users\\Teacher\\Desktop";
const DOWNLOADS = "C:\\Users\\Teacher\\Downloads";
const SCHOOL = `${DOCS}\\School Files`;
const NOW = Math.floor(Date.now() / 1000);
const DAY = 86_400;

function key(name: string): string {
  const stem = name.replace(/\.[^.]+$/, "").toLowerCase().replace(/\(\d+\)/g, "");
  const ext = name.split(".").pop() ?? "";
  const words = stem.split(/[^a-z0-9]+/).filter((w) => w && !["copy", "of", "final", "revised"].includes(w) && !/^v\d+$/.test(w));
  return `${words.join(" ")}|${ext}`;
}

let nextId = 1;
function file(parent: string, name: string, extra: Partial<FileRow> = {}): FileRow {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  const kind = (({ docx: "word", doc: "word", pdf: "pdf", xlsx: "excel", csv: "excel", pptx: "powerpoint", jpg: "images", png: "images", mp4: "videos" }) as Record<string, FileRow["kind"]>)[ext] ?? "other";
  return {
    id: nextId++,
    path: `${parent}\\${name}`,
    parent,
    name,
    ext,
    kind,
    size: 48_000,
    modified: NOW - 3 * DAY,
    originLabel: null,
    downloadedAt: null,
    groupKey: key(name),
    snippet: null,
    pinned: false,
    ...extra,
  };
}

const CONTENT: Record<string, string> = {
  "Book1.xlsx": "Santos Reyes Dela Cruz attendance Grade 7 Mabini",
  "Grade 7 Quiz 2.docx": "Grade 7 Science Quiz 2 Photosynthesis Name Score",
  "DLL Week 5.docx": "Daily Lesson Log Week 5 Grade 7 English",
  "Lesson Plan.docx": "Lesson plan template objectives procedure evaluation",
};

let files: FileRow[] = [
  file(`${SCHOOL}\\Word\\2026-10`, "Grade 7 Quiz 2.docx", { originLabel: "Messenger", downloadedAt: NOW - 2 * 3600, modified: NOW - 2 * 3600 }),
  file(`${SCHOOL}\\PDF\\2026-10`, "DepEd Order 2026-014.pdf", { originLabel: "DepEd", downloadedAt: NOW - DAY, modified: NOW - DAY }),
  file(`${SCHOOL}\\Excel\\2026-10`, "Book1.xlsx", { originLabel: "Gmail", downloadedAt: NOW - 3 * DAY, modified: NOW - 3 * DAY }),
  file(`${SCHOOL}\\Word\\2026-10`, "DLL Week 5.docx", { originLabel: "Google Drive", downloadedAt: NOW - 5 * DAY, modified: NOW - 5 * DAY }),
  file(`${SCHOOL}\\Images\\2026-10`, "IMG_2041.jpg", { originLabel: "Unknown source", downloadedAt: NOW - 6 * DAY, modified: NOW - 6 * DAY }),
  file(DESKTOP, "Lesson Plan.docx", { modified: NOW - 40 * DAY }),
  file(DESKTOP, "Lesson Plan (2).docx", { modified: NOW - 20 * DAY }),
  file(DESKTOP, "Lesson Plan final_final.docx", { modified: NOW - 2 * DAY }),
  file(DESKTOP, "Class Record Grade 7.xlsx", { modified: NOW - 10 * DAY, pinned: true }),
  file(DOWNLOADS, "old attendance.pdf", { modified: NOW - 90 * DAY }),
  file(DOWNLOADS, "setup-printer.exe", { modified: NOW - 120 * DAY }),
  file(`${DOCS}\\Forms`, "SF9 Template.docx", { modified: NOW - 200 * DAY }),
];

const ops: OpRow[] = [
  { id: 1, kind: "autoSort", src: `${DOWNLOADS}\\Grade 7 Quiz 2.docx`, dst: `${SCHOOL}\\Word\\2026-10\\Grade 7 Quiz 2.docx`, status: "done", message: null, automatic: true, createdAt: NOW - 2 * 3600 },
  { id: 2, kind: "autoSort", src: `${DOWNLOADS}\\DepEd Order 2026-014.pdf`, dst: `${SCHOOL}\\PDF\\2026-10\\DepEd Order 2026-014.pdf`, status: "done", message: null, automatic: true, createdAt: NOW - DAY },
  { id: 3, kind: "autoSort", src: `${DOWNLOADS}\\budget.xlsx`, dst: null, status: "failed", message: "The file stayed open in another program for a day, so it was left in Downloads.", automatic: true, createdAt: NOW - 2 * DAY },
];
let nextOp = 4;
let templates: Template[] = [{ id: 1, path: `${DOCS}\\Forms\\SF9 Template.docx`, name: "SF9 Template", missing: false }];
let settings: Settings = { startAtLogin: true, notifications: true, tourDone: false, schoolYear: null, quarters: [], extraFolders: [], excludedFolders: [] };
const listeners = new Map<string, Set<(payload: unknown) => void>>();
const calls: { command: string; args: Record<string, unknown> }[] = [];

function emit(event: string, payload: unknown = null) {
  for (const handler of listeners.get(event) ?? []) handler(payload);
}

function group(rows: FileRow[]): Group[] {
  const order: string[] = [];
  const buckets = new Map<string, FileRow[]>();
  for (const row of rows) {
    if (!buckets.has(row.groupKey)) {
      order.push(row.groupKey);
      buckets.set(row.groupKey, []);
    }
    buckets.get(row.groupKey)?.push(row);
  }
  return order.map((k) => {
    const members = [...(buckets.get(k) ?? [])].sort((a, b) => b.modified - a.modified);
    return { latest: members[0], others: members.slice(1) };
  });
}

function matches(row: FileRow, query: string, filters: Filters): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const hay = `${row.name} ${CONTENT[row.name] ?? ""}`.toLowerCase();
  if (!words.every((w) => hay.includes(w))) return false;
  const when = row.downloadedAt ?? row.modified;
  if (filters.from && when < filters.from) return false;
  if (filters.to && when >= filters.to) return false;
  if (filters.kinds.length && !filters.kinds.includes(row.kind)) return false;
  if (filters.sources.length) {
    const known = ["Messenger", "Gmail", "Google Drive", "DepEd"];
    const label = row.originLabel ?? "";
    const other = filters.sources.includes("Other") && label && !known.includes(label);
    if (!filters.sources.includes(label) && !other) return false;
  }
  return true;
}

function find(query: string, filters: Filters): FindResult {
  const searching = query.trim() !== "" || filters.from !== undefined || filters.to !== undefined || filters.kinds.length > 0 || filters.sources.length > 0;
  const index = { scanning: false, scanned: files.length, total: files.length, files: files.length };
  if (!searching) {
    return { groups: [], pinned: files.filter((f) => f.pinned), orphanPins: [], recent: files.filter((f) => f.downloadedAt).sort((a, b) => (b.downloadedAt ?? 0) - (a.downloadedAt ?? 0)), index };
  }
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const hits = files
    .filter((f) => matches(f, query, filters))
    .map((f) => {
      const inName = words.every((w) => f.name.toLowerCase().includes(w));
      const content = CONTENT[f.name];
      const snippet = !inName && content ? content.replace(new RegExp(`(${words.join("|")})`, "ig"), "[$1]") : null;
      return { ...f, snippet };
    });
  return { groups: group(hits), pinned: [], orphanPins: [], recent: [], index };
}

function moveTo(path: string, destination: string, kind: OpRow["kind"]): string {
  const row = files.find((f) => f.path === path);
  if (!row) throw { code: "notFound", message: "That file is no longer there." };
  let name = row.name;
  let n = 2;
  while (files.some((f) => f.parent === destination && f.name === name)) {
    name = row.name.replace(/(\.[^.]+)$/, ` (${n})$1`);
    n += 1;
  }
  const dst = `${destination}\\${name}`;
  ops.unshift({ id: nextOp++, kind, src: row.path, dst, status: "done", message: null, automatic: false, createdAt: NOW });
  row.path = dst;
  row.parent = destination;
  row.name = name;
  return dst;
}

const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
  get_app_info: () => ({
    version: "0.1.0-mock",
    roots: [DESKTOP, DOCS, DOWNLOADS, ...settings.extraFolders].filter((r) => !settings.excludedFolders.includes(r)),
    removableRoots: [DESKTOP, ...settings.extraFolders].filter((r) => !settings.excludedFolders.includes(r)),
    testMode: false,
    schoolFiles: SCHOOL,
    downloads: DOWNLOADS,
    settings,
    index: { scanning: false, scanned: files.length, total: files.length, files: files.length },
    sortedThisWeek: 5,
    recovered: false,
  }),
  find: (a) => find(String(a.query ?? ""), (a.filters as Filters) ?? { kinds: [], sources: [] }),
  set_pinned: (a) => {
    const row = files.find((f) => f.path === a.path);
    if (row) row.pinned = Boolean(a.pinned);
  },
  forget_pin: () => undefined,
  open_path: () => undefined,
  reveal_path: () => undefined,
  open_school_files: () => undefined,
  copy_files: () => undefined,
  preview: (a) => {
    const row = files.find((f) => f.path === a.path);
    if (!row) throw { code: "notFound", message: "That file is no longer there." };
    const text = CONTENT[row.name];
    return { kind: row.kind === "images" ? "image" : text ? "text" : "details", text: text ?? null, name: row.name, size: row.size, modified: row.modified, parent: row.parent };
  },
  move_files: (a) => (a.paths as string[]).map((p) => moveTo(p, String(a.destination), "move")),
  rename_file: (a) => {
    const row = files.find((f) => f.path === a.path);
    if (!row) throw { code: "notFound", message: "That file is no longer there." };
    const stem = String(a.newStem);
    if (/[\\/:*?"<>|]/.test(stem)) throw { code: "invalidName", message: "Names cannot contain : or similar characters." };
    const name = `${stem}.${row.ext}`;
    if (files.some((f) => f.parent === row.parent && f.name === name && f !== row)) throw { code: "alreadyExists", message: "A file with that name already exists in this folder." };
    ops.unshift({ id: nextOp++, kind: "rename", src: row.path, dst: `${row.parent}\\${name}`, status: "done", message: null, automatic: false, createdAt: NOW });
    row.name = name;
    row.path = `${row.parent}\\${name}`;
    return row.path;
  },
  create_folder: (a) => `${a.parent}\\${a.name}`,
  recycle_files: (a) => {
    const out: OpRow[] = [];
    for (const path of a.paths as string[]) {
      files = files.filter((f) => f.path !== path);
      const op: OpRow = { id: nextOp++, kind: "recycle", src: path, dst: null, status: "done", message: null, automatic: false, createdAt: NOW };
      ops.unshift(op);
      out.push(op);
    }
    return out;
  },
  list_destinations: (): Destination[] => [
    { path: SCHOOL, label: "School Files", recent: false },
    { path: `${SCHOOL}\\Word`, label: "School Files > Word", recent: true },
    { path: `${SCHOOL}\\Word\\2026-10`, label: "School Files > Word > 2026-10", recent: false },
    { path: `${SCHOOL}\\PDF`, label: "School Files > PDF", recent: false },
    { path: `${DOCS}\\Forms`, label: "Documents > Forms", recent: false },
    { path: DESKTOP, label: "Desktop", recent: false },
  ],
  // A fresh array each time, as the real backend returns: React skips a
  // re-render when state is the same object.
  list_activity: () => ops.map((op) => ({ ...op })),
  undo: (a) => {
    const op = ops.find((o) => o.id === a.id);
    if (!op || op.status !== "done") throw { code: "validation", message: "Only completed changes can be undone." };
    if (op.kind === "recycle") files.push(file(op.src.slice(0, op.src.lastIndexOf("\\")), op.src.slice(op.src.lastIndexOf("\\") + 1)));
    else if (op.dst) {
      const row = files.find((f) => f.path === op.dst);
      if (row) {
        row.path = op.src;
        row.parent = op.src.slice(0, op.src.lastIndexOf("\\"));
        row.name = op.src.slice(op.src.lastIndexOf("\\") + 1);
      }
    }
    op.status = "undone";
  },
  cleanup_report: (): CleanupReport => {
    const groups = group(files).filter((g) => g.others.length > 0);
    const suggest = (f: FileRow) => ({ file: f, suggestedDir: `${SCHOOL}\\${{ word: "Word", pdf: "PDF", excel: "Excel", powerpoint: "PowerPoint", images: "Images", videos: "Videos", other: "Other" }[f.kind]}\\2026-10` });
    return {
      duplicates: groups,
      desktop: files.filter((f) => f.parent === DESKTOP).map(suggest),
      oldDownloads: files.filter((f) => f.parent === DOWNLOADS && f.ext !== "exe" && f.modified < NOW - 60 * DAY).map(suggest),
    };
  },
  list_templates: () => templates,
  add_template: (a) => {
    const t: Template = { id: templates.length + 1, path: String(a.path), name: String(a.path).split("\\").pop()?.replace(/\.[^.]+$/, "") ?? "", missing: false };
    templates = [...templates, t];
    return t;
  },
  remove_template: (a) => {
    templates = templates.filter((t) => t.id !== a.id);
  },
  use_template: (a) => {
    const t = templates.find((x) => x.id === a.id);
    if (!t) throw { code: "notFound", message: "That template is no longer on the shelf." };
    const copy = file(`${SCHOOL}\\Word\\2026-10`, `${t.name} 2026-10-12.docx`, { modified: NOW });
    files.unshift(copy);
    ops.unshift({ id: nextOp++, kind: "copy", src: t.path, dst: copy.path, status: "done", message: null, automatic: false, createdAt: NOW });
    return copy.path;
  },
  get_settings: () => settings,
  set_settings: (a) => {
    const next = a.settings as Settings;
    settings = { ...next, extraFolders: settings.extraFolders, excludedFolders: settings.excludedFolders };
    return settings;
  },
  add_folder: (a) => {
    const path = String(a.path);
    settings = { ...settings, extraFolders: [...settings.extraFolders, path], excludedFolders: settings.excludedFolders.filter((p) => p !== path) };
    return settings;
  },
  remove_folder: (a) => {
    const path = String(a.path);
    if (path === DOCS || path === DOWNLOADS) throw { code: "validation", message: "Documents and Downloads are where filing happens, so they stay." };
    const wasExtra = settings.extraFolders.includes(path);
    settings = { ...settings, extraFolders: settings.extraFolders.filter((p) => p !== path), excludedFolders: wasExtra ? settings.excludedFolders : [...settings.excludedFolders, path] };
    return settings;
  },
  restart_app: () => undefined,
  browser_check: () => [
    { browser: "Chrome", profile: "Teacher", ok: true, asks: false, folder: null, advice: "Saves to Downloads. Nothing to change." },
    {
      browser: "Edge",
      profile: "Default",
      ok: false,
      asks: true,
      folder: null,
      advice: "Asks where to save each file. In Edge, open Settings > Downloads and turn that off, with the location set to Downloads.",
    },
  ],
  pick_folder: () => `${DOCS}\\Lesson Plans`,
  pick_file: () => `${DOCS}\\Forms\\DLL Template.docx`,
  index_status: () => ({ scanning: false, scanned: files.length, total: files.length, files: files.length }),
};

export function installMockBackend(): void {
  const mock: MockBackend & { calls: typeof calls; simulateDownload(): void } = {
    calls,
    async invoke(command, args) {
      calls.push({ command, args });
      const handler = handlers[command];
      if (!handler) throw { code: "unsupported", message: `mock: no handler for ${command}` };
      await new Promise((done) => setTimeout(done, 15));
      return handler(args);
    },
    listen(event, handler) {
      if (!listeners.has(event)) listeners.set(event, new Set());
      listeners.get(event)?.add(handler);
      return () => listeners.get(event)?.delete(handler);
    },
    simulateDownload() {
      const row = file(`${SCHOOL}\\PDF\\2026-10`, "Reading Assessment Q2.pdf", { originLabel: "Messenger", downloadedAt: NOW, modified: NOW });
      files.unshift(row);
      ops.unshift({ id: nextOp++, kind: "autoSort", src: `${DOWNLOADS}\\${row.name}`, dst: row.path, status: "done", message: null, automatic: true, createdAt: NOW });
      emit("desk://sorted", { name: row.name, folder: "School Files > PDF", path: row.path });
      emit("desk://changed");
    },
  };
  window.__deskMock = mock;
}
