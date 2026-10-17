// Every change in plain words, newest first, grouped by day, with Undo
// (spec 5.7). Several files sent at once are one row with Undo all. A file
// that was filed or moved can be shown in its folder; a change that failed
// says what to do next.

import { CircleAlert, Copy, FolderInput, FolderOpen, FolderPlus, History, Pencil, RotateCcw, Trash, Undo2 } from "lucide-react";
import { useEffect, useState } from "react";
import { describeError, plainMessage, undoText } from "../lib/errors";
import { locationLine, relativeTime } from "../lib/format";
import { ipc } from "../lib/ipc";
import { LoadFailed } from "../components/LoadFailed";
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

/** "from the Desktop" when every file left one folder, else "from several folders". */
function batchDetail(ops: OpRow[], roots: string[]): string {
  if ((ops[0]?.batch ?? "").startsWith("organize-")) return "from the Desktop and Downloads";
  const origins = new Set(ops.map((o) => folderOf(o.src, roots)));
  const [only] = origins;
  return origins.size === 1 && only ? `from ${only.replace(/^in /, "")}` : "from several folders";
}

/** Changes whose file now sits somewhere she may want to open. */
function showable(op: OpRow): boolean {
  return op.status === "done" && op.dst !== null && (op.kind === "autoSort" || op.kind === "move" || op.kind === "rename" || op.kind === "copy");
}

/** The sentence ops.rs records when Windows deleted a file instead of recycling it. */
const REMOVED_BY_WINDOWS = "Windows removed it without putting it in the Recycle Bin, so it cannot be brought back.";

/** A failed change whose file is still where it was, so File Explorer can show it. */
export function fileStillThere(op: OpRow): boolean {
  return !(op.kind === "recycle" && op.message === REMOVED_BY_WINDOWS);
}

/** The next step when a failed row carries no reason: the file never moved. */
const STAYED = "It is still where it was. Try again from Find.";

/**
 * A failed change says "Could not ..." with the reason the native side gave
 * and what to do next; a success says what happened. Program text never
 * passes through: a reason that is not plain words is replaced.
 */
export function describeOp(op: OpRow, roots: string[]): { title: string; detail: string } {
  const name = nameOf(op.kind === "newFolder" ? op.src : op.dst ?? op.src);
  const failed = op.status === "failed";
  switch (op.kind) {
    case "autoSort":
      return failed
        ? { title: `${nameOf(op.src)} was left in Downloads`, detail: plainMessage(op.message, "It is still in Downloads.") }
        : { title: `Filed ${name}`, detail: `${folderOf(op.dst, roots)}, automatically` };
    case "move":
      return failed
        ? { title: `Could not move ${nameOf(op.src)}`, detail: plainMessage(op.message, STAYED) }
        : { title: `Moved ${name}`, detail: `${folderOf(op.src, roots)} to ${folderOf(op.dst, roots).replace(/^in /, "")}` };
    case "rename":
      return failed
        ? { title: `Could not rename ${nameOf(op.src)}`, detail: plainMessage(op.message, STAYED) }
        : { title: `Renamed ${nameOf(op.src)}`, detail: `now ${name}` };
    case "newFolder":
      return failed
        ? { title: `Could not make the folder ${name}`, detail: plainMessage(op.message, "Try again from Move to.") }
        : { title: `New folder ${name}`, detail: folderOf(op.src, roots) };
    case "recycle":
      return failed
        ? { title: `Could not send ${nameOf(op.src)} to the Recycle Bin`, detail: plainMessage(op.message, "Windows could not move it to the Recycle Bin. Try again, or delete it in File Explorer if you are sure.") }
        : { title: `Moved ${nameOf(op.src)} to the Recycle Bin`, detail: folderOf(op.src, roots) };
    case "copy":
      return failed
        ? { title: `Could not make a copy of ${nameOf(op.src)}`, detail: plainMessage(op.message, "The template was not touched. Try again from Find.") }
        : { title: `New copy ${name}`, detail: `from template ${nameOf(op.src)}` };
  }
}


/** Opens File Explorer with the file selected, where the change left it. */
function ShowButton({ op, onError }: { op: OpRow; onError(error: unknown): void }) {
  const name = nameOf(op.dst);
  return (
    <button type="button" className="icon-button activity-show" onClick={() => void ipc.reveal(op.dst ?? op.src).catch(onError)} aria-label={`Show ${name} in its folder`} title="Show in folder" data-testid="activity-show">
      <FolderOpen size={20} aria-hidden />
    </button>
  );
}

