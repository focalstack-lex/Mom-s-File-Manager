// A small modal: Escape closes, focus starts inside and stays inside (Tab
// wraps), the backdrop closes, and focus returns where it was.

import { useEffect, useRef, type ReactNode } from "react";
import { trapTab } from "../lib/focus";

interface Props {
  title: string;
  children: ReactNode;
  onClose(): void;
  testId?: string;
}

export function Dialog({ title, children, onClose, testId }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    ref.current?.querySelector<HTMLElement>("button, input, [tabindex]")?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
      else if (event.key === "Tab" && ref.current) trapTab(event, ref.current);
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      previous?.focus();
    };
  }, [onClose]);

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title" ref={ref} data-testid={testId}>
        <h2 id="dialog-title">{title}</h2>
        {children}
      </div>
    </div>
  );
}
