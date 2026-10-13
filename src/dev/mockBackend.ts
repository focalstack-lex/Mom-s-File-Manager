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
// "Two hours ago", but never before midnight, so the drive's "Today" heading
// holds whatever time it runs.
const EARLIER_TODAY = Math.max(NOW - 2 * 3600, Math.floor(new Date().setHours(0, 1, 0, 0) / 1000));
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
  file(`${SCHOOL}\\Word\\2026-10`, "Grade 7 Quiz 2.docx", { originLabel: "Messenger", downloadedAt: EARLIER_TODAY, modified: EARLIER_TODAY }),
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

// Enough files that the lists need "Show more" (spec 5.10): 9 more recent
// downloads over the last weeks, and 13 more old downloads for Clean up.
const MORE_RECENT: [string, string, number][] = [
  ["Reading Log Week 3.docx", "Messenger", 2],
  ["Seat Plan 7-Mabini.xlsx", "Gmail", 3],
  ["Science Fair Rubric.pdf", "Google Drive", 4],
  ["Parent Letter October.docx", "Messenger", 8],
  ["Quarter 1 Grades.xlsx", "Gmail", 9],
  ["Math Drill Sheet 4.pdf", "Messenger", 11],
  ["Faculty Meeting Notes.docx", "Gmail", 15],
  ["Field Trip Consent.pdf", "Messenger", 18],
  ["Spelling Bee Words.docx", "Google Drive", 21],
];
for (const [name, origin, days] of MORE_RECENT) {
  const kind = name.endsWith(".pdf") ? "PDF" : name.endsWith(".xlsx") ? "Excel" : "Word";
  files.push(file(`${SCHOOL}\\${kind}\\2026-10`, name, { originLabel: origin, downloadedAt: NOW - days * DAY, modified: NOW - days * DAY }));
}
for (let i = 1; i <= 13; i += 1) files.push(file(DOWNLOADS, `old handout ${i}.pdf`, { modified: NOW - (70 + i) * DAY }));

const ops: OpRow[] = [
  { id: 1, kind: "autoSort", src: `${DOWNLOADS}\\Grade 7 Quiz 2.docx`, dst: `${SCHOOL}\\Word\\2026-10\\Grade 7 Quiz 2.docx`, status: "done", message: null, automatic: true, createdAt: EARLIER_TODAY, batch: null },
  { id: 2, kind: "autoSort", src: `${DOWNLOADS}\\DepEd Order 2026-014.pdf`, dst: `${SCHOOL}\\PDF\\2026-10\\DepEd Order 2026-014.pdf`, status: "done", message: null, automatic: true, createdAt: NOW - DAY, batch: null },
  { id: 3, kind: "autoSort", src: `${DOWNLOADS}\\budget.xlsx`, dst: null, status: "failed", message: "The file stayed open in another program for a day, so it was left in Downloads.", automatic: true, createdAt: NOW - 2 * DAY, batch: null },
];
let nextOp = 4;
let templates: Template[] = [{ id: 1, path: `${DOCS}\\Forms\\SF9 Template.docx`, name: "SF9 Template", missing: false }];
let settings: Settings = { startAtLogin: true, notifications: true, tourDone: false, schoolYear: null, quarters: [], extraFolders: [], excludedFolders: [], subjectRules: [] };
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

function moveTo(path: string, destination: string, kind: OpRow["kind"], batch: string | null = null): string {
  const row = files.find((f) => f.path === path);
  if (!row) throw { code: "notFound", message: "That file is no longer there." };
  let name = row.name;
  let n = 2;
  while (files.some((f) => f.parent === destination && f.name === name)) {
    name = row.name.replace(/(\.[^.]+)$/, ` (${n})$1`);
    n += 1;
  }
  const dst = `${destination}\\${name}`;
  ops.unshift({ id: nextOp++, kind, src: row.path, dst, status: "done", message: null, automatic: false, createdAt: NOW, batch });
  row.path = dst;
  row.parent = destination;
  row.name = name;
  return dst;
}


