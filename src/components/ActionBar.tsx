// The bar that appears when files are ticked (spec 5.4): Move to, Rename,
// Pin, Recycle Bin. Only the Recycle Bin asks first.

import { FolderInput, Pencil, Pin, Trash, X } from "lucide-react";
import { useState } from "react";
import { ipc } from "../lib/ipc";
import { useApp } from "../store/app";
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
  const [picking, setPicking] = useState(false);
  const [confirmRecycle, setConfirmRecycle] = useState(false);
  if (selected.length === 0) return null;

  const rows = allRows(result).filter((r) => selected.includes(r.path));
  const anyOld = rows.some((r) => !r.downloadedAt || (Date.now() / 1000 - r.downloadedAt) > 7 * 86_400);

  const move = async (destination: string) => {
    setPicking(false);
    try {
      const moved = await ipc.moveFiles(selected, destination);
      clear();
      await refresh();
      toast(moved.length === 1 ? "Moved. Undo is in Activity." : `Moved ${moved.length} files. Undo is in Activity.`);
    } catch (error) {
      fail(error);
      await refresh();
    }
  };

  const pin = async () => {
    try {
      for (const path of selected) await ipc.setPinned(path, true);
      clear();
      await refresh();
      toast("Pinned to the top of Find.");
    } catch (error) {
      fail(error);
    }
  };

  const recycle = async () => {
    setConfirmRecycle(false);
    try {
      const ops = await ipc.recycleFiles(selected);
      clear();
      await refresh();
      const failed = ops.filter((o) => o.status === "failed");
      if (failed.length > 0) toast(failed[0].message ?? "Some files could not be moved to the Recycle Bin.", "error");
      else toast(ops.length === 1 ? "Moved to the Recycle Bin. Undo is in Activity." : `${ops.length} files moved to the Recycle Bin. Undo is in Activity.`);
    } catch (error) {
      fail(error);
      await refresh();
    }
  };

  return (
    <>
      <div className="action-bar" role="toolbar" aria-label="Selected files" data-testid="action-bar">
        <span className="action-bar-count">{selected.length === 1 ? "1 file" : `${selected.length} files`}</span>
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
        <button type="button" className="button button-danger" onClick={() => setConfirmRecycle(true)}>
          <Trash size={18} aria-hidden />
          Recycle Bin
        </button>
        <button type="button" className="icon-button" onClick={clear} aria-label="Clear selection">
          <X size={20} aria-hidden />
        </button>
      </div>
      {picking && <FolderPicker count={selected.length} warnOld={anyOld} onPick={(d) => void move(d)} onClose={() => setPicking(false)} />}
      {confirmRecycle && (
        <Dialog title={selected.length === 1 ? "Move this file to the Recycle Bin?" : `Move ${selected.length} files to the Recycle Bin?`} onClose={() => setConfirmRecycle(false)} testId="recycle-confirm">
          <p>Nothing is deleted for good. You can bring it back from Activity or from the Recycle Bin.</p>
          <div className="dialog-actions">
            <button type="button" className="button" onClick={() => setConfirmRecycle(false)}>
              Keep it
            </button>
            <button type="button" className="button button-danger" onClick={() => void recycle()}>
              Move to Recycle Bin
            </button>
          </div>
        </Dialog>
      )}
    </>
  );
}
