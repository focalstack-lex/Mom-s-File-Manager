// The first-run walkthrough (spec 5.9). The page dims except for the control
// being explained; a card with an arrow drawn to that control says what it
// does. Enter is Next, Escape is Skip.

import { ArrowLeft, ArrowRight } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { placeCard, TOUR_STEPS, useTour } from "../store/tour";

interface Rect {
  top: number;
  left: number;
  width: number;
  height: number;
}

const PAD = 8;

export function Tour({ onFinish }: { onFinish(): void }) {
  const active = useTour((s) => s.active);
  const index = useTour((s) => s.index);
  const next = useTour((s) => s.next);
  const back = useTour((s) => s.back);
  const finish = useTour((s) => s.finish);
  const [target, setTarget] = useState<Rect | null>(null);
  const [card, setCard] = useState({ width: 360, height: 200 });
  const [viewport, setViewport] = useState({ width: window.innerWidth, height: window.innerHeight });
  const cardRef = useRef<HTMLDivElement>(null);
  const step = TOUR_STEPS[index];

  useLayoutEffect(() => {
    if (!active) return;
    const measure = () => {
      const el = document.querySelector<HTMLElement>(`[data-tour="${step.target}"]`);
      if (el) {
        el.scrollIntoView({ block: "nearest", inline: "nearest" });
        const r = el.getBoundingClientRect();
        setTarget({ top: r.top - PAD, left: r.left - PAD, width: r.width + PAD * 2, height: r.height + PAD * 2 });
      } else {
        setTarget(null);
      }
      setViewport({ width: window.innerWidth, height: window.innerHeight });
      if (cardRef.current) setCard({ width: cardRef.current.offsetWidth, height: cardRef.current.offsetHeight });
    };
    measure();
    const raf = window.requestAnimationFrame(measure);
    window.addEventListener("resize", measure);
    return () => {
      window.cancelAnimationFrame(raf);
      window.removeEventListener("resize", measure);
    };
  }, [active, step]);

  useEffect(() => {
    if (!active) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        finish();
        onFinish();
      } else if (event.key === "Enter") {
        event.preventDefault();
        if (index >= TOUR_STEPS.length - 1) onFinish();
        next();
      }
    };
    window.addEventListener("keydown", onKey);
    cardRef.current?.querySelector<HTMLElement>("button.button-primary")?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, [active, index, next, finish, onFinish]);

  if (!active) return null;

  const fallback: Rect = { top: viewport.height / 2 - 40, left: viewport.width / 2 - 160, width: 320, height: 80 };
  const box = target ?? fallback;
  const placed = placeCard(box, card, viewport);
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
