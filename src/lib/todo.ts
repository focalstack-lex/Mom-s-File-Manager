// The To do list in plain words (spec 2026-10-14): which group a task sits
// in, when it is due, how it repeats, and whether "Every quarter" can be
// offered. Pure functions; "now" is passed in so tests can fix the day.

import { parseDay } from "./dates";
import type { DayGroup } from "./paging";
import type { Repeat, Settings, Task } from "./types";

export const TASK_GROUPS = ["Overdue", "Today", "Tomorrow", "This week", "Later", "No date"] as const;
export type TaskGroup = (typeof TASK_GROUPS)[number];

const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

/** Whole days from today to `day` (rounded, so a daylight saving change does not shift it). */
function daysFrom(today: Date, day: Date): number {
  return Math.round((day.getTime() - today.getTime()) / 86_400_000);
}

export function groupOf(due: string | null, now: Date = new Date()): TaskGroup {
  const day = due ? parseDay(due) : null;
  if (!day) return "No date";
  const today = startOfDay(now);
  const diff = daysFrom(today, day);
  if (diff < 0) return "Overdue";
  if (diff === 0) return "Today";
  if (diff === 1) return "Tomorrow";
  // The rest of the Monday to Sunday week.
  const toSunday = (7 - today.getDay()) % 7;
  return diff <= toSunday ? "This week" : "Later";
}

function byDue(a: Task, b: Task): number {
  if (a.due !== b.due) {
    if (a.due === null) return 1;
    if (b.due === null) return -1;
    return a.due < b.due ? -1 : 1;
  }
  return a.createdAt - b.createdAt || a.id - b.id;
}

/** Open tasks in their groups, in group order, empty groups left out. */
export function groupTasks(tasks: Task[], now: Date = new Date()): DayGroup<Task>[] {
  const open = tasks.filter((t) => t.doneAt === null).sort(byDue);
  return TASK_GROUPS.map((title) => ({ title, items: open.filter((t) => groupOf(t.due, now) === title) })).filter((g) => g.items.length > 0);
}

function dayMonth(day: Date, now: Date): string {
  const text = `${day.getDate()} ${MONTHS[day.getMonth()]}`;
  return day.getFullYear() === now.getFullYear() ? text : `${text} ${day.getFullYear()}`;
}

/** "due today", "due Friday", "due 30 October", "was due yesterday". */
export function dueWords(due: string | null, now: Date = new Date()): string | null {
  const day = due ? parseDay(due) : null;
  if (!day) return null;
  const diff = daysFrom(startOfDay(now), day);
  if (diff === 0) return "due today";
  if (diff === 1) return "due tomorrow";
  if (diff === -1) return "was due yesterday";
  if (diff > 1 && diff < 7) return `due ${DAYS[day.getDay()]}`;
  if (diff < -1 && diff > -7) return `was due ${DAYS[day.getDay()]}`;
  return `${diff < 0 ? "was due" : "due"} ${dayMonth(day, now)}`;
}

/** "14 November", for the toast after a repeating task is ticked. */
export function dayWords(due: string, now: Date = new Date()): string {
  const day = parseDay(due);
  return day ? dayMonth(day, now) : due;
}

export function repeatWords(repeat: Repeat): string | null {
  switch (repeat) {
    case "week":
      return "every week";
    case "month":
      return "every month";
    case "quarter":
      return "every quarter";
    default:
      return null;
  }
}

export function filesWords(count: number): string | null {
  if (count === 0) return null;
  return count === 1 ? "1 file" : `${count} files`;
}

/** The one quiet line under a task's title. Done tasks keep only the repeat and files. */
export function taskMeta(task: Task, now: Date = new Date()): string {
  return [task.doneAt === null ? dueWords(task.due, now) : null, repeatWords(task.repeat), filesWords(task.files.length)].filter(Boolean).join(" · ");
}

/** "Every quarter" needs at least one quarter with both dates in Settings. */
export function hasQuarters(settings: Settings): boolean {
  return settings.quarters.some((q) => {
    const start = parseDay(q.start);
    const end = parseDay(q.end);
    return Boolean(start && end && start <= end);
  });
}
