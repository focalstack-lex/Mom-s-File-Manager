import { describe, expect, it } from "vitest";
import { describeError, recycleFailureText } from "../lib/errors";
import { snippetHtml } from "../components/ResultRow";
import { describeOp, fileStillThere } from "../views/ActivityView";
import type { OpRow } from "../lib/types";

describe("describeError", () => {
  it("turns codes into one plain sentence", () => {
    expect(describeError({ code: "notFound", message: "x" })).toMatch(/no longer where it was/);
    expect(describeError({ code: "outsideRoots", message: "x" })).toMatch(/Desktop, Documents, Downloads/);
    expect(describeError({ code: "invalidName", message: "Names cannot contain :." })).toBe("Names cannot contain :.");
    expect(describeError(new Error("boom"))).toBe("boom");
    expect(describeError("weird")).toMatch(/Nothing was changed/);
  });

  it("words the Recycle Bin, in-use and update codes with a next step", () => {
    expect(describeError({ code: "inUse", message: "" })).toBe("That file is open in another program. Close it there, then try again.");
    expect(describeError({ code: "notInRecycleBin", message: "" })).toMatch(/Open the Recycle Bin to check/);
    expect(describeError({ code: "noRecycleBin", message: "" })).toMatch(/flash drive or a network folder/);
    expect(describeError({ code: "noRecycleBin", message: "This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first." })).toMatch(/too long/);
    expect(describeError({ code: "updateSignature", message: "" })).toMatch(/safety check/);
    expect(describeError({ code: "updateInstall", message: "" })).toMatch(/could not be installed/);
  });
});

describe("recycleFailureText", () => {
  const row = (src: string, status: OpRow["status"], message: string | null = null): OpRow => ({ id: 1, kind: "recycle", src, dst: null, status, message, automatic: false, createdAt: 0, batch: null });

  it("is silent when every file went to the Recycle Bin", () => {
    expect(recycleFailureText([row("C:\\a\\Quiz.docx", "done")])).toBeNull();
  });

  it("names the first file that stayed, with its reason, after what did move", () => {
    const refused = row("C:\\a\\Deep Quiz.docx", "failed", "This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first.");
    expect(recycleFailureText([refused])).toBe("Deep Quiz.docx: This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first.");
    expect(recycleFailureText([row("C:\\a\\A.docx", "done"), refused, row("C:\\a\\B.docx", "done")])).toBe("Moved 2 of 3 to the Recycle Bin. Deep Quiz.docx: This file's folder name is too long for the Recycle Bin. Move it to a shorter folder first.");
    expect(recycleFailureText([row("C:\\a\\C.docx", "failed")])).toBe("C.docx: Windows could not move it to the Recycle Bin.");
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
