// Every change in plain words, newest first, with Undo (spec 5.7).

import { CircleAlert, Copy, FolderInput, FolderPlus, Pencil, RotateCcw, Trash, Undo2 } from "lucide-react";
import { useEffect, useState } from "react";
import { locationLine, relativeTime } from "../lib/format";
import { ipc } from "../lib/ipc";
import type { AppInfo, OpRow } from "../lib/types";
import { useApp } from "../store/app";
import { useFind } from "../store/find";

function nameOf(path: string | null): string {
  return path?.split("\\").pop() ?? "";
}

function folderOf(path: string | null, roots: string[]): string {
  if (!path) return "";
  return locationLine(path.slice(0, path.lastIndexOf("\\")), roots);
}

export function describeOp(op: OpRow, roots: string[]): { title: string; detail: string } {
  const name = nameOf(op.kind === "newFolder" ? op.src : op.dst ?? op.src);
  switch (op.kind) {
    case "autoSort":
      return op.status === "failed"
        ? { title: `${nameOf(op.src)} was left in Downloads`, detail: op.message ?? "" }
        : { title: `Filed ${name}`, detail: `${folderOf(op.dst, roots)}, automatically` };
    case "move":
      return { title: `Moved ${name}`, detail: `${folderOf(op.src, roots)} to ${folderOf(op.dst, roots).replace(/^in /, "")}` };
    case "rename":
      return { title: `Renamed ${nameOf(op.src)}`, detail: `now ${name}` };
    case "newFolder":
      return { title: `New folder ${name}`, detail: folderOf(op.src, roots) };
    case "recycle":
      return { title: `Moved ${nameOf(op.src)} to the Recycle Bin`, detail: folderOf(op.src, roots) };
    case "copy":
      return { title: `New copy ${name}`, detail: `from template ${nameOf(op.src)}` };
  }
}

function OpIcon({ op }: { op: OpRow }) {
  const props = { size: 22, "aria-hidden": true };
  if (op.status === "failed") return <CircleAlert {...props} />;
  switch (op.kind) {
    case "rename":
      return <Pencil {...props} />;
    case "newFolder":
      return <FolderPlus {...props} />;
    case "recycle":
      return <Trash {...props} />;
    case "copy":
      return <Copy {...props} />;
    default:
      return <FolderInput {...props} />;
  }
}

export function ActivityView({ info }: { info: AppInfo }) {
  const [ops, setOps] = useState<OpRow[] | null>(null);
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const refreshFind = useFind((s) => s.refresh);

  const load = async () => {
    try {
      setOps(await ipc.activity());
    } catch (error) {
      fail(error);
    }
  };

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const undo = async (op: OpRow) => {
    try {
      await ipc.undo(op.id);
      toast("Put back where it was.");
      await load();
      await refreshFind();
    } catch (error) {
      fail(error);
    }
  };

  return (
    <div className="main-inner" data-testid="activity-view">
      <div>
        <h1 className="view-title">Activity</h1>
        <p className="view-lead">Everything Desk changed, and everything you changed through it. Undo puts it back.</p>
      </div>
      <div className="card">
        {ops === null ? (
          <p className="status-line">Loading...</p>
        ) : ops.length === 0 ? (
          <div className="empty">
            <strong>Nothing yet</strong>
            When a download is filed or you move a file, it shows up here.
          </div>
        ) : (
          ops.map((op) => {
            const { title, detail } = describeOp(op, info.roots);
            return (
              <div key={op.id} className={`activity-row${op.status === "failed" ? " is-failed" : ""}`} data-testid="activity-row">
                <OpIcon op={op} />
                <div className="activity-text">
                  <strong>{title}</strong>
                  <span>
                    {detail}
                    {detail ? " · " : ""}
                    {relativeTime(op.createdAt)}
                  </span>
                </div>
                {op.status === "done" ? (
                  <button type="button" className="button" onClick={() => void undo(op)} data-testid="undo">
                    <Undo2 size={18} aria-hidden />
                    Undo
                  </button>
                ) : op.status === "undone" ? (
                  <span className="activity-state">
                    <RotateCcw size={16} aria-hidden style={{ verticalAlign: "-3px", marginRight: 4 }} />
                    Undone
                  </span>
                ) : (
                  <span className="activity-state">Not done</span>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
