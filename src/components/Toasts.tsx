// Short confirmations in the corner. Errors stay a little longer; a change
// that can be reversed carries its own Undo button.

import { CircleAlert, CircleCheck, Undo2, X } from "lucide-react";
import { ipc } from "../lib/ipc";
import { useApp, type Toast } from "../store/app";
import { useFind } from "../store/find";

export function Toasts() {
  const toasts = useApp((s) => s.toasts);
  const dismiss = useApp((s) => s.dismissToast);
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const refreshFind = useFind((s) => s.refresh);

  const undo = async (item: Toast) => {
    dismiss(item.id);
    try {
      // Newest first, so a chain of moves unwinds in the right order.
      if (item.undoBatch) await ipc.undoBatch(item.undoBatch);
      for (const id of [...(item.undoIds ?? [])].reverse()) await ipc.undo(id);
      await refreshFind();
      toast("Put back where it was.");
    } catch (error) {
      fail(error);
    }
  };

  if (toasts.length === 0) return null;
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((item) => (
        <div key={item.id} className={`toast${item.kind === "error" ? " toast-error" : ""}`} role={item.kind === "error" ? "alert" : "status"}>
          {item.kind === "error" ? <CircleAlert size={22} aria-hidden /> : <CircleCheck size={22} aria-hidden />}
          <span className="toast-text">{item.text}</span>
          {((item.undoIds && item.undoIds.length > 0) || item.undoBatch) && (
            <button type="button" className="button button-quiet" onClick={() => void undo(item)} data-testid="toast-undo">
              <Undo2 size={18} aria-hidden />
              Undo
            </button>
          )}
          <button type="button" className="icon-button" onClick={() => dismiss(item.id)} aria-label="Dismiss">
            <X size={18} aria-hidden />
          </button>
        </div>
      ))}
    </div>
  );
}
