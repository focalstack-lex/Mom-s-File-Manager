// Short confirmations in the corner. Errors stay a little longer.

import { CircleAlert, CircleCheck } from "lucide-react";
import { useApp } from "../store/app";

export function Toasts() {
  const toasts = useApp((s) => s.toasts);
  const dismiss = useApp((s) => s.dismissToast);
  if (toasts.length === 0) return null;
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast${toast.kind === "error" ? " toast-error" : ""}`} role={toast.kind === "error" ? "alert" : "status"}>
          {toast.kind === "error" ? <CircleAlert size={22} aria-hidden /> : <CircleCheck size={22} aria-hidden style={{ color: "var(--accent)", flex: "none" }} />}
          <span>{toast.text}</span>
          <button type="button" className="button button-quiet" onClick={() => dismiss(toast.id)}>
            OK
          </button>
        </div>
      ))}
    </div>
  );
}
