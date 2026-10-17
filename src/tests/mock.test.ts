// The harness's fake native side keeps the batch 6 contracts the UI drive
// relies on: move_each as one batch, rename with pickFree and a change of
// capitals, and same-name files kept apart from copies (audit T-16, T-28,
// T-30). The cargo tests hold the native side to the same rules.

import { describe, expect, it } from "vitest";
import type { CleanupReport, FindResult, MoveResult, OpRow, ToteError } from "../lib/types";

// vitest runs in node; the mock reads the page's query string for edge states.
Object.assign(globalThis, { window: Object.assign(globalThis, { location: { search: "" } }) });
const { installMockBackend } = await import("../dev/mockBackend");
installMockBackend();
const call = <T>(command: string, args: Record<string, unknown> = {}) => window.__toteMock?.invoke(command, args) as Promise<T>;
const DESKTOP = "C:\\Users\\Teacher\\Desktop";
const SCHOOL = "C:\\Users\\Teacher\\Documents\\School Files";

describe("mock backend contracts", () => {
  it("lists Word's default name on two different files as the same name, not as copies", async () => {
    const report = await call<CleanupReport>("cleanup_report");
    const same = report.sameName.find((g) => g.latest.name === "New Microsoft Word Document.docx");
    expect(same?.sameName).toHaveLength(1);
    expect(same?.others).toHaveLength(0);
    expect(report.duplicates.some((g) => g.latest.name === "New Microsoft Word Document.docx")).toBe(false);
    // Copies with a copy mark and the same size stay copies.
    expect(report.duplicates.find((g) => g.latest.name === "Lesson Plan final_final.docx")?.others).toHaveLength(2);
    const found = await call<FindResult>("find", { query: "new microsoft", filters: { kinds: [], sources: [] } });
    expect(found.groups[0]).toMatchObject({ others: [], sameName: [{ name: "New Microsoft Word Document.docx" }] });
  });

  it("renames a change of capitals, refuses a clash, then picks Name (2) when asked", async () => {
    const path = `${DESKTOP}\\Class Record Grade 7.xlsx`;
    const capitals = await call<string>("rename_file", { path, newStem: "CLASS RECORD GRADE 7", pickFree: false });
    expect(capitals).toBe(`${DESKTOP}\\CLASS RECORD GRADE 7.xlsx`);
    const clash = `${DESKTOP}\\Lesson Plan final_final.docx`;
    const refused = await call<string>("rename_file", { path: clash, newStem: "lesson plan", pickFree: false }).catch((e: ToteError) => e);
    expect(refused).toMatchObject({ code: "alreadyExists" });
    expect(await call<string>("rename_file", { path: clash, newStem: "Lesson Plan", pickFree: true })).toBe(`${DESKTOP}\\Lesson Plan (3).docx`);
    await expect(call("rename_file", { path: `${DESKTOP}\\Lesson Plan.docx`, newStem: "Quiz: 2", pickFree: false })).rejects.toMatchObject({ code: "invalidName", message: "Names cannot contain :. Try a space or a hyphen instead." });
  });

  it("moves several files to their own folders as one batch and names a file that stayed", async () => {
    const result = await call<MoveResult>("move_each", {
      moves: [
        { path: `${DESKTOP}\\Lesson Plan.docx`, destination: `${SCHOOL}\\Word\\2026-10` },
        { path: `${DESKTOP}\\CLASS RECORD GRADE 7.xlsx`, destination: `${SCHOOL}\\Excel\\2026-10` },
        { path: `${DESKTOP}\\Gone.docx`, destination: `${SCHOOL}\\Word\\2026-10` },
      ],
    });
    expect(result.moved).toEqual([`${SCHOOL}\\Word\\2026-10\\Lesson Plan.docx`, `${SCHOOL}\\Excel\\2026-10\\CLASS RECORD GRADE 7.xlsx`]);
    expect(result.failed).toEqual([{ name: "Gone.docx", message: "That file is no longer there. Search for it in Find." }]);
    const activity = await call<OpRow[]>("list_activity");
    expect(activity.filter((op) => op.batch === result.batch)).toHaveLength(2);
    expect(result.batch).toMatch(/^move-/);
  });
});
