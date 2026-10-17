import { describe, expect, it } from "vitest";
import { placeCard, TOUR_STEPS, useTour } from "../store/tour";

describe("walkthrough", () => {
  it("has six steps that each point at a real control", () => {
    expect(TOUR_STEPS).toHaveLength(6);
    expect(TOUR_STEPS.map((s) => s.target)).toEqual(["search", "recent", "open", "nav-todo", "nav-cleanup", "nav-activity"]);
    for (const step of TOUR_STEPS) {
      expect(step.title.length).toBeLessThan(40);
      expect(step.body.split(" ").length).toBeLessThan(30);
    }
  });

  it("steps forward, back, and finishes after the last step", () => {
    const tour = useTour.getState();
    tour.start();
    expect(useTour.getState()).toMatchObject({ active: true, index: 0 });
    tour.back();
    expect(useTour.getState().index).toBe(0);
    for (let i = 0; i < 5; i += 1) tour.next();
    expect(useTour.getState().index).toBe(5);
    tour.next();
    expect(useTour.getState().active).toBe(false);
  });

  it("places the card where it never covers its target", () => {
    const viewport = { width: 1280, height: 800 };
    const card = { width: 360, height: 200 };
    const wide = { top: 100, left: 100, width: 800, height: 80 };
    expect(placeCard(wide, card, viewport)).toMatchObject({ side: "below", top: 100 + 80 + 24 });
    const bottomNarrow = { top: 700, left: 20, width: 200, height: 48 };
    expect(placeCard(bottomNarrow, card, viewport).side).toBe("right");
    const bottomWide = { top: 700, left: 100, width: 900, height: 48 };
    expect(placeCard(bottomWide, card, viewport).side).toBe("above");
    const corner = { top: 700, left: 1000, width: 260, height: 60 };
    const placed = placeCard(corner, card, viewport);
    expect(placed.left + card.width).toBeLessThanOrEqual(viewport.width - 16 + 0.001);
  });

  // Audit T-26: on small windows the left fallback was clamped onto the target.
  it("never covers the ring when any side has room, at 640x480 and 1093x566, and stays under the title bar", () => {
    const covers = (a: { top: number; left: number; width: number; height: number }, b: typeof a) =>
      Math.min(a.left + a.width, b.left + b.width) > Math.max(a.left, b.left) && Math.min(a.top + a.height, b.top + b.height) > Math.max(a.top, b.top);
    const cases = [
      { viewport: { width: 640, height: 480 }, card: { width: 320, height: 190 }, target: { top: 60, left: 72, width: 520, height: 77 } },
      { viewport: { width: 640, height: 480 }, card: { width: 320, height: 190 }, target: { top: 300, left: 72, width: 520, height: 120 } },
      { viewport: { width: 640, height: 480 }, card: { width: 320, height: 190 }, target: { top: 200, left: 6, width: 52, height: 64 } },
      { viewport: { width: 1093, height: 566 }, card: { width: 320, height: 190 }, target: { top: 150, left: 280, width: 770, height: 77 } },
      { viewport: { width: 1093, height: 566 }, card: { width: 320, height: 190 }, target: { top: 420, left: 960, width: 90, height: 60 } },
      { viewport: { width: 1093, height: 566 }, card: { width: 320, height: 190 }, target: { top: 250, left: 4, width: 228, height: 68 } },
    ];
    for (const { viewport, card, target } of cases) {
      const placed = placeCard(target, card, viewport, 24, 52);
      expect(placed.fits).toBe(true);
      expect(covers({ top: placed.top, left: placed.left, ...card }, target)).toBe(false);
      expect(placed.top).toBeGreaterThanOrEqual(52);
      expect(placed.top + card.height).toBeLessThanOrEqual(viewport.height - 16);
      expect(placed.left).toBeGreaterThanOrEqual(16);
      expect(placed.left + card.width).toBeLessThanOrEqual(viewport.width - 16);
    }
  });

  it("says when no side has room, so the walkthrough scrolls the target first", () => {
    const placed = placeCard({ top: 150, left: 72, width: 520, height: 160 }, { width: 320, height: 190 }, { width: 640, height: 480 }, 24, 52);
    expect(placed.fits).toBe(false);
    expect(placed.top).toBeGreaterThanOrEqual(52);
  });
});
