// The one date field Tote uses (Lex's reference, 2026-10-17), in place of
// the browser's own date input: the field says the day in words ("Sat 17
// Oct 2026"), and a click opens a month of large round days under it. One
// click on a day picks it and closes; Today and No date sit at the foot.
// Days of other months are left out, so every number on the grid can be
// picked. Keys: arrows by a day or a week, Home and End to the week's ends,
// Page Up and Page Down by a month, Enter picks, Escape closes; Tab stays
// inside while it is open, and focus goes back to the field when it closes.
// The value is "YYYY-MM-DD" text, as the native field gave, or null.

import { CalendarDays, ChevronDown, ChevronLeft, ChevronRight } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { addMonths, allowed, canTurn, clampDay, dayLabel, fieldWords, keyMove, monthTitle, monthWeeks, today, WEEKDAYS, WEEKDAYS_SHORT } from "../lib/calendar";
import { parseDay } from "../lib/dates";
import { trapTab } from "../lib/focus";

interface Props {
  /** "YYYY-MM-DD", or null or "" for no day. */
  value: string | null;
  onChange(day: string | null): void;
  /** What the day is for, as a screen reader says it ("Due date", "School year starts"). */
  label: string;
  id?: string;
  min?: string;
  max?: string;
  /** Offers No date at the foot when a day is set. */
  clearable?: boolean;
  placeholder?: string;
  disabled?: boolean;
  /** For narrow columns (Settings): "5 Oct 2026", no weekday and no chevron. */
  compact?: boolean;
  testId?: string;
}

/** Space between the field and the calendar, and the least room kept from the window's edges. */
const GAP = 6;
const EDGE = 8;

