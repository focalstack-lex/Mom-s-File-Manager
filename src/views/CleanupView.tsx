// Clean up (spec 5.5): duplicate copies, Desktop clutter, old downloads.
// Nothing happens until she clicks; no destructive box starts ticked.

import { Trash } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Dialog } from "../components/Dialog";
import { FolderPicker } from "../components/FolderPicker";
import { KindIcon } from "../components/icons";
import { locationLine, monthName, relativeTime } from "../lib/format";
import { ipc } from "../lib/ipc";
import type { AppInfo, CleanupReport, Suggestion } from "../lib/types";
import { useApp } from "../store/app";

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

  const act = async (work: () => Promise<void>, done: string) => {
    try {
      await work();
      toast(done);
      await load();
    } catch (error) {
      fail(error);
      await load();
    }
  };

  const accept = (s: Suggestion) => act(async () => void (await ipc.moveFiles([s.file.path], s.suggestedDir)), `Moved to ${dirLabel(s.suggestedDir, info.roots)}. Undo is in Activity.`);
  const moveTo = (path: string, dir: string) => {
    setPicking(null);
    return act(async () => void (await ipc.moveFiles([path], dir)), "Moved. Undo is in Activity.");
  };
  const recycleTicked = () => {
    setConfirm(false);
    const count = ticked.length;
    return act(async () => {
      await ipc.recycleFiles(ticked);
      setTicked([]);
    }, count === 1 ? "1 copy moved to the Recycle Bin. Undo is in Activity." : `${count} copies moved to the Recycle Bin. Undo is in Activity.`);
  };
  const acceptAll = (items: Suggestion[]) => {
    const live = items.filter((s) => !skipped.includes(s.file.path));
    return act(async () => {
      for (const s of live) await ipc.moveFiles([s.file.path], s.suggestedDir);
    }, `Moved ${live.length} files into School Files. Undo is in Activity.`);
  };

  const SuggestionList = ({ items, testId }: { items: Suggestion[]; testId: string }) => (
    <div data-testid={testId}>
      {items
        .filter((s) => !skipped.includes(s.file.path))
        .map((s) => (
          <div key={s.file.path} className="suggestion">
            <KindIcon kind={s.file.kind} />
            <div className="suggestion-text">
              <strong>{s.file.name}</strong>
              <span>
                edited {relativeTime(s.file.modified)} · would go to {dirLabel(s.suggestedDir, info.roots).split(" > ").map(monthName).join(" > ")}
              </span>
            </div>
            <div className="suggestion-actions">
              <button type="button" className="button button-primary" onClick={() => void accept(s)}>
                Move it there
              </button>
              <button type="button" className="button" onClick={() => setPicking(s.file.path)}>
                Somewhere else
              </button>
              <button type="button" className="button button-quiet" onClick={() => setSkipped((list) => [...list, s.file.path])}>
                Skip
              </button>
            </div>
          </div>
        ))}
    </div>
  );

  const tidy = (report?.duplicates.length ?? 0) === 0 && (report?.desktop.length ?? 0) === 0 && (report?.oldDownloads.length ?? 0) === 0;

  return (
    <div className="main-inner" data-testid="cleanup-view">
      <div>
        <h1 className="view-title">Clean up</h1>
        <p className="view-lead">Things Desk noticed. Nothing moves until you click, and everything can be undone.</p>
      </div>
      {report === null ? (
        <p className="status-line">Looking around...</p>
      ) : tidy ? (
        <div className="card empty">
          <strong>All tidy</strong>
          No duplicate copies, nothing loose on the Desktop, no old downloads.
        </div>
      ) : (
        <div className="cleanup-cards">
          {report.duplicates.length > 0 && (
            <section className="card" aria-labelledby="dupes-title">
              <div className="cleanup-head">
                <div>
                  <h2 id="dupes-title">Duplicate copies</h2>
                  <p>The newest copy is marked Latest. Tick the extra copies you no longer need.</p>
                </div>
                <button type="button" className="button button-danger" disabled={ticked.length === 0} onClick={() => setConfirm(true)} data-testid="recycle-ticked">
                  <Trash size={18} aria-hidden />
                  {ticked.length === 0 ? "Recycle ticked copies" : `Recycle ${ticked.length} ${ticked.length === 1 ? "copy" : "copies"}`}
                </button>
              </div>
              {report.duplicates.map((group) => (
                <div key={group.latest.groupKey} style={{ marginBottom: "var(--space-3)" }}>
                  <div className="suggestion">
                    <KindIcon kind={group.latest.kind} />
                    <div className="suggestion-text">
                      <strong>
                        {group.latest.name} <span className="latest">Latest</span>
                      </strong>
                      <span>
                        edited {relativeTime(group.latest.modified)} · {locationLine(group.latest.parent, info.roots)}
                      </span>
                    </div>
                    <span className="activity-state">keep</span>
                  </div>
                  {group.others.map((copy) => (
                    <div key={copy.path} className="suggestion" style={{ paddingLeft: "var(--space-5)" }}>
                      <label className="row-check">
                        <input type="checkbox" checked={ticked.includes(copy.path)} onChange={() => setTicked((list) => (list.includes(copy.path) ? list.filter((p) => p !== copy.path) : [...list, copy.path]))} aria-label={`Recycle ${copy.name}`} />
                      </label>
                      <div className="suggestion-text">
                        <strong>{copy.name}</strong>
                        <span>
                          edited {relativeTime(copy.modified)} · {locationLine(copy.parent, info.roots)}
                        </span>
                      </div>
                      <button type="button" className="button button-quiet" onClick={() => void ipc.open(copy.path)}>
                        Open
                      </button>
                    </div>
                  ))}
                </div>
              ))}
            </section>
          )}
          {report.desktop.length > 0 && (
            <section className="card" aria-labelledby="desktop-title">
              <div className="cleanup-head">
                <div>
                  <h2 id="desktop-title">Loose files on the Desktop</h2>
                  <p>Each one has a suggested home in School Files.</p>
                </div>
                <button type="button" className="button" onClick={() => void acceptAll(report.desktop)}>
                  Move all as suggested
                </button>
              </div>
              <SuggestionList items={report.desktop} testId="desktop-list" />
            </section>
          )}
          {report.oldDownloads.length > 0 && (
            <section className="card" aria-labelledby="old-title">
              <div className="cleanup-head">
                <div>
                  <h2 id="old-title">Old downloads</h2>
                  <p>Untouched for two months or more.</p>
                </div>
              </div>
              <SuggestionList items={report.oldDownloads} testId="old-downloads-list" />
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