// Organize now in the harness: a small version of src-tauri/src/organize.rs.
const MOCK_RULES = [
  { folder: "DepEd Forms", words: ["sf9", "sf 9", "form 137"] },
  { folder: "Math", words: ["math", "mathematics"] },
  { folder: "Science", words: ["science"] },
  { folder: "English", words: ["english", "reading"] },
];
const KIND_FOLDER: Record<string, string> = { word: "Word", pdf: "PDF", excel: "Excel", powerpoint: "PowerPoint", images: "Images", videos: "Videos", other: "Other" };
function organizePlan() {
  const words = (name: string) => ` ${name.toLowerCase().replace(/\.[^.]+$/, "").replace(/[^a-z0-9]+/g, " ").trim()} `;
  const loose = files.filter((f) => (f.parent === DESKTOP || f.parent === DOWNLOADS) && !["exe", "lnk", "url", "zip"].includes(f.ext));
  const skipped = files.filter((f) => (f.parent === DESKTOP || f.parent === DOWNLOADS) && ["exe", "lnk", "url", "zip"].includes(f.ext)).length;
  const moves = loose.map((f) => {
    const w = words(f.name);
    const grade = /\sgrade (\d{1,2})\s/.exec(w)?.[1];
    const subject = MOCK_RULES.find((r) => r.words.some((x) => w.includes(` ${x} `)))?.folder;
    const parts = [grade ? `Grade ${grade}` : null, subject ?? null].filter(Boolean) as string[];
    const destDir = parts.length ? `${SCHOOL}\\${parts.join("\\")}` : `${SCHOOL}\\${KIND_FOLDER[f.kind]}\\2026-10`;
    const reason = parts.length ? `${parts.join(", ")}, from the name` : "no grade or subject in the name, so by type and month";
    return { path: f.path, name: f.name, ext: f.ext, kind: f.kind, modified: f.modified, destDir, reason };
  });
  const counts = new Map<string, number>();
  for (const m of moves) counts.set(m.destDir, (counts.get(m.destDir) ?? 0) + 1);
  const folders = [...counts.entries()].map(([dir, count]) => ({ dir, count, exists: files.some((f) => f.parent === dir) })).sort((a, b) => b.count - a.count);
  return { moves, folders, skipped };
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
  // Several files moved or recycled together share a batch, as in ops.rs.
  move_files: (a) => {
    const paths = a.paths as string[];
    const batch = paths.length > 1 ? `move-${nextOp}` : null;
    return paths.map((p) => moveTo(p, String(a.destination), "move", batch));
  },
  rename_file: (a) => {
    const row = files.find((f) => f.path === a.path);
    if (!row) throw { code: "notFound", message: "That file is no longer there." };
    const stem = String(a.newStem);
    if (/[\\/:*?"<>|]/.test(stem)) throw { code: "invalidName", message: "Names cannot contain : or similar characters." };
    const name = `${stem}.${row.ext}`;
    if (files.some((f) => f.parent === row.parent && f.name === name && f !== row)) throw { code: "alreadyExists", message: "A file with that name already exists in this folder." };
    ops.unshift({ id: nextOp++, kind: "rename", src: row.path, dst: `${row.parent}\\${name}`, status: "done", message: null, automatic: false, createdAt: NOW, batch: null });
    row.name = name;
    row.path = `${row.parent}\\${name}`;
    return row.path;
  },
  create_folder: (a) => `${a.parent}\\${a.name}`,
  recycle_files: (a) => {
    const out: OpRow[] = [];
    const paths = a.paths as string[];
    const batch = paths.length > 1 ? `recycle-${nextOp}` : null;
    for (const path of paths) {
      files = files.filter((f) => f.path !== path);
      const op: OpRow = { id: nextOp++, kind: "recycle", src: path, dst: null, status: "done", message: null, automatic: false, createdAt: NOW, batch };
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
    ops.unshift({ id: nextOp++, kind: "copy", src: t.path, dst: copy.path, status: "done", message: null, automatic: false, createdAt: NOW, batch: null });
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
  default_subject_rules: () => MOCK_RULES,
  organize_plan: () => organizePlan(),
  organize_run: (a) => {
    const wanted = new Set(a.paths as string[]);
    const batch = `organize-${nextOp}`;
    const folders = new Set<string>();
    let moved = 0;
    for (const m of organizePlan().moves.filter((m) => wanted.has(m.path))) {
      const row = files.find((f) => f.path === m.path);
      if (!row) continue;
      const dst = `${m.destDir}\\${row.name}`;
      ops.unshift({ id: nextOp++, kind: "move", src: row.path, dst, status: "done", message: null, automatic: false, createdAt: NOW, batch });
      row.path = dst;
      row.parent = m.destDir;
      folders.add(m.destDir);
      moved += 1;
    }
    return { batch, moved, folders: folders.size, failed: [] };
  },
  undo_batch: (a) => {
    let undone = 0;
    for (const op of ops.filter((o) => o.batch === a.batch && o.status === "done")) {
      if (op.kind === "recycle") files.push(file(op.src.slice(0, op.src.lastIndexOf("\\")), op.src.slice(op.src.lastIndexOf("\\") + 1)));
      const row = files.find((f) => f.path === op.dst);
      if (row) {
        row.path = op.src;
        row.parent = op.src.slice(0, op.src.lastIndexOf("\\"));
      }
      op.status = "undone";
      undone += 1;
    }
    return { undone, failed: [] };
  },
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
      ops.unshift({ id: nextOp++, kind: "autoSort", src: `${DOWNLOADS}\\${row.name}`, dst: row.path, status: "done", message: null, automatic: true, createdAt: NOW, batch: null });
      emit("desk://sorted", { name: row.name, folder: "School Files > PDF", path: row.path });
      emit("desk://changed");
    },
  };
  window.__deskMock = mock;
}
