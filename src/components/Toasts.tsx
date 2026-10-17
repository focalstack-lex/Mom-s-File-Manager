// Short confirmations in the corner. Errors stay a little longer; a change
// that can be reversed carries its own Undo button. A toast waits while the
// pointer is over it or focus is in it. Undo tries every part and then says
// what really happened, in the same words everywhere; focus, which was on
// the Undo button, goes back to the screen.

import { CircleAlert, CircleCheck, Undo2, X } from "lucide-react";
import { describeError, undoText, type UndoFailure } from "../lib/errors";
import { refocusIfLost } from "../lib/focus";
import { ipc, logRenderer } from "../lib/ipc";
import type { OpRow } from "../lib/types";
import { useApp, type Toast } from "../store/app";
import { useFind } from "../store/find";
import { useTodo } from "../store/todo";

/** The file names behind Activity ids, read once so a failed Undo can be named; "That file" when the log cannot be read. */
async function namesFor(stuck: { id: number; reason: string }[]): Promise<UndoFailure[]> {
  let ops: OpRow[] = [];
  try {
    ops = await ipc.activity();
  } catch (error) {
    console.warn("Tote: could not read Activity for the names", error);
    logRenderer("warn", describeError(error));
  }
  return stuck.map(({ id, reason }) => {
    const op = ops.find((o) => o.id === id);
    return { name: (op?.dst ?? op?.src ?? "").split(/[\\/]/).pop() || "That file", reason };
  });
}

export function Toasts() {
  const toasts = useApp((s) => s.toasts);
  const dismiss = useApp((s) => s.dismissToast);
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const hold = useApp((s) => s.holdToast);
  const release = useApp((s) => s.releaseToast);
  const refreshFind = useFind((s) => s.refresh);

  const undo = async (item: Toast) => {
    dismiss(item.id);
    if (item.undoTask) {
      const todo = useTodo.getState();
      const ok = item.undoTask.action === "reopen" ? await todo.reopen(item.undoTask.id) : await todo.restore(item.undoTask.id);
      // The store has already said what went wrong; only a task that is back gets the good news.
      if (ok) toast("Back on your list.");
      refocusIfLost('[data-testid="todo-input"]');
      return;
    }
    let undone = 0;
    const failed: UndoFailure[] = [];
    if (item.undoBatch) {
      try {
        const result = await ipc.undoBatch(item.undoBatch);
        undone += result.undone;
        failed.push(...result.failed.map((file) => ({ name: file.name, reason: file.message })));
      } catch (error) {
        fail(error);
        return;
      }
    }
    // Newest first, so a chain of moves unwinds in the right order. A file
    // that cannot go back does not stop the others; it is named at the end.
    const stuck: { id: number; reason: string }[] = [];
    for (const id of [...(item.undoIds ?? [])].reverse()) {
      try {
        await ipc.undo(id);
        undone += 1;
      } catch (error) {
        console.warn("Tote: undo failed", error);
        logRenderer("warn", describeError(error));
        stuck.push({ id, reason: describeError(error) });
      }
    }
    if (stuck.length > 0) failed.push(...(await namesFor(stuck)));
    await refreshFind();
    toast(undoText(undone, failed), failed.length === 0 ? "info" : "error");
    refocusIfLost();
  };

  if (toasts.length === 0) return null;
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((item) => (
        <div
          key={item.id}
          className={`toast${item.kind === "error" ? " toast-error" : ""}`}
          role={item.kind === "error" ? "alert" : "status"}
          onMouseEnter={() => hold(item.id)}
          onMouseLeave={() => release(item.id)}
          onFocus={() => hold(item.id)}
          onBlur={() => release(item.id)}
          data-testid="toast"
        >
          {item.kind === "error" ? <CircleAlert size={22} aria-hidden /> : <CircleCheck size={22} aria-hidden />}
          <span className="toast-text">{item.text}</span>
          {((item.undoIds && item.undoIds.length > 0) || item.undoBatch || item.undoTask) && (
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
