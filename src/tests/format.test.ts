import { describe, expect, it } from "vitest";
import { copiesLabel, extOf, fileSize, isProgram, locationLine, monthName, originLine, relativeTime, stemOf } from "../lib/format";

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
