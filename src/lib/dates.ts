// Date chips (spec 5.3): Today, This week, This month, plus This quarter and
// School year once their dates are set in Settings.

import type { DateRange, Settings } from "./types";

export type DateChip = "all" | "today" | "week" | "month" | "quarter" | "year";

export interface Span {
  from?: number;
  to?: number;
}

function unix(date: Date): number {
  return Math.floor(date.getTime() / 1000);
}

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

/** Parses "YYYY-MM-DD" as a local date, or null. */
export function parseDay(text: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text.trim());
  if (!match) return null;
  const date = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  return Number.isNaN(date.getTime()) ? null : date;
}

function rangeSpan(range: DateRange): Span | null {
  const start = parseDay(range.start);
  const end = parseDay(range.end);
  if (!start || !end) return null;
  const endExclusive = new Date(end.getFullYear(), end.getMonth(), end.getDate() + 1);
  return { from: unix(start), to: unix(endExclusive) };
}

/** The quarter whose range contains `now`, or the most recent one. */
export function currentQuarter(settings: Settings, now: Date = new Date()): DateRange | null {
  const today = startOfDay(now).getTime();
  let latest: DateRange | null = null;
  for (const quarter of settings.quarters) {
    const start = parseDay(quarter.start);
    const end = parseDay(quarter.end);
    if (!start || !end) continue;
    if (today >= start.getTime() && today <= end.getTime()) return quarter;
    if (start.getTime() <= today && (!latest || start.getTime() > (parseDay(latest.start)?.getTime() ?? 0))) latest = quarter;
  }
  return latest;
}

export function spanFor(chip: DateChip, settings: Settings, now: Date = new Date()): Span {
  const today = startOfDay(now);
  switch (chip) {
    case "all":
      return {};
    case "today":
      return { from: unix(today) };
    case "week": {
      const monday = new Date(today);
      monday.setDate(today.getDate() - ((today.getDay() + 6) % 7));
      return { from: unix(monday) };
    }
    case "month":
      return { from: unix(new Date(today.getFullYear(), today.getMonth(), 1)) };
    case "quarter": {
      const quarter = currentQuarter(settings, now);
      return quarter ? (rangeSpan(quarter) ?? {}) : {};
    }
    case "year":
      return settings.schoolYear ? (rangeSpan(settings.schoolYear) ?? {}) : {};
  }
}

/** Chips to show: quarter and school year only when their dates are usable. */
export function availableChips(settings: Settings, now: Date = new Date()): { id: DateChip; label: string }[] {
  const chips: { id: DateChip; label: string }[] = [
    { id: "all", label: "Any time" },
    { id: "today", label: "Today" },
    { id: "week", label: "This week" },
    { id: "month", label: "This month" },
  ];
  const quarter = currentQuarter(settings, now);
  if (quarter) chips.push({ id: "quarter", label: quarter.name || "This quarter" });
  if (settings.schoolYear && rangeSpan(settings.schoolYear)) chips.push({ id: "year", label: "School year" });
  return chips;
}
