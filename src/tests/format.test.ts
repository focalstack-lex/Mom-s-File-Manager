import { describe, expect, it } from "vitest";
import { copiesLabel, extOf, fileSize, isProgram, labelWords, locationLine, monthName, nameProblem, numberedName, originLine, relativeTime, sameNameLabel, stemOf } from "../lib/format";

const now = new Date(2026, 9, 12, 15, 0, 0); // 12 October 2026, 3 PM local
const unix = (d: Date) => Math.floor(d.getTime() / 1000);

describe("relativeTime", () => {
  it("reads as today, yesterday, weekday, then dates", () => {
    expect(relativeTime(unix(new Date(2026, 9, 12, 14, 14)), now)).toBe("today 2:14 PM");
    expect(relativeTime(unix(new Date(2026, 9, 11, 9, 0)), now)).toBe("yesterday");
    expect(relativeTime(unix(new Date(2026, 9, 8, 9, 0)), now)).toBe("Thursday");
    expect(relativeTime(unix(new Date(2026, 8, 1)), now)).toBe("1 September");
    expect(relativeTime(unix(new Date(2025, 8, 1)), now)).toBe("1 September 2025");
  });
});

describe("originLine and locationLine", () => {
  it("names the source for downloads and the edit time otherwise", () => {
    expect(originLine({ originLabel: "Messenger", downloadedAt: unix(new Date(2026, 9, 11)), modified: 0 }, now)).toBe("from Messenger, yesterday");
    expect(originLine({ originLabel: "Unknown source", downloadedAt: unix(new Date(2026, 9, 11)), modified: 0 }, now)).toBe("downloaded, yesterday");
    expect(originLine({ originLabel: null, downloadedAt: null, modified: unix(new Date(2026, 9, 11)) }, now)).toBe("edited yesterday");
  });

  it("turns folders into words", () => {
    const roots = ["C:\\Users\\Mom\\Desktop", "C:\\Users\\Mom\\Documents", "C:\\Users\\Mom\\Downloads"];
    expect(locationLine("C:\\Users\\Mom\\Documents\\School Files\\Word\\2026-10", roots)).toBe("in School Files > Word > October 2026");
    expect(locationLine("C:\\Users\\Mom\\Desktop", roots)).toBe("in Desktop");
    expect(locationLine("C:\\Users\\Mom\\Documents\\Forms", roots)).toBe("in Documents > Forms");
    expect(locationLine("D:\\Elsewhere", roots)).toBe("in D: > Elsewhere");
    expect(monthName("2026-13")).toBe("2026-13");
  });
});

describe("small helpers", () => {
  it("formats sizes, stems and copy counts", () => {
    expect(fileSize(900)).toBe("900 B");
    expect(fileSize(48_000)).toBe("47 KB");
    expect(fileSize(2.5 * 1024 * 1024)).toBe("2.5 MB");
    expect(stemOf("Lesson Plan.docx")).toBe("Lesson Plan");
    expect(extOf("Lesson Plan.docx")).toBe(".docx");
    expect(stemOf("noext")).toBe("noext");
    expect(copiesLabel(1)).toBe("1 other copy");
    expect(copiesLabel(3)).toBe("3 other copies");
  });
});

describe("isProgram", () => {
  it("knows the programs, scripts and shortcuts that run when opened (audit T-41)", () => {
    for (const ext of ["exe", "msi", "bat", "cmd", "ps1", "vbs", "js", "jse", "wsf", "hta", "lnk", "url", "scr", "com", "reg"]) expect(isProgram(ext)).toBe(true);
    expect(isProgram(".EXE")).toBe(true);
    expect(isProgram(extOf("setup-printer.exe"))).toBe(true);
    for (const ext of ["docx", "pdf", "xlsx", "jpg", "html", "zip", "", undefined]) expect(isProgram(ext)).toBe(false);
  });
});

describe("nameProblem", () => {
  // The sentences paths::validate_name answers with, so Rename can say them as she types (audit T-28).
  it("words every name Windows refuses the way the native side does", () => {
    expect(nameProblem("")).toBe("Type a name first.");
    expect(nameProblem("   ")).toBe("Type a name first.");
    expect(nameProblem("Quiz: Week 2")).toBe("Names cannot contain :. Try a space or a hyphen instead.");
    for (const bad of ["a/b", "a\\b", "a*b", "a?b", 'a"b', "a<b", "a>b", "a|b", "a\u0007b"]) expect(nameProblem(bad)).toMatch(/^Names cannot contain /);
    expect(nameProblem("Lesson Plan.")).toBe("Names cannot start or end with a space or a period. Remove it and try again.");
    expect(nameProblem(" Lesson Plan")).toBe("Names cannot start or end with a space or a period. Remove it and try again.");
    expect(nameProblem("con")).toBe("con is a name Windows reserves. Pick another.");
    expect(nameProblem("LPT1.backup")).toBe("LPT1.backup is a name Windows reserves. Pick another.");
    expect(nameProblem("x".repeat(201))).toBe("That name is too long. Keep it under 200 characters.");
    // The limit counts bytes, as Rust's str::len does: 100 letters with an accent are 200 bytes, 101 are too many.
    expect(nameProblem("ñ".repeat(100))).toBeNull();
    expect(nameProblem("ñ".repeat(101))).toBe("That name is too long. Keep it under 200 characters.");
  });

  it("takes ordinary names, a change of capitals included", () => {
    for (const ok of ["Grade 7 Science Quiz 2", "LESSON PLAN", "Peñaflor class list", "SF2 - October", "Teacher's Day", "console"]) expect(nameProblem(ok)).toBeNull();
  });
});

describe("month folders and same-name words", () => {
  it("shows a native folder label with its month folders in words", () => {
    expect(labelWords("School Files > Word > 2026-10")).toBe("School Files > Word > October 2026");
    expect(labelWords("Documents > Forms")).toBe("Documents > Forms");
    expect(labelWords("School Files > 2026-13")).toBe("School Files > 2026-13");
  });

  it("offers Name (2) on a clash and counts same-name files apart from copies", () => {
    expect(numberedName("Lesson Plan", ".docx")).toBe("Lesson Plan (2).docx");
    expect(numberedName("notes", "", 3)).toBe("notes (3)");
    expect(sameNameLabel(1)).toBe("1 other file with the same name");
    expect(sameNameLabel(2)).toBe("2 other files with the same name");
  });
});