export function DatePicker({ value, onChange, label, id, min, max, clearable = true, placeholder = "Pick a day", disabled, compact, testId }: Props) {
  const chosen = value && parseDay(value) ? value : null;
  const [open, setOpen] = useState(false);
  const [focus, setFocus] = useState<string>(() => clampDay(chosen ?? today(), min, max));
  const [place, setPlace] = useState<{ top: number; left: number } | null>(null);
  const field = useRef<HTMLButtonElement>(null);
  const pop = useRef<HTMLDivElement>(null);
  // Opening and the keys move the keyboard focus to the day; the month
  // buttons leave it on themselves, so a second click turns another month.
  const followFocus = useRef(false);
  // Once per opening, a field with no room on either side is scrolled up first.
  const scrolledFor = useRef(false);
  const now = today();

  const focusDate = parseDay(focus) ?? new Date();
  const year = focusDate.getFullYear();
  const month = focusDate.getMonth();

  const show = () => {
    setFocus(clampDay(chosen ?? now, min, max));
    followFocus.current = true;
    scrolledFor.current = false;
    setPlace(null);
    setOpen(true);
  };

  const close = (backToField: boolean) => {
    setOpen(false);
    if (backToField) field.current?.focus();
  };

  const pick = (day: string | null) => {
    onChange(day);
    close(true);
  };

  // Under the field, or above it when the window has no room below; kept
  // inside the window either way. Measured after each render while open,
  // and again when the window or anything under it scrolls or resizes.
  useLayoutEffect(() => {
    if (!open) return;
    const measure = () => {
      const box = pop.current?.getBoundingClientRect();
      let anchor = field.current?.getBoundingClientRect();
      if (!anchor || !box) return;
      const fitsBelow = (at: DOMRect) => at.bottom + GAP + box.height + EDGE <= window.innerHeight;
      const fitsAbove = (at: DOMRect) => at.top - GAP - box.height >= EDGE;
      // Neither side has room: the page scrolls the field up once, the way
      // a native list does, and the field is measured again where it went.
      if (!fitsBelow(anchor) && !fitsAbove(anchor) && !scrolledFor.current && field.current) {
        scrolledFor.current = true;
        field.current.scrollIntoView({ block: "start" });
        anchor = field.current.getBoundingClientRect();
      }
      // Still no room (a very short window): as low as it fits, never past the edge.
      const top = fitsBelow(anchor) ? anchor.bottom + GAP : fitsAbove(anchor) ? anchor.top - GAP - box.height : Math.max(EDGE, window.innerHeight - box.height - EDGE);
      const left = Math.max(EDGE, Math.min(anchor.left, window.innerWidth - box.width - EDGE));
      setPlace((was) => (was && was.top === top && was.left === left ? was : { top, left }));
    };
    measure();
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [open, focus]);

  // The focused day takes the keyboard focus whenever it changes.
  useEffect(() => {
    if (!open || !place || !followFocus.current) return;
    followFocus.current = false;
    pop.current?.querySelector<HTMLElement>(`[data-day="${focus}"]`)?.focus();
  }, [open, place, focus]);

  // A click anywhere else closes it, leaving focus where she clicked.
  useEffect(() => {
    if (!open) return;
    const away = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!pop.current?.contains(target) && !field.current?.contains(target)) close(false);
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  }, [open]);

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close(true);
      return;
    }
    if (event.key === "Tab" && pop.current) {
      trapTab(event.nativeEvent, pop.current);
      return;
    }
    const target = event.target as HTMLElement;
    if (!target.dataset.day) return;
    const next = keyMove(focus, event.key);
    if (next === null) return;
    event.preventDefault();
    followFocus.current = true;
    setFocus(clampDay(next, min, max));
  };

  // From the latest focus, so two quick clicks turn two months.
  const turn = (step: -1 | 1) => setFocus((was) => clampDay(addMonths(was, step), min, max));

  const title = monthTitle(year, month);

  return (
    <>
      <button
        type="button"
        id={id}
        ref={field}
        className={`date-field${open ? " is-open" : ""}${compact ? " is-compact" : ""}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`${label}: ${chosen ? dayLabel(chosen) : "no day picked"}`}
        disabled={disabled}
        onClick={() => (open ? close(true) : show())}
        data-testid={testId}
        data-value={chosen ?? ""}
      >
        <CalendarDays size={20} aria-hidden className="date-field-icon" />
        <span className={`date-field-text${chosen ? "" : " is-empty"}`}>{chosen ? fieldWords(chosen, compact) : placeholder}</span>
        {!compact && <ChevronDown size={18} aria-hidden className="date-field-chevron" />}
      </button>
      {open &&
        createPortal(
          <div
            ref={pop}
            className="date-pop"
            role="dialog"
            aria-label={`Choose: ${label}`}
            style={place ? { top: place.top, left: place.left } : { top: 0, left: 0, visibility: "hidden" }}
            onKeyDown={onKeyDown}
            data-testid="date-pop"
          >
            <div className="date-pop-head">
              <button type="button" className="icon-button" onClick={() => turn(-1)} disabled={!canTurn(focus, -1, min, max)} aria-label="Previous month" data-testid="date-prev">
                <ChevronLeft size={22} aria-hidden />
              </button>
              <h2 className="date-pop-title" aria-live="polite" data-testid="date-title">
                {title}
              </h2>
              <button type="button" className="icon-button" onClick={() => turn(1)} disabled={!canTurn(focus, 1, min, max)} aria-label="Next month" data-testid="date-next">
                <ChevronRight size={22} aria-hidden />
              </button>
            </div>
            <table className="date-cal" role="grid" aria-label={title}>
              <thead>
                <tr>
                  {WEEKDAYS_SHORT.map((short, i) => (
                    <th key={short} scope="col" abbr={WEEKDAYS[i]}>
                      {short}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {monthWeeks(year, month).map((week) => (
                  <tr key={week.find(Boolean) ?? "empty"}>
                    {week.map((day, i) =>
                      day === null ? (
                        <td key={`blank-${i}`} />
                      ) : (
                        <td key={day}>
                          <button
                            type="button"
                            className={`date-day${day === chosen ? " is-chosen" : ""}${day === now ? " is-today" : ""}`}
                            tabIndex={day === focus ? 0 : -1}
                            disabled={!allowed(day, min, max)}
                            aria-pressed={day === chosen}
                            aria-current={day === now ? "date" : undefined}
                            aria-label={`${dayLabel(day)}${day === now ? ", today" : ""}`}
                            onClick={() => pick(day)}
                            onFocus={() => setFocus(day)}
                            data-day={day}
                          >
                            {Number(day.slice(8))}
                          </button>
                        </td>
                      ),
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="date-pop-foot">
              {clearable && chosen ? (
                <button type="button" className="button button-subtle" onClick={() => pick(null)} data-testid="date-clear">
                  No date
                </button>
              ) : (
                <span />
              )}
              <button type="button" className="button button-quiet" onClick={() => pick(now)} disabled={!allowed(now, min, max)} data-testid="date-today">
                Today
              </button>
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}
