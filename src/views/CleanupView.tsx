// Clean up (spec 5.5): duplicate copies, Desktop clutter, old downloads.
// Nothing happens until she clicks; no destructive box starts ticked. A file
// that is an extra copy is only offered for the Recycle Bin, never also for a
// move, so she is never asked two opposite things about the same file.

import { CheckCheck, ChevronRight, CircleCheck, FolderTree, Trash } from "lucide-react";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Dialog } from "../components/Dialog";
import { FolderPicker } from "../components/FolderPicker";
import { OrganizeDialog } from "../components/OrganizeDialog";
import { planSummary } from "../lib/organize";
import { KindTile } from "../components/icons";
import { ShowMore } from "../components/ShowMore";
import { SkeletonList } from "../components/Skeleton";
import { OPEN_CARD_MAX, PAGE, useShown } from "../lib/paging";
import { locationLine, monthName, relativeTime } from "../lib/format";
import { ipc } from "../lib/ipc";
import type { AppInfo, CleanupReport, OrganizePlan, Suggestion } from "../lib/types";
import { moveOpIds, useApp } from "../store/app";

function dirLabel(dir: string, roots: string[]): string {
  return locationLine(dir, roots).replace(/^in /, "");
}

/**
 * One Clean up section, a row of the shared card. A short section starts
 * open; a long one starts folded with its count, and opens to 10 rows at a
 * time (spec 5.10). The whole title line opens and closes it; the chevron
 * sits at the right so the title and its description share one left edge.
 * `children` gets how many rows to draw.
 */
function FoldSection({ id, title, lead, count, action, testId, children }: { id: string; title: string; lead: string; count: number; action?: ReactNode; testId: string; children(shown: number): ReactNode }) {
  const [open, setOpen] = useState(count <= OPEN_CARD_MAX);
  const [shown, more] = useShown(PAGE.cleanup, PAGE.cleanup);
  return (
    <section className={`fold${open ? " is-open" : ""}`} aria-labelledby={`${id}-title`} data-testid={testId}>
      <h2 className="fold-title" id={`${id}-title`}>
        <button type="button" className="fold-toggle" aria-expanded={open} aria-controls={`${id}-body`} onClick={() => setOpen((v) => !v)} data-testid={`${testId}-toggle`}>
          <span className="fold-name">{title}</span>
          <span className="count-pill section-count">{count.toLocaleString()}</span>
          <ChevronRight className="fold-chevron" size={20} aria-hidden />
        </button>
      </h2>
      <div className="fold-sub">
        <p>{lead}</p>
        {open && action}
      </div>
      {open && (
        <div id={`${id}-body`} className="fold-body">
          {children(shown)}
          <ShowMore shown={Math.min(shown, count)} total={count} step={PAGE.cleanup} onMore={more} testId={`${testId}-more`} />
        </div>
      )}
    </section>
  );
}

