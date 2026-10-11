// Clean up (spec 5.5): duplicate copies, Desktop clutter, old downloads.
// Nothing happens until she clicks; no destructive box starts ticked. A file
// that is an extra copy is only offered for the Recycle Bin, never also for a
// move, so she is never asked two opposite things about the same file.

import { CheckCheck, CircleCheck, Trash } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Dialog } from "../components/Dialog";
import { FolderPicker } from "../components/FolderPicker";
import { KindTile } from "../components/icons";
import { SkeletonList } from "../components/Skeleton";
import { locationLine, monthName, relativeTime } from "../lib/format";
import { ipc } from "../lib/ipc";
import type { AppInfo, CleanupReport, Suggestion } from "../lib/types";
import { moveOpIds, useApp } from "../store/app";

function dirLabel(dir: string, roots: string[]): string {
  return locationLine(dir, roots).replace(/^in /, "");
}

export function CleanupView({ info }: { info: AppInfo }) {
  const [report, setReport] = useState<CleanupReport | null>(null);
  const [ticked, setTicked] = useState<string[]>([]);
  const [skipped, setSkipped] = useState<string[]>([]);
  const [picking, setPicking] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);

  const load = useCallback(async () => {
    try {
      setReport(await ipc.cleanup());
    } catch (error) {
      fail(error);
    }
  }, [fail]);

  useEffect(() => {
    void load();
  }, [load]);

  const act = async (work: () => Promise<number[] | void>, done: string) => {
    try {
      const undoIds = await work();
      toast(done, "info", undoIds ?? undefined);
      await load();
    } catch (error) {
      fail(error);
      await load();
    }
  };

  const accept = (s: Suggestion) => act(async () => moveOpIds(await ipc.moveFiles([s.file.path], s.suggestedDir)), `Moved to ${dirLabel(s.suggestedDir, info.roots)}.`);
  const moveTo = (path: string, dir: string) => {
    setPicking(null);
    return act(async () => moveOpIds(await ipc.moveFiles([path], dir)), "Moved.");
  };
  const recycleTicked = () => {
    setConfirm(false);
    const count = ticked.length;
    return act(async () => {
      const ops = await ipc.recycleFiles(ticked);
      setTicked([]);
      return ops.map((o) => o.id);
    }, count === 1 ? "1 copy moved to the Recycle Bin." : `${count} copies moved to the Recycle Bin.`);
  };
  const acceptAll = (items: Suggestion[]) => {
    const live = items.filter((s) => !skipped.includes(s.file.path));
    return act(async () => {
      const moved: string[] = [];
      for (const s of live) moved.push(...(await ipc.moveFiles([s.file.path], s.suggestedDir)));
      return moveOpIds(moved);
    }, `Moved ${live.length} files into School Files.`);
  };

  const SuggestionList = ({ items, testId }: { items: Suggestion[]; testId: string }) => (
    <div className="rows" data-testid={testId}>
      {items
        .filter((s) => !skipped.includes(s.file.path))
        .map((s) => (
          <div key={s.file.path} className="suggestion">
            <KindTile kind={s.file.kind} ext={s.file.ext} />
            <div className="suggestion-text">
              <strong>{s.file.name}</strong>
              <span>
                edited {relativeTime(s.file.modified)} · would go to {dirLabel(s.suggestedDir, info.roots).split(" > ").map(monthName).join(" > ")}
              </span>
            </div>
            <div className="suggestion-actions">
              <button type="button" className="button" onClick={() => void accept(s)}>
                Move it there
              </button>
              <button type="button" className="button button-subtle" onClick={() => setPicking(s.file.path)}>
                Somewhere else
              </button>
              <button type="button" className="button button-subtle" onClick={() => setSkipped((list) => [...list, s.file.path])}>
                Skip
              </button>
            </div>
          </div>
        ))}
    </div>
  );

  // Extra copies belong to the Recycle Bin question only.
  const extraCopies = new Set(report?.duplicates.flatMap((g) => g.others.map((o) => o.path)) ?? []);
  const notCopies = (items: Suggestion[]) => items.filter((s) => !extraCopies.has(s.file.path));
  const desktop = notCopies(report?.desktop ?? []);
  const oldDownloads = notCopies(report?.oldDownloads ?? []);
  const tidy = (report?.duplicates.length ?? 0) === 0 && desktop.length === 0 && oldDownloads.length === 0;

  return (
    <div className="main-inner" data-testid="cleanup-view">
      <div className="view-head">
        <h1 className="view-title">Clean up</h1>
        <p className="view-lead">Things Desk noticed. Nothing moves until you click, and everything can be undone.</p>
      </div>
      {report === null ? (
        <SkeletonList label="Looking around" />
      ) : tidy ? (
        <div className="card empty" data-testid="cleanup-tidy">
          <span className="empty-mark is-good" aria-hidden>
            <CheckCheck size={28} />
          </span>
          <strong>All tidy</strong>
          No duplicate copies, nothing loose on the Desktop, no old downloads.
        </div>
      ) : (
        <div className="cleanup-cards">
          {report.duplicates.length > 0 && (
            <section className="card section" aria-labelledby="dupes-title">
              <div className="cleanup-head">
                <div>
                  <h2 className="card-title" id="dupes-title">
                    Duplicate copies
                  </h2>
                  <p>The newest copy is marked Latest. Tick the extra copies you no longer need.</p>
                </div>
                <button type="button" className="button button-danger" disabled={ticked.length === 0} onClick={() => setConfirm(true)} data-testid="recycle-ticked">
                  <Trash size={18} aria-hidden />
                  {ticked.length === 0 ? "Recycle ticked copies" : `Recycle ${ticked.length} ${ticked.length === 1 ? "copy" : "copies"}`}
                </button>
              </div>
              {report.duplicates.map((group) => (
                <div key={group.latest.groupKey} className="dupe-group">
                  <div className="suggestion">
                    <KindTile kind={group.latest.kind} ext={group.latest.ext} />
                    <div className="suggestion-text">
                      <strong>
                        <span>{group.latest.name}</span> <span className="latest">Latest</span>
                      </strong>
                      <span>
                        edited {relativeTime(group.latest.modified)} · {locationLine(group.latest.parent, info.roots)}
                      </span>
                    </div>
                    <span className="keep-mark">
                      <CircleCheck size={18} aria-hidden />
                      Keeping
                    </span>
                  </div>
                  {group.others.map((copy) => (
                    <div key={copy.path} className="suggestion is-copy">
                      <label className="row-check">
                        <input type="checkbox" checked={ticked.includes(copy.path)} onChange={() => setTicked((list) => (list.includes(copy.path) ? list.filter((p) => p !== copy.path) : [...list, copy.path]))} aria-label={`Recycle ${copy.name}`} />
                      </label>
                      <div className="suggestion-text">
                        <strong>{copy.name}</strong>
                        <span>
                          edited {relativeTime(copy.modified)} · {locationLine(copy.parent, info.roots)}
                        </span>
                      </div>
                      <button type="button" className="button button-subtle" onClick={() => void ipc.open(copy.path).catch(fail)}>
                        Open
                      </button>
                    </div>
                  ))}
                </div>
              ))}
            </section>
          )}
          {desktop.length > 0 && (
            <section className="card section" aria-labelledby="desktop-title">
              <div className="cleanup-head">
                <div>
                  <h2 className="card-title" id="desktop-title">
                    Loose files on the Desktop
                  </h2>
                  <p>Each one has a suggested home in School Files.</p>
                </div>
                <button type="button" className="button button-primary" onClick={() => void acceptAll(desktop)}>
                  Move all as suggested
                </button>
              </div>
              <SuggestionList items={desktop} testId="desktop-list" />
            </section>
          )}
          {oldDownloads.length > 0 && (
            <section className="card section" aria-labelledby="old-title">
              <div className="cleanup-head">
                <div>
                  <h2 className="card-title" id="old-title">
                    Old downloads
                  </h2>
                  <p>Untouched for two months or more.</p>
                </div>
              </div>
              <SuggestionList items={oldDownloads} testId="old-downloads-list" />
            </section>
          )}
        </div>
      )}
      {picking && <FolderPicker count={1} warnOld onPick={(dir) => void moveTo(picking, dir)} onClose={() => setPicking(null)} />}
      {confirm && (
        <Dialog title={ticked.length === 1 ? "Move 1 copy to the Recycle Bin?" : `Move ${ticked.length} copies to the Recycle Bin?`} onClose={() => setConfirm(false)}>
          <p>The Latest copy stays. Nothing is deleted for good.</p>
          <div className="dialog-actions">
            <button type="button" className="button" onClick={() => setConfirm(false)}>
              Keep them
            </button>
            <button type="button" className="button button-danger" onClick={() => void recycleTicked()}>
              Move to Recycle Bin
            </button>
          </div>
        </Dialog>
      )}
    </div>
  );
}
