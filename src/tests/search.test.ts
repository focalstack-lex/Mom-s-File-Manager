import { describe, expect, it } from "vitest";
import { hasWord, indexedName, words } from "../lib/search";
import { useFind } from "../store/find";

describe("words", () => {
  it("cuts typed text the way the index cuts names", () => {
    expect(words("Book1.xlsx")).toEqual(["book1", "xlsx"]);
    expect(words("SF2_October")).toEqual(["sf2", "october"]);
    expect(words("Teacher's Day")).toEqual(["teacher", "s", "day"]);
    expect(words("2026-10-14")).toEqual(["2026", "10", "14"]);
    expect(words("Dela Cruz, Juan")).toEqual(["dela", "cruz", "juan"]);
    expect(words("--- ' ...")).toEqual([]);
  });

  it("knows punctuation alone is no word", () => {
    expect(hasWord("Grade-7")).toBe(true);
    expect(hasWord("---")).toBe(false);
    expect(hasWord("   ")).toBe(false);
    expect(hasWord("")).toBe(false);
  });
});

describe("indexedName", () => {
  it("adds the apostrophe-free form, so teachers day finds Teacher's Day", () => {
    expect(indexedName("Teacher's Day.docx")).toBe("Teacher's Day.docx Teachers Day.docx");
    expect(indexedName("Juan’s Report.pdf")).toBe("Juan’s Report.pdf Juans Report.pdf");
    expect(indexedName("Book1.xlsx")).toBe("Book1.xlsx");
    expect(words(indexedName("Teacher's Day.docx"))).toContain("teachers");
  });
});

describe("isSearching", () => {
  it("is false for punctuation alone, so the pinned and recent lists stay", () => {
    useFind.setState({ query: "---" });
    expect(useFind.getState().isSearching()).toBe(false);
    useFind.setState({ query: "Grade-7" });
    expect(useFind.getState().isSearching()).toBe(true);
    useFind.setState({ query: "---", kinds: ["word"] });
    expect(useFind.getState().isSearching()).toBe(true);
    useFind.setState({ query: "", kinds: [] });
  });
});