/** What a failed row offers: Not done, and File Explorer on the file while it is still there. */
function FailedActions({ op, onError }: { op: OpRow; onError(error: unknown): void }) {
  return (
    <span className="activity-actions">
      <span className="activity-state">Not done</span>
      {fileStillThere(op) && (
        <button type="button" className="button button-subtle activity-undo" onClick={() => void ipc.reveal(op.src).catch(onError)} data-testid="failed-reveal">
          <FolderOpen size={18} aria-hidden />
          Show the file
        </button>
      )}
    </span>
  );
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
  const [loadError, setLoadError] = useState<string | null>(null);
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const refreshFind = useFind((s) => s.refresh);

  const load = async () => {
    try {
      setOps(await ipc.activity());
      setLoadError(null);
    } catch (error) {
      // The sentence sits where the list would be, with Try again; never a skeleton for good.
      console.warn("Tote:", error);
      setLoadError(describeError(error));
    }
  };

  useEffect(() => {
    void load();
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
      // The same words as the toast Undo: what went back, and the first file that did not.
      toast(undoText(result.undone, result.failed.map((name) => ({ name, reason: null }))), result.failed.length === 0 ? "info" : "error");
      await load();
      await refreshFind();
    } catch (error) {
      fail(error);
    }
  };

  const undo = async (op: OpRow) => {
    try {
      await ipc.undo(op.id);
      toast(undoText(1, []));
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
        <p className="view-lead">Everything Tote changed, and everything you changed through it. Undo puts it back.</p>
      </div>
      {loadError && <LoadFailed text={loadError} onRetry={() => void load()} testId="activity-load-failed" />}
      {ops === null ? (
        !loadError && <SkeletonList label="Loading activity" rows={4} />
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
                  const recycled = entry.ops.every((o) => o.kind === "recycle");
                  const firstShown = entry.ops.find(showable);
                  // The files the batch could not take are listed under it, each with its reason.
                  const stayed = entry.ops.filter((o) => o.status === "failed");
                  const undone = !live && stayed.length < entry.ops.length;
                  return (
                    <div key={entry.batch} className="activity-batch" data-testid="activity-batch">
                      <div className={`activity-row${undone ? " is-undone" : ""}${stayed.length > 0 ? " has-failed" : ""}`}>
                        <span className={`op-mark${live && !recycled ? " is-filed" : ""}`}>{recycled ? <Trash size={20} aria-hidden /> : <FolderInput size={20} aria-hidden />}</span>
                        <div className="activity-text">
                          <strong>{batchTitle(entry.ops)}</strong>
                          <span>
                            {batchDetail(entry.ops, info.roots)} {"\u00b7"} {relativeTime(entry.createdAt)}
                          </span>
                        </div>
                        {live ? (
                          <span className="activity-actions">
                            {firstShown && <ShowButton op={firstShown} onError={fail} />}
                            <button type="button" className="button button-subtle activity-undo" onClick={() => void undoAll(entry.batch)} data-testid="undo-batch">
                              <Undo2 size={18} aria-hidden />
                              Undo all
                            </button>
                          </span>
                        ) : undone ? (
                          <span className="activity-state">
                            <RotateCcw size={16} aria-hidden />
                            Undone
                          </span>
                        ) : (
                          <span className="activity-state">Not done</span>
                        )}
                      </div>
                      {stayed.map((op) => {
                        const { title, detail } = describeOp(op, info.roots);
                        return (
                          <div key={op.id} className="activity-row is-failed activity-batch-row" data-testid="activity-batch-failed">
                            <span className="op-mark">
                              <OpIcon op={op} />
                            </span>
                            <div className="activity-text">
                              <strong>{title}</strong>
                              <span>{detail}</span>
                            </div>
                            <FailedActions op={op} onError={fail} />
                          </div>
                        );
                      })}
                    </div>
                  );
                }
                const op = entry.op;
                const { title, detail } = describeOp(op, info.roots);
                return (
                  <div key={op.id} className={`activity-row${op.status === "failed" ? " is-failed" : ""}${op.status === "undone" ? " is-undone" : ""}`} data-testid="activity-row">
                    <span className={`op-mark${op.status === "done" && (op.kind === "autoSort" || op.kind === "move") ? " is-filed" : ""}`}>
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
                      <span className="activity-actions">
                        {showable(op) && <ShowButton op={op} onError={fail} />}
                        <button type="button" className="button button-subtle activity-undo" onClick={() => void undo(op)} data-testid="undo">
                          <Undo2 size={18} aria-hidden />
                          Undo
                        </button>
                      </span>
                    ) : op.status === "undone" ? (
                      <span className="activity-state">
                        <RotateCcw size={16} aria-hidden />
                        Undone
                      </span>
                    ) : (
                      <FailedActions op={op} onError={fail} />
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
