import { describe, expect, it } from "vitest";
import { availableChips, currentQuarter, parseDay, spanFor } from "../lib/dates";
import type { Settings } from "../lib/types";

const base: Settings = { startAtLogin: true, notifications: true, tourDone: true, schoolYear: null, quarters: [] };
const now = new Date(2026, 9, 14, 10, 0, 0); // Wednesday 14 October 2026
const unix = (d: Date) => Math.floor(d.getTime() / 1000);

describe("date chips", () => {
  it("computes today, this week (Monday) and this month", () => {
    expect(spanFor("today", base, now).from).toBe(unix(new Date(2026, 9, 14)));
    expect(spanFor("week", base, now).from).toBe(unix(new Date(2026, 9, 12)));
    expect(spanFor("month", base, now).from).toBe(unix(new Date(2026, 9, 1)));
    expect(spanFor("all", base, now)).toEqual({});
  });

  it("offers quarter and school year only when their dates are set", () => {
    expect(availableChips(base, now).map((c) => c.id)).toEqual(["all", "today", "week", "month"]);
    const settings: Settings = {
      ...base,
      schoolYear: { name: "School year", start: "2026-07-28", end: "2027-04-16" },
      quarters: [
        { name: "Quarter 1", start: "2026-07-28", end: "2026-10-02" },
        { name: "Quarter 2", start: "2026-10-05", end: "2026-12-11" },
      ],
    };
    expect(availableChips(settings, now).map((c) => c.label)).toEqual(["Any time", "Today", "This week", "This month", "Quarter 2", "School year"]);
    expect(currentQuarter(settings, now)?.name).toBe("Quarter 2");
    const quarter = spanFor("quarter", settings, now);
    expect(quarter.from).toBe(unix(new Date(2026, 9, 5)));
    expect(quarter.to).toBe(unix(new Date(2026, 11, 12)));
    expect(spanFor("year", settings, now).from).toBe(unix(new Date(2026, 6, 28)));
  });

  it("ignores half-typed dates", () => {
    expect(parseDay("2026-1")).toBeNull();
    const settings: Settings = { ...base, schoolYear: { name: "School year", start: "2026-07-28", end: "" } };
    expect(availableChips(settings, now).some((c) => c.id === "year")).toBe(false);
  });
});
