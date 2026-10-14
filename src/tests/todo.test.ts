import { describe, expect, it } from "vitest";
import { dayWords, dueWords, filesWords, groupOf, groupTasks, hasQuarters, repeatWords, taskMeta } from "../lib/todo";
import type { Settings, Task } from "../lib/types";

const wednesday = new Date(2026, 9, 14, 10, 0, 0); // Wednesday 14 October 2026
const saturday = new Date(2026, 9, 17, 9, 0, 0);
const sunday = new Date(2026, 9, 18, 9, 0, 0);

function task(id: number, due: string | null, extra: Partial<Task> = {}): Task {
  return { id, title: `Task ${id}`, due, repeat: "never", doneAt: null, createdAt: id, files: [], ...extra };
}

const settings: Settings = { startAtLogin: true, notifications: true, tourDone: true, schoolYear: null, quarters: [], extraFolders: [], excludedFolders: [], subjectRules: [], checkUpdates: true, todoSeen: true };

describe("To do groups", () => {
  it("puts each due day in its group", () => {
    expect(groupOf("2026-10-13", wednesday)).toBe("Overdue");
    expect(groupOf("2026-10-14", wednesday)).toBe("Today");
    expect(groupOf("2026-10-15", wednesday)).toBe("Tomorrow");
    expect(groupOf("2026-10-18", wednesday)).toBe("This week");
    expect(groupOf("2026-10-19", wednesday)).toBe("Later");
    expect(groupOf(null, wednesday)).toBe("No date");
    expect(groupOf("not a day", wednesday)).toBe("No date");
  });

  it("has no This week on the weekend edges", () => {
    expect(groupOf("2026-10-18", saturday)).toBe("Tomorrow");
    expect(groupOf("2026-10-19", saturday)).toBe("Later");
    expect(groupOf("2026-10-19", sunday)).toBe("Tomorrow");
    expect(groupOf("2026-10-20", sunday)).toBe("Later");
  });

  it("orders groups and tasks, leaving done tasks out", () => {
    const groups = groupTasks([task(1, null), task(2, "2026-10-20"), task(3, "2026-10-14"), task(4, "2026-10-10"), task(5, "2026-10-14", { doneAt: 1 }), task(6, "2026-10-14")], wednesday);
    expect(groups.map((g) => g.title)).toEqual(["Overdue", "Today", "Later", "No date"]);
    expect(groups[1].items.map((t) => t.id)).toEqual([3, 6]);
  });
});

describe("To do words", () => {
  it("says when a task is due", () => {
    expect(dueWords("2026-10-14", wednesday)).toBe("due today");
    expect(dueWords("2026-10-15", wednesday)).toBe("due tomorrow");
    expect(dueWords("2026-10-16", wednesday)).toBe("due Friday");
    expect(dueWords("2026-10-30", wednesday)).toBe("due 30 October");
    expect(dueWords("2027-01-05", wednesday)).toBe("due 5 January 2027");
    expect(dueWords("2026-10-13", wednesday)).toBe("was due yesterday");
    expect(dueWords("2026-10-09", wednesday)).toBe("was due Friday");
    expect(dueWords("2026-09-30", wednesday)).toBe("was due 30 September");
    expect(dueWords(null, wednesday)).toBeNull();
    expect(dayWords("2026-11-14", wednesday)).toBe("14 November");
  });

  it("names repeats and files, and joins them into one quiet line", () => {
    expect(repeatWords("never")).toBeNull();
    expect(repeatWords("quarter")).toBe("every quarter");
    expect(filesWords(0)).toBeNull();
    expect(filesWords(1)).toBe("1 file");
    expect(filesWords(3)).toBe("3 files");
    const file = { path: "C:\\a.docx", name: "a.docx", ext: "docx", kind: "word" as const, missing: false };
    expect(taskMeta(task(1, "2026-10-14", { repeat: "month", files: [file, file] }), wednesday)).toBe("due today \u00b7 every month \u00b7 2 files");
    expect(taskMeta(task(1, "2026-10-14", { doneAt: 5 }), wednesday)).toBe("");
  });

  it("offers the quarter repeat only with usable quarter dates", () => {
    expect(hasQuarters(settings)).toBe(false);
    expect(hasQuarters({ ...settings, quarters: [{ name: "Quarter 1", start: "2026-06-08", end: "" }] })).toBe(false);
    expect(hasQuarters({ ...settings, quarters: [{ name: "Quarter 1", start: "2026-06-08", end: "2026-08-15" }] })).toBe(true);
  });
});
