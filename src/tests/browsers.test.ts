import { describe, expect, it } from "vitest";
import { browserLabel, browserNotice } from "../lib/browsers";
import type { BrowserStatus } from "../lib/types";

const ok = (browser: string, profile = "Default"): BrowserStatus => ({ browser, profile, ok: true, asks: false, folder: null, advice: "Saves to Downloads." });
const bad = (browser: string, profile = "Default"): BrowserStatus => ({ browser, profile, ok: false, asks: true, folder: null, advice: "Asks where to save each file." });

describe("browser notice", () => {
  it("stays hidden until the check returns and when every browser is fine", () => {
    expect(browserNotice(null)).toBeNull();
    expect(browserNotice([])).toBeNull();
    expect(browserNotice([ok("Chrome"), ok("Edge")])).toBeNull();
  });

  it("names the browsers that need a change, once each", () => {
    expect(browserNotice([ok("Chrome"), bad("Edge")])).toBe("Edge saves some downloads outside Downloads, so Recent downloads may miss them.");
    expect(browserNotice([bad("Chrome", "A"), bad("Chrome", "B"), bad("Edge"), bad("Firefox")])).toBe(
      "Chrome, Edge and Firefox save some downloads outside Downloads, so Recent downloads may miss them.",
    );
  });

  it("adds the profile only when a browser has several", () => {
    const all = [ok("Chrome", "Mom"), bad("Chrome", "School"), ok("Edge")];
    expect(browserLabel(all[0], all)).toBe("Chrome (Mom)");
    expect(browserLabel(all[2], all)).toBe("Edge");
  });
});
