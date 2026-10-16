import { describe, expect, it } from "vitest";
import { hasWord, words } from "../lib/search";
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
