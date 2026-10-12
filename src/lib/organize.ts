// Words for Organize now (spec 5.11): folder labels, the plan summary, and
// the comma-separated word lists in Settings.

import { monthName } from "./format";
import type { OpRow, OrganizePlan } from "./types";

/** "C:\...\School Files\Grade 7\Science" becomes "Grade 7 > Science". */
export function folderLabel(dir: string, schoolFiles: string): string {
  const rest = dir.toLowerCase().startsWith(schoolFiles.toLowerCase()) ? dir.slice(schoolFiles.length) : dir;
  const parts = rest.split("\\").filter(Boolean).map(monthName);
  return parts.length === 0 ? "School Files" : parts.join(" > ");
}

/** "84 files into 9 folders", "1 file into 1 folder". */
export function planSummary(files: number, folders: number): string {
  const f = files === 1 ? "1 file" : `${files.toLocaleString()} files`;
  const d = folders === 1 ? "1 folder" : `${folders.toLocaleString()} folders`;
  return `${f} into ${d}`;
}

/** How many of the plan's folders do not exist yet. */
export function newFolderCount(plan: OrganizePlan): number {
  return plan.folders.filter((f) => !f.exists).length;
}

/** "math, mathematics" becomes ["math", "mathematics"]; blanks and repeats are dropped. */
export function parseWords(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of text.split(",")) {
    const word = raw.trim().toLowerCase();
    if (word && !seen.has(word)) {
      seen.add(word);
      out.push(word);
    }
  }
  return out;
}

export function wordsText(words: string[]): string {
  return words.join(", ");
}

/** An Activity entry: one change, or one whole Organize run shown as a single line. */
export type ActivityEntry = { type: "op"; op: OpRow; createdAt: number } | { type: "batch"; batch: string; ops: OpRow[]; createdAt: number };

/** Folds the moves of each Organize run (newest first, consecutive) into one entry. */
export function foldBatches(ops: OpRow[]): ActivityEntry[] {
  const out: ActivityEntry[] = [];
  for (const op of ops) {
    const last = out[out.length - 1];
    if (op.batch && last?.type === "batch" && last.batch === op.batch) last.ops.push(op);
    else if (op.batch) out.push({ type: "batch", batch: op.batch, ops: [op], createdAt: op.createdAt });
    else out.push({ type: "op", op, createdAt: op.createdAt });
  }
  return out;
}

/** "Organized 84 files into 9 folders" for a folded run. */
export function batchTitle(ops: OpRow[]): string {
  const folders = new Set(ops.map((o) => (o.dst ?? "").slice(0, (o.dst ?? "").lastIndexOf("\\"))));
  return `Organized ${planSummary(ops.length, folders.size)}`;
}
