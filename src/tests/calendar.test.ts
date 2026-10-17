import { describe, expect, it } from "vitest";
import { addDays, addMonths, allowed, canTurn, clampDay, dayLabel, fieldWords, keyMove, monthTitle, monthWeeks, today } from "../lib/calendar";

describe("calendar", () => {
  it("lays a month out in weeks that start on Sunday, other months left empty", () => {
    const october = monthWeeks(2026, 9);
    expect(october).toHaveLength(5);
    // 1 October 2026 is a Thursday.
    expect(october[0]).toEqual([null, null, null, null, "2026-10-01", "2026-10-02", "2026-10-03"]);
    expect(october[4]).toEqual(["2026-10-25", "2026-10-26", "2026-10-27", "2026-10-28", "2026-10-29", "2026-10-30", "2026-10-31"]);
    // February 2026 starts on a Sunday and fills four weeks exactly.
    expect(monthWeeks(2026, 1)).toHaveLength(4);
    expect(monthWeeks(2026, 1)[0][0]).toBe("2026-02-01");
    expect(monthTitle(2026, 9)).toBe("October 2026");
  });

  it("words a day plainly on the field and in full for a screen reader", () => {
    expect(fieldWords("2026-10-17")).toBe("Sat 17 Oct 2026");
    expect(fieldWords("2026-10-17", true)).toBe("17 Oct 2026");
    expect(dayLabel("2026-10-17")).toBe("Saturday 17 October 2026");
    expect(fieldWords("")).toBe("");
    expect(today(new Date(2026, 0, 5))).toBe("2026-01-05");
  });

  it("moves by days and months across month and year ends", () => {
    expect(addDays("2026-12-31", 1)).toBe("2027-01-01");
    expect(addDays("2026-03-01", -1)).toBe("2026-02-28");
    expect(addMonths("2026-01-31", 1)).toBe("2026-02-28");
    expect(addMonths("2028-01-31", 1)).toBe("2028-02-29");
    expect(addMonths("2026-01-15", -1)).toBe("2025-12-15");
  });

  it("moves the focused day the way the keys say", () => {
    expect(keyMove("2026-10-17", "ArrowLeft")).toBe("2026-10-16");
    expect(keyMove("2026-10-17", "ArrowRight")).toBe("2026-10-18");
    expect(keyMove("2026-10-17", "ArrowUp")).toBe("2026-10-10");
    expect(keyMove("2026-10-17", "ArrowDown")).toBe("2026-10-24");
    expect(keyMove("2026-10-14", "Home")).toBe("2026-10-11");
    expect(keyMove("2026-10-14", "End")).toBe("2026-10-17");
    expect(keyMove("2026-10-31", "PageDown")).toBe("2026-11-30");
    expect(keyMove("2026-10-17", "PageUp")).toBe("2026-09-17");
    expect(keyMove("2026-10-17", "a")).toBeNull();
  });

  it("keeps days inside the allowed range and knows when a month has nothing to pick", () => {
    expect(allowed("2026-06-01", "2026-06-01", "2027-03-31")).toBe(true);
    expect(allowed("2026-05-31", "2026-06-01")).toBe(false);
    expect(allowed("2027-04-01", undefined, "2027-03-31")).toBe(false);
    expect(clampDay("2026-01-01", "2026-06-01", "2027-03-31")).toBe("2026-06-01");
    expect(clampDay("2028-01-01", "2026-06-01", "2027-03-31")).toBe("2027-03-31");
    expect(canTurn("2026-06-15", -1, "2026-06-01")).toBe(false);
    expect(canTurn("2026-07-15", -1, "2026-06-01")).toBe(true);
    expect(canTurn("2027-03-02", 1, undefined, "2027-03-31")).toBe(false);
    expect(canTurn("2027-02-02", 1, undefined, "2027-03-31")).toBe(true);
    expect(canTurn("2027-02-02", 1)).toBe(true);
  });
});
