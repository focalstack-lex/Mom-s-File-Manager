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
  it("marks matches and escapes markup", () => {
    expect(snippetHtml("Grade 7 [Santos] <b>")).toBe("Grade 7 <mark>Santos</mark> &lt;b&gt;");
  });
});
