import { describe, expect, it } from "vitest";
import { describeError } from "../lib/errors";
import { snippetHtml } from "../components/ResultRow";

describe("describeError", () => {
  it("turns codes into one plain sentence", () => {
    expect(describeError({ code: "notFound", message: "x" })).toMatch(/no longer where it was/);
    expect(describeError({ code: "outsideRoots", message: "x" })).toMatch(/Desktop, Documents, Downloads/);
    expect(describeError({ code: "invalidName", message: "Names cannot contain :." })).toBe("Names cannot contain :.");
    expect(describeError(new Error("boom"))).toBe("boom");
    expect(describeError("weird")).toMatch(/Nothing was changed/);
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
