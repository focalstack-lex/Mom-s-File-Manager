// The first-run walkthrough (spec 5.9). The page dims except for the control
// being explained; a card with an arrow drawn to that control says what it
// does, beside it and never on it. Enter is Next unless Skip or Back has
// focus (then Enter presses that button), Escape is Skip, and Tab stays
// inside the card.

import { ArrowLeft, ArrowRight } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { trapTab } from "../lib/focus";
import { placeCard, TOUR_STEPS, useTour, type Box } from "../store/tour";

type Rect = Box;

const PAD = 8;

/** The ring's box around a target: the control plus a little room. */
function ringBox(el: HTMLElement): Rect {
  const r = el.getBoundingClientRect();
  return { top: r.top - PAD, left: r.left - PAD, width: r.width + PAD * 2, height: r.height + PAD * 2 };
}

/** The card stays under the title bar, which sits above the walkthrough so the window can always be moved. */
function cardTop(): number {
  return (document.querySelector('[data-testid="titlebar"]')?.getBoundingClientRect().bottom ?? 0) + PAD;
}

export function Tour({ onFinish }: { onFinish(): void }) {
  const active = useTour((s) => s.active);
  const index = useTour((s) => s.index);
  const next = useTour((s) => s.next);
  const back = useTour((s) => s.back);
  const finish = useTour((s) => s.finish);
  const [target, setTarget] = useState<Rect | null>(null);
  const [card, setCard] = useState({ width: 360, height: 200 });
  const [viewport, setViewport] = useState({ width: window.innerWidth, height: window.innerHeight });
  const [minTop, setMinTop] = useState(16);
  const cardRef = useRef<HTMLDivElement>(null);
  const step = TOUR_STEPS[index];

  useLayoutEffect(() => {
    if (!active) return;
    const find = () => document.querySelector<HTMLElement>(`[data-tour="${step.target}"]`);
    // Where the target is now, without scrolling: content above it can still
    // arrive (the browser notice, the first answer) and push it down.
    const place = () => {
      const el = find();
      setTarget(el ? ringBox(el) : null);
      setViewport({ width: window.innerWidth, height: window.innerHeight });
      setMinTop(cardTop());
      // Fractional, unlike offsetWidth/offsetHeight: the ring's box is also
      // fractional, and a rounded-up height lands the card 0.4 px inside it
      // at 640x480 (measured 2026-10-18).
      if (cardRef.current) {
        const box = cardRef.current.getBoundingClientRect();
        setCard({ width: box.width, height: box.height });
      }
    };
    // Bring the target on screen; when the card then has no free side
    // (a small window), move the target to the top of its scrolling area
    // and then to the bottom, and keep the first spot where the card fits.
    const measure = () => {
      const el = find();
      if (el) {
        const box = cardRef.current?.getBoundingClientRect();
        const size = box ? { width: box.width, height: box.height } : { width: 360, height: 200 };
        const view = { width: window.innerWidth, height: window.innerHeight };
        const fits = () => placeCard(ringBox(el), size, view, 24, cardTop()).fits;
        el.scrollIntoView({ block: "nearest", inline: "nearest" });
        if (!fits()) {
          el.scrollIntoView({ block: "start", inline: "nearest" });
          if (!fits()) el.scrollIntoView({ block: "end", inline: "nearest" });
        }
      }
      place();
    };
    measure();
    const raf = window.requestAnimationFrame(measure);
    window.addEventListener("resize", measure);
    // Follow layout shifts and scrolling so the ring and arrow stay on the target.
    const observer = new ResizeObserver(place);
    const target = find();
    if (target) observer.observe(target);
    const view = document.querySelector('[data-testid="main"] > *');
    if (view) observer.observe(view);
    window.addEventListener("scroll", place, true);
    // A view eases in with a short slide; place the ring again once it lands.
    window.addEventListener("animationend", place, true);
    return () => {
      window.removeEventListener("animationend", place, true);
      window.cancelAnimationFrame(raf);
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", place, true);
      observer.disconnect();
    };
  }, [active, step]);

  useEffect(() => {
    if (!active) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        finish();
        onFinish();
      } else if (event.key === "Enter") {
        // A focused card button (Skip, Back, Next) takes Enter itself, as any
        // button does; Enter anywhere else means Next.
        const focused = document.activeElement;
        if (focused instanceof HTMLButtonElement && cardRef.current?.contains(focused)) return;
        event.preventDefault();
        if (index >= TOUR_STEPS.length - 1) onFinish();
        next();
      } else if (event.key === "Tab" && cardRef.current) {
        trapTab(event, cardRef.current);
      }
    };
    window.addEventListener("keydown", onKey);
    cardRef.current?.querySelector<HTMLElement>("button.button-primary")?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, [active, index, next, finish, onFinish]);

  if (!active) return null;

  const fallback: Rect = { top: viewport.height / 2 - 40, left: viewport.width / 2 - 160, width: 320, height: 80 };
  const box = target ?? fallback;
  const placed = placeCard(box, card, viewport, 24, minTop);
  const from = anchorPoint(placed, card, box);
  const to = edgePoint(box, from);
  const control = { x: (from.x + to.x) / 2 + (placed.side === "below" || placed.side === "above" ? 40 : 0), y: (from.y + to.y) / 2 + (placed.side === "left" || placed.side === "right" ? 30 : 0) };

  return (
    <div className="tour-layer" data-testid="tour" role="dialog" aria-modal="true" aria-labelledby="tour-title">
      <svg className="tour-mask" aria-hidden>
        <defs>
          <mask id="tour-hole">
            <rect x="0" y="0" width="100%" height="100%" fill="white" />
            {target && <rect x={box.left} y={box.top} width={box.width} height={box.height} rx="16" fill="black" />}
          </mask>
          <marker id="tour-head" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse">
            <path d="M 0 0 L 10 5 L 0 10 z" fill="var(--accent)" />
          </marker>
        </defs>
        <rect x="0" y="0" width="100%" height="100%" fill="rgba(23, 32, 27, 0.55)" mask="url(#tour-hole)" />
      </svg>
      {target && <div className="tour-ring" style={{ top: box.top, left: box.left, width: box.width, height: box.height }} />}
      {target && (
        <svg className="tour-arrow" aria-hidden>
          <path d={`M ${from.x} ${from.y} Q ${control.x} ${control.y} ${to.x} ${to.y}`} fill="none" stroke="var(--accent)" strokeWidth="3" strokeLinecap="round" markerEnd="url(#tour-head)" />
        </svg>
      )}
      <div className="tour-card" ref={cardRef} style={{ top: placed.top, left: placed.left }}>
        <div className="tour-progress" aria-hidden>
          {TOUR_STEPS.map((s, i) => (
            <span key={s.target} className={i <= index ? "is-done" : undefined} />
          ))}
        </div>
        <span className="tour-step">
          Step {index + 1} of {TOUR_STEPS.length}
        </span>
        <h2 id="tour-title">{step.title}</h2>
        <p>{step.body}</p>
        <div className="tour-actions">
          <button
            type="button"
            className="button button-quiet"
            onClick={() => {
              finish();
              onFinish();
            }}
          >
            Skip
          </button>
          {index > 0 && (
            <button type="button" className="button" onClick={back} aria-label="Back">
              <ArrowLeft size={18} aria-hidden />
              Back
            </button>
          )}
          <button
            type="button"
            className="button button-primary"
            onClick={() => {
              if (index >= TOUR_STEPS.length - 1) onFinish();
              next();
            }}
            data-testid="tour-next"
          >
            {index >= TOUR_STEPS.length - 1 ? "Done" : "Next"}
            {index < TOUR_STEPS.length - 1 && <ArrowRight size={18} aria-hidden />}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Where the arrow leaves the card: the edge facing the target. */
function anchorPoint(placed: { top: number; left: number; side: string }, card: { width: number; height: number }, box: Rect) {
  switch (placed.side) {
    case "below":
      return { x: Math.min(Math.max(box.left + box.width / 2, placed.left + 32), placed.left + card.width - 32), y: placed.top };
    case "above":
      return { x: Math.min(Math.max(box.left + box.width / 2, placed.left + 32), placed.left + card.width - 32), y: placed.top + card.height };
    case "right":
      return { x: placed.left, y: placed.top + Math.min(card.height / 2, 60) };
    default:
      return { x: placed.left + card.width, y: placed.top + Math.min(card.height / 2, 60) };
  }
}

/** Where the arrow lands: the target edge nearest the card. */
function edgePoint(box: Rect, from: { x: number; y: number }) {
  const cx = box.left + box.width / 2;
  const cy = box.top + box.height / 2;
  const dx = from.x - cx;
  const dy = from.y - cy;
  if (Math.abs(dx) * box.height > Math.abs(dy) * box.width) {
    return { x: dx > 0 ? box.left + box.width + 4 : box.left - 4, y: Math.min(Math.max(from.y, box.top + 12), box.top + box.height - 12) };
  }
  return { x: Math.min(Math.max(from.x, box.left + 12), box.left + box.width - 12), y: dy > 0 ? box.top + box.height + 4 : box.top - 4 };
}
