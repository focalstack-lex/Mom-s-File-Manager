// The date picker's logic (components/DatePicker.tsx): a day is "YYYY-MM-DD"
// text in local time, the same value the native date field gave, so the
// stores and the Rust side see no change. Weeks start on Sunday, as the
// calendars on a Philippine school wall and in Windows (en-PH) do.

import { parseDay } from "./dates";

export const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MONTHS_SHORT = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
export const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
/** Two letters, so Tuesday and Thursday, Saturday and Sunday never look alike. */
export const WEEKDAYS_SHORT = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
const WEEKDAYS_FIELD = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

const pad = (n: number) => String(n).padStart(2, "0");

/** "YYYY-MM-DD" for a local date. */
export function dayText(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** Today as "YYYY-MM-DD". */
export function today(now: Date = new Date()): string {
  return dayText(now);
}

function dateOf(day: string): Date {
  return parseDay(day) ?? new Date();
}

export function addDays(day: string, count: number): string {
  const date = dateOf(day);
  return dayText(new Date(date.getFullYear(), date.getMonth(), date.getDate() + count));
}

/** The same day `count` months on, or the month's last day when it is shorter (31 Jan + 1 is 28 or 29 Feb). */
export function addMonths(day: string, count: number): string {
  const date = dateOf(day);
  const first = new Date(date.getFullYear(), date.getMonth() + count, 1);
  const last = new Date(first.getFullYear(), first.getMonth() + 1, 0).getDate();
  return dayText(new Date(first.getFullYear(), first.getMonth(), Math.min(date.getDate(), last)));
}

/** The month of `day` as weeks of seven, Sunday first; days of other months are null. */
export function monthWeeks(year: number, month: number): (string | null)[][] {
  const lead = new Date(year, month, 1).getDay();
  const length = new Date(year, month + 1, 0).getDate();
  const cells: (string | null)[] = Array.from({ length: lead }, () => null);
  for (let d = 1; d <= length; d += 1) cells.push(dayText(new Date(year, month, d)));
  while (cells.length % 7 !== 0) cells.push(null);
  const weeks: (string | null)[][] = [];
  for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7));
  return weeks;
}

/** What the field shows: "Fri 17 Oct 2026", or "17 Oct 2026" where a column is narrow. */
export function fieldWords(day: string, compact = false): string {
  const date = parseDay(day);
  if (!date) return "";
  const words = `${date.getDate()} ${MONTHS_SHORT[date.getMonth()]} ${date.getFullYear()}`;
  return compact ? words : `${WEEKDAYS_FIELD[date.getDay()]} ${words}`;
}

/** What a screen reader says for a day: "Friday 17 October 2026". */
export function dayLabel(day: string): string {
  const date = parseDay(day);
  if (!date) return "";
  return `${WEEKDAYS[date.getDay()]} ${date.getDate()} ${MONTHS[date.getMonth()]} ${date.getFullYear()}`;
}

/** "October 2026". */
export function monthTitle(year: number, month: number): string {
  return `${MONTHS[month]} ${year}`;
}

/** Whether `day` may be picked: inside min and max when they are set ("YYYY-MM-DD" text sorts by date). */
export function allowed(day: string, min?: string, max?: string): boolean {
  return (!min || day >= min) && (!max || day <= max);
}

/** `day` moved inside min and max. */
export function clampDay(day: string, min?: string, max?: string): string {
  if (min && day < min) return min;
  if (max && day > max) return max;
  return day;
}

/** Whether any day of the month before (or after) the one `day` is in may be picked. */
export function canTurn(day: string, step: -1 | 1, min?: string, max?: string): boolean {
  const date = dateOf(day);
  if (step < 0) return !min || dayText(new Date(date.getFullYear(), date.getMonth(), 0)) >= min;
  return !max || dayText(new Date(date.getFullYear(), date.getMonth() + 1, 1)) <= max;
}

/**
 * Where a key moves the focused day in the grid, or null for a key the grid
 * leaves alone: arrows by a day or a week, Home and End to the week's
 * Sunday and Saturday, Page Up and Page Down by a month.
 */
export function keyMove(day: string, key: string): string | null {
  const weekday = dateOf(day).getDay();
  switch (key) {
    case "ArrowLeft":
      return addDays(day, -1);
    case "ArrowRight":
      return addDays(day, 1);
    case "ArrowUp":
      return addDays(day, -7);
    case "ArrowDown":
      return addDays(day, 7);
    case "Home":
      return addDays(day, -weekday);
    case "End":
      return addDays(day, 6 - weekday);
    case "PageUp":
      return addMonths(day, -1);
    case "PageDown":
      return addMonths(day, 1);
    default:
      return null;
  }
}
