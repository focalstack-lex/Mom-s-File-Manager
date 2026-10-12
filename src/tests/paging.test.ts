import { describe, expect, it } from "vitest";
import { activityFirstPage, dayTitle, groupByDay, moreLabel, takeRows } from "../lib/paging";

const now = new Date(2026, 9, 14, 15, 0, 0); // Wednesday 14 October 2026
const at = (y: number, m: number, d: number, h = 9) => Math.floor(new Date(y, m, d, h).getTime() / 1000);

describe("day groups", () => {
  it("names days the way she remembers them", () => {
    expect(dayTitle(at(2026, 9, 14), now)).toBe("Today");
    expect(dayTitle(at(2026, 9, 13), now)).toBe("Yesterday");
    expect(dayTitle(at(2026, 9, 10), now)).toBe("Earlier this week");
    expect(dayTitle(at(2026, 8, 1), now)).toBe("Earlier");
  });

  it("groups a newest-first list and trims by rows, keeping headings", () => {
    const items = [at(2026, 9, 14, 12), at(2026, 9, 14, 8), at(2026, 9, 13), at(2026, 9, 11), at(2026, 9, 11), at(2026, 8, 2)];
    const groups = groupByDay(items, (t) => t, now);
    expect(groups.map((g) => [g.title, g.items.length])).toEqual([
      ["Today", 2],
      ["Yesterday", 1],
      ["Earlier this week", 2],
      ["Earlier", 1],
    ]);
    const first = takeRows(groups, 4);
    expect(first.map((g) => [g.title, g.items.length])).toEqual([
      ["Today", 2],
      ["Yesterday", 1],
      ["Earlier this week", 1],
    ]);
    expect(takeRows(groups, 0)).toEqual([]);
    expect(takeRows(groups, 99).reduce((n, g) => n + g.items.length, 0)).toBe(6);
  });
});

describe("pages", () => {
  it("labels the next page and hides when everything shows", () => {
    expect(moreLabel(10, 84, 10)).toBe("Show 10 more");
    expect(moreLabel(80, 84, 10)).toBe("Show 4 more");
    expect(moreLabel(84, 84, 10)).toBeNull();
    expect(moreLabel(10, 3, 10)).toBeNull();
  });

  it("opens Activity on Today and Yesterday, within 10 to 30 rows", () => {
    const g = (title: string, n: number) => ({ title, items: Array.from({ length: n }, (_, i) => i) });
    expect(activityFirstPage([g("Today", 2), g("Earlier", 50)])).toBe(10);
    expect(activityFirstPage([g("Today", 8), g("Yesterday", 9), g("Earlier", 50)])).toBe(17);
    expect(activityFirstPage([g("Today", 120)])).toBe(30);
  });
});
