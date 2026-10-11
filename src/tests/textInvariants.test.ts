import { describe, expect, it } from "vitest";
import { isScannable, scanText } from "../../scripts/verify-text.mjs";

describe("text invariants gate", () => {
  it("finds dashes and emoji by code point", () => {
    const dash = String.fromCodePoint(0x2014);
    expect(scanText(`a ${dash} b`)).toEqual([{ line: 1, column: 3, kind: "dash", sample: dash }]);
    const emoji = String.fromCodePoint(0x1f600);
    expect(scanText(`ok\nx ${emoji}`)[0]).toMatchObject({ line: 2, kind: "emoji" });
    expect(scanText("plain - hyphen (c)")).toEqual([]);
  });

  it("skips binaries and lockfiles", () => {
    expect(isScannable("src/App.tsx")).toBe(true);
    expect(isScannable("src-tauri/icons/icon.ico")).toBe(false);
    expect(isScannable("package-lock.json")).toBe(false);
  });
});