export function CleanupView({ info }: { info: AppInfo }) {
  const [report, setReport] = useState<CleanupReport | null>(null);
  const [ticked, setTicked] = useState<string[]>([]);
  const [skipped, setSkipped] = useState<string[]>([]);
  const [picking, setPicking] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [plan, setPlan] = useState<OrganizePlan | null>(null);
  const [organizing, setOrganizing] = useState<"closed" | "open" | "busy">("closed");
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);

  const load = useCallback(async () => {
    try {
      const [nextReport, nextPlan] = await Promise.all([ipc.cleanup(), ipc.organizePlan()]);
      setReport(nextReport);
      setPlan(nextPlan);
    } catch (error) {
      fail(error);
    }
  }, [fail]);

  // Organize now (spec 5.11): the plan is worked out again on the native side
  // when it runs; only the confirmed file list travels.
  const organize = async () => {
    if (!plan) return;
    setOrganizing("busy");
    try {
      const result = await ipc.organizeRun(plan.moves.map((m) => m.path));
      setOrganizing("closed");
      if (result.moved > 0) toast(`Organized ${planSummary(result.moved, result.folders)}.`, "info", undefined, result.batch);
      if (result.failed.length > 0) {
        const names = result.failed.slice(0, 3).join(", ");
        toast(`${result.failed.length === 1 ? "1 file was" : `${result.failed.length} files were`} left where ${result.failed.length === 1 ? "it was" : "they were"}: ${names}${result.failed.length > 3 ? " and more" : ""}. It may be open in another program.`, "error");
      }
    } catch (error) {
      setOrganizing("closed");
      fail(error);
    }
    await load();
  };

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
      {items.map((s) => (
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
  const notSkipped = (items: Suggestion[]) => items.filter((s) => !skipped.includes(s.file.path));
  const desktop = notSkipped(notCopies(report?.desktop ?? []));
  const oldDownloads = notSkipped(notCopies(report?.oldDownloads ?? []));
  const moveAllLabel = (n: number) => (n === 1 ? "Move it as suggested" : `Move all ${n.toLocaleString()} as suggested`);
  const loose = plan?.moves.length ?? 0;
  const tidy = (report?.duplicates.length ?? 0) === 0 && desktop.length === 0 && oldDownloads.length === 0 && loose === 0;

  return (
    <div className="main-inner" data-testid="cleanup-view">
      <div className="view-head">
        <h1 className="view-title">Clean up</h1>
        <p className="view-lead">Things Tote noticed. Nothing moves until you click, and everything can be undone.</p>
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
          {plan && loose > 0 && (
            <section className="card organize-card" aria-labelledby="organize-title" data-testid="organize-card">
              <div>
                <h2 className="card-title" id="organize-title">
                  Organize the Desktop and Downloads
                </h2>
                <p>
                  Sorts {loose === 1 ? "1 loose file" : `${loose.toLocaleString()} loose files`} into folders by grade and subject, guessed from each name. You see the plan first, and one Undo puts everything back.
                </p>
              </div>
              <button type="button" className="button button-primary" onClick={() => setOrganizing("open")} data-testid="organize-open">
                <FolderTree size={18} aria-hidden />
                {loose === 1 ? "Organize 1 file..." : `Organize ${loose.toLocaleString()} files...`}
              </button>
            </section>
          )}
          {(report.duplicates.length > 0 || desktop.length > 0 || oldDownloads.length > 0) && (
            <div className="card fold-group" data-testid="tidy-card">
              {report.duplicates.length > 0 && (
                <FoldSection
                  id="dupes"
                  title="Duplicate copies"
                  lead="The newest copy is marked Latest. Tick the extra copies you no longer need."
                  count={report.duplicates.length}
                  testId="dupes-card"
                  action={
                    <button type="button" className="button button-danger" disabled={ticked.length === 0} onClick={() => setConfirm(true)} data-testid="recycle-ticked">
                      <Trash size={18} aria-hidden />
                      {ticked.length === 0 ? "Recycle ticked copies" : `Recycle ${ticked.length} ${ticked.length === 1 ? "copy" : "copies"}`}
                    </button>
                  }
                >
                  {(shown) =>
                    report.duplicates.slice(0, shown).map((group) => (
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
                    ))
                  }
                </FoldSection>
              )}
              {desktop.length > 0 && (
                <FoldSection
                  id="desktop"
                  title="Loose files on the Desktop"
                  lead="Each one has a suggested home in School Files."
                  count={desktop.length}
                  testId="desktop-card"
                  action={
                    <button type="button" className="button button-primary" onClick={() => void acceptAll(desktop)} data-testid="desktop-move-all">
                      {moveAllLabel(desktop.length)}
                    </button>
                  }
                >
                  {(shown) => <SuggestionList items={desktop.slice(0, shown)} testId="desktop-list" />}
                </FoldSection>
              )}
              {oldDownloads.length > 0 && (
                <FoldSection id="old" title="Old downloads" lead="Untouched for two months or more." count={oldDownloads.length} testId="old-card">
                  {(shown) => <SuggestionList items={oldDownloads.slice(0, shown)} testId="old-downloads-list" />}
                </FoldSection>
              )}
            </div>
          )}
        </div>
      )}
      {plan && organizing !== "closed" && (
        <OrganizeDialog plan={plan} schoolFiles={info.schoolFiles} busy={organizing === "busy"} onConfirm={() => void organize()} onClose={() => setOrganizing("closed")} />
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
