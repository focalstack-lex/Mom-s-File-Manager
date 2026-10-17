import { describe, expect, it } from "vitest";
import { CODE_SENTENCES, describeError, isPlain, moveEachText, plainMessage, recycleFailureText, undoText } from "../lib/errors";
import { snippetHtml, snippetText } from "../components/ResultRow";
import { describeOp, fileStillThere } from "../views/ActivityView";
import type { OpRow, ToteError } from "../lib/types";

/** Program text the native side used to pass through (probe-security.json in the 2026-10-16 audit). */
const RAW_TEXTS = [
  "The process cannot access the file because it is being used by another process. (os error 32)",
  'Windows could not move it to the Recycle Bin: Error during a `trash` operation: Unknown { description: "Some operations were aborted" }',
  "Access is denied. (os error 5)",
  "ToteError { code: Io, message: \"x\" }",
  "Error: something::deep panicked",
];
const PROGRAM_TEXT = /os error|Error during|[{}`]|::|panicked/;

// CODE_SENTENCES is typed over every ErrorCode, so its keys are the whole table.
const CODES = Object.keys(CODE_SENTENCES) as ToteError["code"][];

describe("describeError", () => {
  it("has one plain sentence with a next step for every code", () => {
    expect(CODES).toHaveLength(16);
    for (const code of CODES) {
      const sentence = describeError({ code, message: "" });
      expect(sentence, code).toBe(CODE_SENTENCES[code]);
      expect(sentence, code).toMatch(/^[A-Z].*\.$/);
      expect(sentence, code).not.toMatch(PROGRAM_TEXT);
    }
  });

  it("never passes program text through, for any code", () => {
    for (const code of CODES) {
      for (const message of RAW_TEXTS) {
        const shown = describeError({ code, message });
        expect(shown, `${code}: ${message}`).toBe(CODE_SENTENCES[code]);
        expect(shown, code).not.toMatch(PROGRAM_TEXT);
      }
    }
  });

  it("keeps the native sentence when it is already plain", () => {
    expect(describeError({ code: "notInRecycleBin", message: "That file is not in the Recycle Bin any more. Open the Recycle Bin to check." })).toBe("That file is not in the Recycle Bin any more. Open the Recycle Bin to check.");
    expect(describeError({ code: "invalidName", message: "Names cannot contain :." })).toBe("Names cannot contain :.");
    expect(describeError({ code: "noRecycleBin", message: "This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first." })).toMatch(/too long/);
    expect(describeError({ code: "inUse", message: "" })).toBe("That file is open in another program. Close it there, then try again.");
    expect(describeError({ code: "updateSignature", message: "" })).toMatch(/safety check/);
    expect(describeError({ code: "updateInstall", message: "" })).toMatch(/could not be installed/);
  });

  it("separates a failed read from a failed save", () => {
    expect(describeError({ code: "db", message: "" })).toBe("Tote could not read its list right now. Try again in a moment.");
    expect(describeError({ code: "db", message: "Tote could not save that. Try again in a moment." })).toBe("Tote could not save that. Try again in a moment.");
  });

  it("names no folders for outsideRoots and never shows a renderer Error's text", () => {
    expect(describeError({ code: "outsideRoots", message: "" })).not.toMatch(/Desktop, Documents/);
    expect(describeError(new Error("TypeError: cannot read properties of undefined"))).toBe("Something went wrong. Nothing was changed.");
    expect(describeError("weird")).toMatch(/Nothing was changed/);
  });

  it("tells plain words from program text", () => {
    expect(isPlain("Close it there, then try again.")).toBe(true);
    expect(isPlain("")).toBe(false);
    expect(isPlain(null)).toBe(false);
    for (const text of RAW_TEXTS) expect(isPlain(text), text).toBe(false);
    expect(plainMessage(RAW_TEXTS[0], "Fallback.")).toBe("Fallback.");
    expect(plainMessage("Kept.", "Fallback.")).toBe("Kept.");
  });
});

describe("undoText", () => {
  it("says everything went back when nothing failed", () => {
    expect(undoText(1, [])).toBe("Put back where it was.");
    expect(undoText(3, [])).toBe("Everything is back where it was.");
  });

  it("counts what went back and names the first file that did not, with its reason", () => {
    const inUse = "That file is open in another program. Close it there, then try again.";
    expect(undoText(5, [{ name: "Grade 7 Quiz.docx", reason: inUse }])).toBe(`Put back 5 of 6. Grade 7 Quiz.docx: ${inUse}`);
    expect(undoText(0, [{ name: "Grade 7 Quiz.docx", reason: inUse }])).toBe(`Grade 7 Quiz.docx: ${inUse}`);
    expect(undoText(0, [{ name: "A.docx", reason: null }, { name: "B.docx", reason: null }])).toBe("None of the 2 could be put back. A.docx: It could not be moved back. It may be open in another program; close it there, then undo again from Activity.");
    expect(undoText(1, [{ name: "A.docx", reason: RAW_TEXTS[0] }])).not.toMatch(PROGRAM_TEXT);
  });
});

describe("recycleFailureText", () => {
  const row = (src: string, status: OpRow["status"], message: string | null = null): OpRow => ({ id: 1, kind: "recycle", src, dst: null, status, message, automatic: false, createdAt: 0, batch: null, stampLost: false });

  it("is silent when every file went to the Recycle Bin", () => {
    expect(recycleFailureText([row("C:\\a\\Quiz.docx", "done")])).toBeNull();
  });

  it("names the first file that stayed, with its reason, after what did move", () => {
    const refused = row("C:\\a\\Deep Quiz.docx", "failed", "This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first.");
    expect(recycleFailureText([refused])).toBe("Deep Quiz.docx: This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first.");
    expect(recycleFailureText([row("C:\\a\\A.docx", "done"), refused, row("C:\\a\\B.docx", "done")])).toBe("Moved 2 of 3 to the Recycle Bin. Deep Quiz.docx: This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first.");
    const fallback = "C.docx: Windows could not move it to the Recycle Bin. Open the Recycle Bin to check, or delete it in File Explorer if you are sure.";
    expect(recycleFailureText([row("C:\\a\\C.docx", "failed")])).toBe(fallback);
    expect(recycleFailureText([row("C:\\a\\C.docx", "failed", RAW_TEXTS[1])])).toBe(fallback);
  });

  it("words a failed Recycle Bin row in Activity with its reason, and hides Show the file once the file is gone", () => {
    const roots = ["C:\\a"];
    expect(describeOp(row("C:\\a\\Quiz.docx", "done"), roots).title).toBe("Moved Quiz.docx to the Recycle Bin");
    const refused = row("C:\\a\\Deep Quiz.docx", "failed", "This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first.");
    expect(describeOp(refused, roots)).toEqual({ title: "Could not send Deep Quiz.docx to the Recycle Bin", detail: "This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first." });
    expect(fileStillThere(refused)).toBe(true);
    const gone = row("C:\\a\\Photos.zip", "failed", "Windows removed it without putting it in the Recycle Bin, so it cannot be brought back.");
    expect(describeOp(gone, roots).title).toBe("Could not send Photos.zip to the Recycle Bin");
    expect(fileStillThere(gone)).toBe(false);
    expect(fileStillThere(row("C:\\a\\C.docx", "failed"))).toBe(true);
  });
});

describe("describeOp for changes that failed", () => {
  const roots = ["C:\\u\\Documents", "C:\\u\\Desktop"];
  const op = (kind: OpRow["kind"], src: string, dst: string | null, status: OpRow["status"], message: string | null): OpRow => ({ id: 7, kind, src, dst, status, message, automatic: false, createdAt: 0, batch: null, stampLost: false });
  const inUse = "That file is open in another program. Close it there, then try again.";

  it("says Could not move or rename with the reason, never the success sentence", () => {
    const move = describeOp(op("move", "C:\\u\\Desktop\\Quiz.docx", "C:\\u\\Documents\\School Files\\Word\\Quiz.docx", "failed", inUse), roots);
    expect(move).toEqual({ title: "Could not move Quiz.docx", detail: inUse });
    const rename = describeOp(op("rename", "C:\\u\\Desktop\\Quiz.docx", "C:\\u\\Desktop\\Quiz 2.docx", "failed", inUse), roots);
    expect(rename).toEqual({ title: "Could not rename Quiz.docx", detail: inUse });
    expect(describeOp(op("newFolder", "C:\\u\\Desktop\\New", null, "failed", null), roots).title).toBe("Could not make the folder New");
    expect(describeOp(op("copy", "C:\\u\\Documents\\Forms\\SF9.docx", null, "failed", null), roots).title).toBe("Could not make a copy of SF9.docx");
  });

  it("gives a next step when the row has no reason and replaces program text", () => {
    expect(describeOp(op("move", "C:\\u\\Desktop\\Quiz.docx", null, "failed", null), roots).detail).toBe("It is still where it was. Try again from Find.");
    expect(describeOp(op("rename", "C:\\u\\Desktop\\Quiz.docx", null, "failed", RAW_TEXTS[0]), roots).detail).not.toMatch(PROGRAM_TEXT);
    const filing = describeOp(op("autoSort", "C:\\u\\Downloads\\budget.xlsx", null, "failed", RAW_TEXTS[2]), roots);
    expect(filing).toEqual({ title: "Could not file budget.xlsx", detail: "It is still where it was. Right-click it in File Explorer to try again." });
  });

  it("keeps the success wording for changes that worked", () => {
    expect(describeOp(op("move", "C:\\u\\Desktop\\Quiz.docx", "C:\\u\\Documents\\School Files\\Word\\Quiz.docx", "done", null), roots).title).toBe("Moved Quiz.docx");
    expect(describeOp(op("rename", "C:\\u\\Desktop\\Quiz.docx", "C:\\u\\Desktop\\Quiz 2.docx", "done", null), roots)).toEqual({ title: "Renamed Quiz.docx", detail: "now Quiz 2.docx" });
  });
});

describe("snippetHtml", () => {
  it("marks matches wrapped in the private-use marks and escapes markup", () => {
    expect(snippetHtml("Grade 7 \uE000Santos\uE001 <b>")).toBe("Grade 7 <mark>Santos</mark> &lt;b&gt;");
  });

  it("shows literal brackets from a document as plain text", () => {
    expect(snippetHtml("[\uE000Name\uE001] [Section] [[Santos]]")).toBe("[<mark>Name</mark>] [Section] [[Santos]]");
    expect(snippetHtml("[Name] [Section]")).toBe("[Name] [Section]");
  });

  it("drops a stray mark instead of showing it", () => {
    expect(snippetHtml("odd \uE000mark")).toBe("odd mark");
  });
});

describe("snippetText", () => {
  it("gives a screen reader the snippet's words without the marks (audit T-22)", () => {
    const [start, end] = [String.fromCharCode(0xe000), String.fromCharCode(0xe001)];
    expect(snippetText(`Santos ${start}Reyes${end} Dela Cruz [Name]`)).toBe("Santos Reyes Dela Cruz [Name]");
  });
});

describe("moveEachText", () => {
  const inUse = "That file is open in another program. Close it there, then try again.";
  it("says how many moved, with the right plural (audit T-30)", () => {
    expect(moveEachText({ batch: "move-1", moved: ["a", "b"], failed: [] }, "School Files")).toEqual({ text: "Moved 2 files into School Files.", error: false });
    expect(moveEachText({ batch: null, moved: ["a"], failed: [] }, "School Files")).toEqual({ text: "Moved 1 file into School Files.", error: false });
  });

  it("names the first file that stayed with its reason, and never passes program text", () => {
    expect(moveEachText({ batch: "move-1", moved: ["a"], failed: [{ name: "Quiz.docx", message: inUse }] }, "School Files")).toEqual({ text: `Moved 1 of 2 files into School Files. Quiz.docx: ${inUse}`, error: true });
    expect(moveEachText({ batch: null, moved: [], failed: [{ name: "Quiz.docx", message: "Access is denied. (os error 5)" }, { name: "B.pdf", message: inUse }] }, "School Files").text).toBe("None of the 2 could be moved. Quiz.docx: It is still where it was. Try again in a moment.");
  });
});
