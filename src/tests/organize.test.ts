import { describe, expect, it } from "vitest";
import { batchTitle, folderLabel, foldBatches, parseWords, planSummary, wordsText } from "../lib/organize";
import type { OpRow } from "../lib/types";

const SCHOOL = "C:\\Users\\Mom\\Documents\\School Files";

describe("organize words", () => {
  it("labels folders relative to School Files, months in words", () => {
    expect(folderLabel(`${SCHOOL}\\Grade 7\\Science`, SCHOOL)).toBe("Grade 7 > Science");
    expect(folderLabel(`${SCHOOL}\\Word\\2026-10`, SCHOOL)).toBe("Word > October 2026");
    expect(folderLabel(SCHOOL, SCHOOL)).toBe("School Files");
  });

  it("summarizes plans in plain words", () => {
    expect(planSummary(84, 9)).toBe("84 files into 9 folders");
    expect(planSummary(1, 1)).toBe("1 file into 1 folder");
    expect(planSummary(1200, 3)).toBe("1,200 files into 3 folders");
  });

  it("reads and writes the comma-separated word lists", () => {
    expect(parseWords(" Math, mathematics,, MATH ,")).toEqual(["math", "mathematics"]);
    expect(wordsText(["math", "mathematics"])).toBe("math, mathematics");
    expect(parseWords("")).toEqual([]);
  });
});

describe("activity folding", () => {
  const op = (id: number, batch: string | null, dst = `${SCHOOL}\\Grade 7\\x${id}.docx`): OpRow => ({
    id,
    kind: "move",
    src: `C:\\Users\\Mom\\Desktop\\x${id}.docx`,
    dst,
    status: "done",
    message: null,
    automatic: false,
    createdAt: 1000 - id,
    batch,
    stampLost: false,
  });

  it("folds one run into one entry and keeps other changes apart", () => {
    const entries = foldBatches([op(1, null), op(2, "b1"), op(3, "b1", `${SCHOOL}\\Science\\x3.docx`), op(4, "b1"), op(5, null)]);
    expect(entries.map((e) => e.type)).toEqual(["op", "batch", "op"]);
    const run = entries[1];
    expect(run.type === "batch" && run.ops.length).toBe(3);
    expect(run.type === "batch" && batchTitle(run.ops)).toBe("Organized 3 files into 2 folders");
  });

  it("names a batch of ticked files by what was done to them", () => {
    const recycled = [op(1, "recycle-1"), op(2, "recycle-1")].map((o) => ({ ...o, kind: "recycle" as const, dst: null }));
    expect(batchTitle(recycled)).toBe("Moved 2 files to the Recycle Bin");
    const moved = [op(3, "move-1", `${SCHOOL}\\Word\\2026-10\\a.docx`), op(4, "move-1", `${SCHOOL}\\Word\\2026-10\\b.docx`)];
    expect(batchTitle(moved)).toBe("Moved 2 files to October 2026");
  });

  it("keeps a refused file inside its batch and counts only the files that went", () => {
    const refused = { ...op(2, "recycle-1"), kind: "recycle" as const, dst: null, status: "failed" as const, message: "Files on a flash drive or a network folder cannot go to the Recycle Bin. Delete it in File Explorer if you are sure." };
    const went = { ...op(1, "recycle-1"), kind: "recycle" as const, dst: null };
    const entries = foldBatches([went, refused, op(3, null)]);
    expect(entries.map((e) => e.type)).toEqual(["batch", "op"]);
    expect(entries[0].type === "batch" && entries[0].ops.map((o) => o.status)).toEqual(["done", "failed"]);
    expect(batchTitle([went, refused])).toBe("Moved 1 of 2 files to the Recycle Bin");
    expect(batchTitle([refused, { ...refused, id: 5 }])).toBe("Could not move 2 files to the Recycle Bin");
    // Undo all says what came back, never what had gone; the refused row
    // keeps its own reason under the row.
    expect(batchTitle([{ ...went, status: "undone" }, refused])).toBe("Put back 1 of 2 files");
    expect(batchTitle([{ ...went, status: "undone" }])).toBe("Put back 1 file");
    const stuck = { ...op(4, "move-1", `${SCHOOL}\\Word\\2026-10\\b.docx`), status: "failed" as const, message: "That file is open in another program. Close it there, then try again." };
    expect(batchTitle([op(3, "move-1", `${SCHOOL}\\Word\\2026-10\\a.docx`), stuck])).toBe("Moved 1 of 2 files to October 2026");
    expect(batchTitle([stuck])).toBe("Could not move 1 file to October 2026");
    expect(batchTitle([op(6, "organize-1"), { ...op(7, "organize-1", `${SCHOOL}\\Science\\x7.docx`), status: "failed" as const }])).toBe("Organized 1 of 2 files into 1 folder");
    expect(batchTitle([{ ...op(8, "organize-1"), status: "failed" as const }])).toBe("Could not organize 1 file");
  });

  it("titles a Move to Tote of several files as filed", () => {
    const sent = (id: number, dst: string) => ({ ...op(id, "tote-1", dst), kind: "autoSort" as const, automatic: true });
    expect(batchTitle([sent(1, `${SCHOOL}\\Grade 7\\a.docx`), sent(2, `${SCHOOL}\\PDF\\2026-10\\b.pdf`)])).toBe("Filed 2 files into 2 folders");
    expect(batchTitle([sent(3, `${SCHOOL}\\Grade 7\\c.docx`), { ...sent(4, `${SCHOOL}\\Grade 7\\d.docx`), status: "failed" as const }])).toBe("Filed 1 of 2 files into 1 folder");
    expect(batchTitle([{ ...sent(5, `${SCHOOL}\\Grade 7\\e.docx`), status: "failed" as const }])).toBe("Could not file 1 file");
  });
});
