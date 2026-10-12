// Every change in plain words, newest first, grouped by day, with Undo
// (spec 5.7). A change that failed says what to do next.

import { CircleAlert, Copy, FolderInput, FolderOpen, FolderPlus, History, Pencil, RotateCcw, Trash, Undo2 } from "lucide-react";
import { useEffect, useState } from "react";
import { locationLine, relativeTime } from "../lib/format";
import { ipc } from "../lib/ipc";
import { ShowMore } from "../components/ShowMore";
import { SkeletonList } from "../components/Skeleton";
import { activityFirstPage, groupByDay, PAGE, takeRows, useShown } from "../lib/paging";
import { batchTitle, foldBatches, type ActivityEntry } from "../lib/organize";
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
  const props = { size: 20, "aria-hidden": true };
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

  // Today and Yesterday first (10 to 30 rows), then older on request.
  const entries = foldBatches(ops ?? []);
  const days = groupByDay(entries, (e) => e.createdAt);
  const first = activityFirstPage(days);
  const [shownRaw, showOlder] = useShown(first, PAGE.activity * 2, ops === null ? "loading" : "loaded");
  const shown = Math.max(shownRaw, first);
  const visibleDays = takeRows(days, shown);

  const undoAll = async (batch: string) => {
    try {
      const result = await ipc.undoBatch(batch);
      toast(result.failed.length === 0 ? "Everything is back where it was." : `Put back ${result.undone}; ${result.failed.length} could not be moved back.`, result.failed.length === 0 ? "info" : "error");
      await load();
      await refreshFind();
    } catch (error) {
      fail(error);
    }
  };

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
      <div className="view-head">
        <h1 className="view-title">Activity</h1>
        <p className="view-lead">Everything Desk changed, and everything you changed through it. Undo puts it back.</p>
      </div>
      {ops === null ? (
        <SkeletonList label="Loading activity" rows={4} />
      ) : ops.length === 0 ? (
        <div className="card empty">
          <span className="empty-mark" aria-hidden>
            <History size={26} />
          </span>
          <strong>Nothing yet</strong>
          When a download is filed or you move a file, it shows up here.
        </div>
      ) : (
        <>
        {visibleDays.map((group) => (
          <section key={group.title} className="activity-group" aria-label={group.title}>
            <h2 className="activity-group-title">{group.title}</h2>
            <div className="rows">
              {group.items.map((entry: ActivityEntry) => {
                if (entry.type === "batch") {
                  const live = entry.ops.some((o) => o.status === "done");
                  return (
                    <div key={entry.batch} className={`activity-row${live ? "" : " is-undone"}`} data-testid="activity-batch">
                      <span className="op-mark">
                        <FolderInput size={20} aria-hidden />
                      </span>
                      <div className="activity-text">
                        <strong>{batchTitle(entry.ops)}</strong>
                        <span>
                          from the Desktop and Downloads {"\u00b7"} {relativeTime(entry.createdAt)}
                        </span>
                      </div>
                      {live ? (
                        <button type="button" className="button" onClick={() => void undoAll(entry.batch)} data-testid="undo-batch">
                          <Undo2 size={18} aria-hidden />
                          Undo all
                        </button>
                      ) : (
                        <span className="activity-state">
                          <RotateCcw size={16} aria-hidden />
                          Undone
                        </span>
                      )}
                    </div>
                  );
                }
                const op = entry.op;
                const { title, detail } = describeOp(op, info.roots);
                return (
                  <div key={op.id} className={`activity-row${op.status === "failed" ? " is-failed" : ""}${op.status === "undone" ? " is-undone" : ""}`} data-testid="activity-row">
                    <span className="op-mark">
                      <OpIcon op={op} />
                    </span>
                    <div className="activity-text">
                      <strong>{title}</strong>
                      <span>
                        {detail.replace(/\.$/, "")}
                        {detail ? " \u00b7 " : ""}
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
                        <RotateCcw size={16} aria-hidden />
                        Undone
                      </span>
                    ) : (
                      <span className="activity-actions">
                        <span className="activity-state">Not done</span>
                        <button type="button" className="button" onClick={() => void ipc.reveal(op.src).catch(fail)} data-testid="failed-reveal">
                          <FolderOpen size={18} aria-hidden />
                          Show the file
                        </button>
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          </section>
        ))}
        <ShowMore shown={Math.min(shown, entries.length)} total={entries.length} step={PAGE.activity * 2} onMore={showOlder} testId="activity-more" />
        </>
      )}
    </div>
  );
}
