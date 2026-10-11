import { describe, expect, it } from "vitest";
import { placeCard, TOUR_STEPS, useTour } from "../store/tour";

describe("walkthrough", () => {
  it("has five steps that each point at a real control", () => {
    expect(TOUR_STEPS).toHaveLength(5);
    expect(TOUR_STEPS.map((s) => s.target)).toEqual(["search", "recent", "open", "nav-cleanup", "nav-activity"]);
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
    for (let i = 0; i < 4; i += 1) tour.next();
    expect(useTour.getState().index).toBe(4);
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
});
