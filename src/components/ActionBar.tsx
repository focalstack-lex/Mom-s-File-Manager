// The bar that appears when files are ticked (spec 5.4): Move to, Rename,
// Pin, Recycle Bin. Only the Recycle Bin asks first, and it names the files.
// Every finished change offers Undo right in its toast. From a ticked row,
// Shift+F10 (or the menu key) jumps to the bar, the Windows key for "actions
// on what is selected", and the count says so to a screen reader. After a
// change, focus goes to the file in its new place or to the next row, never
// back to the page.

import { FolderInput, Pencil, Pin, Trash, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { recycleFailureText } from "../lib/errors";
import { focusSoon, neighbourRows, rowTarget } from "../lib/focus";
import { locationLine } from "../lib/format";
import { ipc } from "../lib/ipc";
import { movedOps, stampNote, useApp } from "../store/app";
import { allRows, useFind } from "../store/find";
import { Dialog } from "./Dialog";
import { FolderPicker } from "./FolderPicker";

export function ActionBar({ onRename }: { onRename(path: string): void }) {
  const selected = useFind((s) => s.selected);
  const result = useFind((s) => s.result);
  const clear = useFind((s) => s.clearSelection);
  const refresh = useFind((s) => s.refresh);
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const roots = useApp((s) => s.info?.roots ?? []);
  const [picking, setPicking] = useState(false);
  const [confirmRecycle, setConfirmRecycle] = useState(false);
  const barRef = useRef<HTMLDivElement>(null);
  const ticked = selected.length > 0;

  useEffect(() => {
    if (!ticked) return;
    const onKey = (event: KeyboardEvent) => {
      const wants = (event.key === "F10" && event.shiftKey) || event.key === "ContextMenu";
      if (!wants || document.querySelector('[role="dialog"]')) return;
      event.preventDefault();
      barRef.current?.querySelector<HTMLElement>("button")?.focus();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ticked]);

  if (!ticked) return null;

  // A pinned file can also be a recent one; list each path once.
  const rows = [...new Map(allRows(result).filter((r) => selected.includes(r.path)).map((r) => [r.path, r])).values()];
  const anyOld = rows.some((r) => !r.downloadedAt || (Date.now() / 1000 - r.downloadedAt) > 7 * 86_400);

  const move = async (destination: string) => {
    setPicking(false);
    const nextRows = neighbourRows(selected);
    try {
      const moved = await ipc.moveFiles(selected, destination);
      clear();
      await refresh();
      // The toast names the folder the way Find does ("School Files > Word > October 2026").
      const where = locationLine(destination, roots).replace(/^in /, "");
      // A file from the web that lost its mark on a flash drive says so (audit T-43).
      const ops = await movedOps(moved);
      toast(`${moved.length === 1 ? `Moved to ${where}.` : `Moved ${moved.length} files to ${where}.`}${stampNote(ops)}`, "info", ops.map((op) => op.id));
      // A recent download stays listed in its new place; focus follows it there.
      focusSoon(moved[0] ? rowTarget(moved[0]) : null, ...nextRows);
    } catch (error) {
      fail(error);
      await refresh();
      focusSoon(rowTarget(selected[0]), ...nextRows);
    }
  };

  const pin = async () => {
    const first = selected[0];
    try {
      for (const path of selected) await ipc.setPinned(path, true);
      clear();
      await refresh();
      toast("Pinned to the top of Find.");
      focusSoon(rowTarget(first));
    } catch (error) {
      fail(error);
    }
  };

  const recycle = async () => {
    setConfirmRecycle(false);
    const nextRows = neighbourRows(selected);
    try {
      const ops = await ipc.recycleFiles(selected);
      clear();
      await refresh();
      // A refused or lost file comes back as a failed row with its reason;
      // the files that did go can still come back from the toast's Undo.
      const trouble = recycleFailureText(ops);
      if (trouble) toast(trouble, "error", ops.filter((o) => o.status === "done").map((o) => o.id));
      else toast(ops.length === 1 ? "Moved to the Recycle Bin." : `${ops.length} files moved to the Recycle Bin.`, "info", ops.map((o) => o.id));
      // The files are gone from the list: the next row takes the focus, or a refused file that stayed.
      focusSoon(...nextRows, ...ops.filter((o) => o.status === "failed").map((o) => rowTarget(o.src)));
    } catch (error) {
      fail(error);
      await refresh();
      focusSoon(rowTarget(selected[0]), ...nextRows);
    }
  };

  const clearAll = () => {
    const first = selected[0];
    clear();
    focusSoon(rowTarget(first));
  };

  return (
    <>
      <div className="action-bar" role="toolbar" aria-label="Selected files" data-testid="action-bar" ref={barRef}>
        <span className="action-bar-count" aria-live="polite" data-testid="action-bar-count">
          {selected.length === 1 ? "1 file" : `${selected.length} files`}
          <span className="visually-hidden"> selected. Shift+F10 goes to {selected.length === 1 ? "Move to, Rename, Pin" : "Move to, Pin"} and Recycle Bin.</span>
        </span>
        <span className="action-bar-divider" aria-hidden />
        <button type="button" className="button" onClick={() => setPicking(true)}>
          <FolderInput size={18} aria-hidden />
          Move to...
        </button>
        {selected.length === 1 && (
          <button type="button" className="button" onClick={() => onRename(selected[0])}>
            <Pencil size={18} aria-hidden />
            Rename
          </button>
        )}
        <button type="button" className="button" onClick={() => void pin()}>
          <Pin size={18} aria-hidden />
          Pin
        </button>
        <button type="button" className="button" onClick={() => setConfirmRecycle(true)}>
          <Trash size={18} aria-hidden />
          Recycle Bin
        </button>
        <span className="action-bar-divider" aria-hidden />
        <button type="button" className="icon-button" onClick={clearAll} aria-label="Clear selection" title="Clear selection">
          <X size={20} aria-hidden />
        </button>
      </div>
      {picking && <FolderPicker count={selected.length} warnOld={anyOld} onPick={(d) => void move(d)} onClose={() => setPicking(false)} />}
      {confirmRecycle && (
        <Dialog title={selected.length === 1 ? "Move this file to the Recycle Bin?" : `Move ${selected.length} files to the Recycle Bin?`} onClose={() => setConfirmRecycle(false)} testId="recycle-confirm">
          <ul className="dialog-files" aria-label="Files">
            {rows.slice(0, 5).map((r) => (
              <li key={r.path}>{r.name}</li>
            ))}
            {rows.length > 5 && <li className="more">and {rows.length - 5} more</li>}
          </ul>
          <p>
            {selected.length === 1 ? "A file on this computer goes" : "Files on this computer go"} to the Recycle Bin, so you can bring {selected.length === 1 ? "it" : "them"} back from Activity or from the Recycle Bin. A file on a flash drive or a network folder stays where it is.
          </p>
          <div className="dialog-actions">
            <button type="button" className="button" onClick={() => setConfirmRecycle(false)}>
              {selected.length === 1 ? "Keep it" : "Keep them"}
            </button>
            <button type="button" className="button" onClick={() => void recycle()}>
              Move to Recycle Bin
            </button>
          </div>
        </Dialog>
      )}
    </>
  );
}
