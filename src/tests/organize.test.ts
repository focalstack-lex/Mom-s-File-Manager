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
  });

  it("folds one run into one entry and keeps other changes apart", () => {
    const entries = foldBatches([op(1, null), op(2, "b1"), op(3, "b1", `${SCHOOL}\\Science\\x3.docx`), op(4, "b1"), op(5, null)]);
    expect(entries.map((e) => e.type)).toEqual(["op", "batch", "op"]);
    const run = entries[1];
    expect(run.type === "batch" && run.ops.length).toBe(3);
    expect(run.type === "batch" && batchTitle(run.ops)).toBe("Organized 3 files into 2 folders");
  });
});
