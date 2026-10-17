// Plain-words formatting: times she can read at a glance, locations as
// "School Files > Word > October", sizes without decimals she does not need.

import type { FileRow, Kind } from "./types";

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

/** "today 2:14 PM", "yesterday", "Tuesday", "12 October", "12 October 2025". */
export function relativeTime(unixSeconds: number, now: Date = new Date()): string {
  const date = new Date(unixSeconds * 1000);
  const dayDiff = Math.round((startOfDay(now).getTime() - startOfDay(date).getTime()) / 86_400_000);
  if (dayDiff === 0) return `today ${clock(date)}`;
  if (dayDiff === 1) return "yesterday";
  if (dayDiff > 1 && dayDiff < 7) return DAYS[date.getDay()];
  if (date.getFullYear() === now.getFullYear()) return `${date.getDate()} ${MONTHS[date.getMonth()]}`;
  return `${date.getDate()} ${MONTHS[date.getMonth()]} ${date.getFullYear()}`;
}

function clock(date: Date): string {
  const hours = date.getHours();
  const minutes = String(date.getMinutes()).padStart(2, "0");
  const suffix = hours >= 12 ? "PM" : "AM";
  const twelve = hours % 12 === 0 ? 12 : hours % 12;
  return `${twelve}:${minutes} ${suffix}`;
}

/** "from Messenger, Tuesday" or "edited yesterday" for files that were not downloaded. */
export function originLine(file: Pick<FileRow, "originLabel" | "downloadedAt" | "modified">, now?: Date): string {
  if (file.downloadedAt) {
    const source = file.originLabel && file.originLabel !== "Unknown source" ? `from ${file.originLabel}` : "downloaded";
    return `${source}, ${relativeTime(file.downloadedAt, now)}`;
  }
  return `edited ${relativeTime(file.modified, now)}`;
}

/** Folder path relative to the roots, in words: "in School Files > Word > October". */
export function locationLine(parent: string, roots: string[]): string {
  let rest = parent;
  let rootName = "";
  for (const root of roots) {
    if (parent === root || parent.startsWith(`${root}\\`)) {
      rootName = root.split("\\").pop() ?? root;
      rest = parent.slice(root.length);
      break;
    }
  }
  const parts = rest.split("\\").filter(Boolean).map(monthName);
  const crumbs = rootName && !(rootName === "Documents" && parts[0] === "School Files") ? [rootName, ...parts] : parts;
  if (crumbs.length === 0) return "in this computer";
  return `in ${crumbs.join(" > ")}`;
}

/** 2026-10 becomes "October 2026" so month folders read as months. */
export function monthName(segment: string): string {
  const match = /^(\d{4})-(\d{2})$/.exec(segment);
  if (!match) return segment;
  const month = Number(match[2]);
  if (month < 1 || month > 12) return segment;
  return `${MONTHS[month - 1]} ${match[1]}`;
}

export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

export const KIND_LABEL: Record<Kind, string> = {
  word: "Word",
  pdf: "PDF",
  excel: "Excel",
  powerpoint: "PowerPoint",
  images: "Image",
  videos: "Video",
  other: "File",
};

/**
 * Programs, scripts and shortcuts, the kinds sorter.rs keeps in Downloads:
 * opening one runs something, so the row offers "Open the program" and the
 * tile says "program" instead of a plain Open (audit T-41).
 */
const PROGRAM_EXTS = new Set(["exe", "msi", "bat", "cmd", "ps1", "vbs", "js", "jse", "wsf", "hta", "lnk", "url", "scr", "com", "reg"]);

export function isProgram(ext: string | undefined): boolean {
  return PROGRAM_EXTS.has((ext ?? "").replace(/^\./, "").toLowerCase());
}

/** The file name without its extension, for inline rename. */
export function stemOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

export function extOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot) : "";
}

/** "3 other copies" or "1 other copy". */
export function copiesLabel(count: number): string {
  return count === 1 ? "1 other copy" : `${count} other copies`;
}
