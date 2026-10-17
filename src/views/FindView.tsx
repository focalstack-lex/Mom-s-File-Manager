// The one screen she opens (spec 5.3): search, pinned, templates, recent
// downloads, results grouped by copy. On a short window (a school laptop at
// 125 percent) the head, the Pinned rows and the template shelf tighten so
// the first recent download is on the first screen (styles.css).

import { FilePlus, Inbox, Info, Search, SearchX, X } from "lucide-react";
import { browserNotice } from "../lib/browsers";
import { useEffect, useRef, useState } from "react";
import { ActionBar } from "../components/ActionBar";
import { Chips } from "../components/Chips";
import { LoadFailed } from "../components/LoadFailed";
import { PreviewPanel } from "../components/PreviewPanel";
import { ResultRow } from "../components/ResultRow";
import { ShowMore } from "../components/ShowMore";
import { SkeletonList } from "../components/Skeleton";
import { revealCard } from "../lib/focus";
import { groupByDay, PAGE, takeRows, useShown } from "../lib/paging";
import { ipc } from "../lib/ipc";
import type { AppInfo, FileRow } from "../lib/types";
import { useApp } from "../store/app";
import { useFind } from "../store/find";
import { useTour } from "../store/tour";

const SAMPLE_ROWS: FileRow[] = [
  {
    id: -1,
    path: "sample-1",
    parent: "School Files\\Word\\2026-10",
    name: "Grade 7 Quiz 2.docx",
    ext: "docx",
    kind: "word",
    size: 0,
    modified: Math.floor(Date.now() / 1000) - 3600,
    originLabel: "Messenger",
    downloadedAt: Math.floor(Date.now() / 1000) - 3600,
    groupKey: "sample-1",
    snippet: null,
    pinned: false,
  },
  {
    id: -2,
    path: "sample-2",
    parent: "School Files\\PDF\\2026-10",
    name: "DepEd Order 2026-014.pdf",
    ext: "pdf",
    kind: "pdf",
    size: 0,
    modified: Math.floor(Date.now() / 1000) - 86_400,
    originLabel: "DepEd",
    downloadedAt: Math.floor(Date.now() / 1000) - 86_400,
    groupKey: "sample-2",
    snippet: null,
    pinned: false,
  },
];

export function FindView({ info }: { info: AppInfo }) {
  const query = useFind((s) => s.query);
  const setQuery = useFind((s) => s.setQuery);
  const result = useFind((s) => s.result);
  const templates = useFind((s) => s.templates);
  const refresh = useFind((s) => s.refresh);
  const loadTemplates = useFind((s) => s.loadTemplates);
  const previewPath = useFind((s) => s.previewPath);
  const searching = useFind((s) => s.isSearching());
  const clearFilters = useFind((s) => s.clearFilters);
  const loading = useFind((s) => s.loading);
  const failed = useFind((s) => s.failed);
  const tourActive = useTour((s) => s.active);
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const setView = useApp((s) => s.setView);
  const notice = browserNotice(useApp((s) => s.browsers));
  const inputRef = useRef<HTMLInputElement>(null);
  const [renaming, setRenaming] = useState<string | null>(null);

  useEffect(() => {
    void refresh();
    void loadTemplates();
    inputRef.current?.focus();
  }, [refresh, loadTemplates]);

  const roots = info.roots;
  const index = result?.index ?? info.index;
  // A week with nothing filed has nothing to report, so the line stays away.
  const status = index.scanning
    ? index.total === 0
      ? "Getting to know your files..."
      : `Getting to know your files: ${index.scanned.toLocaleString()} of ${index.total.toLocaleString()} so far.`
    : info.sortedThisWeek === 0
      ? null
      : info.sortedThisWeek === 1
        ? "1 file filed for you this week."
        : `${info.sortedThisWeek} files filed for you this week.`;

  const makeFromTemplate = async (id: number, name: string) => {
    try {
      const copy = await ipc.useTemplate(id);
      toast(`A fresh copy of ${name} is in School Files.${copy.opened ? "" : " Tote could not open it; open it from Find."}`);
      await refresh();
    } catch (error) {
      fail(error);
    }
  };

  const recent = result?.recent ?? [];
  const showSamples = tourActive && recent.length === 0;
  const totalGroups = result?.groups.length ?? 0;
  const countLabel = totalGroups === 1 ? "1 file" : `${totalGroups.toLocaleString()} files`;

  // Show a little, offer more (spec 5.10). A new search or filter starts at
  // the first page again; background refreshes keep what she opened.
  const filterKey = useFind((s) => JSON.stringify([s.query.trim(), s.dateChip, s.kinds, s.sources]));
  const [resultsShown, moreResults] = useShown(PAGE.results, PAGE.results, filterKey);
  const [recentShown, moreRecent] = useShown(PAGE.recent, PAGE.recent);
  const recentRows = showSamples ? SAMPLE_ROWS : recent;
  const recentDays = takeRows(
    groupByDay(recentRows, (f) => f.downloadedAt ?? f.modified),
    recentShown,
  );
  const visibleGroups = result?.groups.slice(0, resultsShown) ?? [];

  return (
    <div className="main-inner find-view" data-testid="find-view">
      <div className="view-head find-head">
        <h1 className="view-title">Find a file</h1>
        {status && (
          <p className="status-line" data-testid="status-line">
            {status}
          </p>
        )}
        {notice && (
          <p className="notice" data-testid="browser-notice">
            <Info size={18} aria-hidden />
            <span>{notice}</span>
            <button
              type="button"
              className="button button-quiet"
              onClick={() => {
                setView("settings");
                revealCard("browsers-card");
              }}
            >
              How to fix
            </button>
          </p>
        )}
      </div>

      <div className="search" data-tour="search">
        <Search size={26} aria-hidden />
        <input
          ref={inputRef}
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Type a name, a student, or a word inside the file"
          aria-label="Find a file"
          data-testid="search-input"
          autoComplete="off"
          spellCheck={false}
        />
        {searching && loading && <span className="spinner" role="status" aria-label="Searching" />}
        {query ? (
          <button type="button" className="icon-button" onClick={() => setQuery("")} aria-label="Clear search" title="Clear search">
            <X size={22} aria-hidden />
          </button>
        ) : (
          <span className="search-hint" aria-hidden>
            <kbd>Ctrl</kbd>
            <kbd>F</kbd>
          </span>
        )}
      </div>

      <Chips settings={info.settings} />

      <div className={`find-layout${previewPath ? " has-preview" : ""}`}>
        <div className="find-results">
          {searching ? (
            <section className="section" aria-labelledby="results-title" data-testid="results">
              <div className="section-head">
                <h2 className="card-title" id="results-title">
                  Results
                </h2>
                <span className="section-count" aria-live="polite" data-testid="result-count">
                  {result && !(loading && totalGroups === 0) ? countLabel : ""}
                </span>
              </div>
              {failed && <LoadFailed quiet text="Search is not available right now. Try again in a moment." onRetry={() => void refresh()} testId="search-failed" />}
              {loading && totalGroups === 0 && !failed ? (
                <SkeletonList label="Searching" />
              ) : totalGroups === 0 ? (
                // A broken search keeps the last answer and the quiet line; it never reads as no match.
                !failed && (
                <div className="card empty">
                  <span className="empty-mark" aria-hidden>
                    <SearchX size={26} />
                  </span>
                  <strong>Nothing matched yet</strong>
                  {index.scanning ? "Tote is still reading your files. Try again in a minute." : "Try fewer words, or a word you remember from inside the file."}
                  {!query && (
                    <button type="button" className="button" onClick={clearFilters}>
                      Clear filters
                    </button>
                  )}
                </div>
                )
              ) : (
                <>
                  <div className="rows">
                    {visibleGroups.map((group) => (
                      <ResultRow
                        key={group.latest.path}
                        file={group.latest}
                        roots={roots}
                        latest
                        others={group.others}
                        sameName={group.sameName}
                        renaming={renaming === group.latest.path}
                        onRenamed={() => setRenaming(null)}
                        onCancelRename={() => setRenaming(null)}
                      />
                    ))}
                  </div>
                  <ShowMore shown={visibleGroups.length} total={totalGroups} step={PAGE.results} onMore={moreResults} testId="results-more" />
                </>
              )}
            </section>
          ) : (
            <>
              {/* A refresh that failed after a change leaves the last lists up and says so above them; a first load that failed is worded under Recent downloads. */}
              {failed && result !== null && <LoadFailed quiet text="Tote could not refresh this list, so it may be out of date. Try again in a moment." onRetry={() => void refresh()} testId="find-refresh-failed" />}
              {result && result.pinned.length > 0 && (
                <section className="section" aria-label="Pinned" data-testid="pinned">
                  <h2 className="card-title">Pinned</h2>
                  <div className="rows">
                    {result.pinned.map((file) => (
                      <ResultRow key={file.path} file={file} roots={roots} compact renaming={renaming === file.path} onRenamed={() => setRenaming(null)} onCancelRename={() => setRenaming(null)} />
                    ))}
                  </div>
                </section>
              )}
              {result && result.orphanPins.length > 0 && (
                <section className="section" aria-label="Pinned files that moved">
                  {result.orphanPins.map((path) => (
                    <div key={path} className="card orphan">
                      <span className="orphan-text">
                        <strong>{path.split("\\").pop()}</strong>
                        <span className="field-note">Moved or deleted outside Tote.</span>
                      </span>
                      <button type="button" className="button" onClick={() => setQuery((path.split("\\").pop() ?? "").replace(/\.[^.]+$/, ""))}>
                        Search for it
                      </button>
                      <button type="button" className="button button-quiet" onClick={() => void ipc.forgetPin(path).then(() => refresh()).catch(fail)}>
                        Forget
                      </button>
                    </div>
                  ))}
                </section>
              )}
              {templates.length > 0 && (
                <section className="section shelf-section" aria-label="Templates" data-testid="templates">
                  <h2 className="card-title">New from a template</h2>
                  <div className="shelf">
                    {templates.map((t) => (
                      <button key={t.id} type="button" className="button" onClick={() => void makeFromTemplate(t.id, t.name)} disabled={t.missing} title={t.missing ? "The template file is missing" : `Make a fresh copy of ${t.name}`}>
                        <FilePlus size={18} aria-hidden />
                        {t.name}
                      </button>
                    ))}
                  </div>
                </section>
              )}
              {/* The walkthrough's "Your downloads land here" points at the first row (ResultRow tourOpen), which always fits on screen. */}
              <section className="section" aria-label="Recent downloads" data-testid="recent">
                <h2 className="card-title">Recent downloads</h2>
                {recent.length === 0 && !showSamples ? (
                  result === null && failed ? (
                    <LoadFailed text="Tote could not read its list right now. Try again in a moment." onRetry={() => void refresh()} testId="find-load-failed" />
                  ) : (
                    <div className="card empty">
                      <span className="empty-mark" aria-hidden>
                        <Inbox size={26} />
                      </span>
                      <strong>Nothing downloaded yet</strong>
                      New downloads appear here, newest first. To file one, right-click it in File Explorer and choose Move to Tote.
                    </div>
                  )
                ) : (
                  <>
                    <div className="day-groups">
                      {recentDays.map((day, d) => (
                        <div key={day.title} className="day-group" data-testid="recent-day">
                          <h3 className="day-title">{day.title}</h3>
                          <div className="rows">
                            {day.items.map((file, i) => (
                              <ResultRow
                                key={file.path}
                                file={file}
                                roots={roots}
                                sample={showSamples}
                                tourOpen={d === 0 && i === 0}
                                renaming={renaming === file.path}
                                onRenamed={() => setRenaming(null)}
                                onCancelRename={() => setRenaming(null)}
                              />
                            ))}
                          </div>
                        </div>
                      ))}
                    </div>
                    <ShowMore shown={Math.min(recentShown, recentRows.length)} total={recentRows.length} step={PAGE.recent} onMore={moreRecent} testId="recent-more" />
                  </>
                )}
              </section>
            </>
          )}
          <ActionBar onRename={(path) => setRenaming(path)} />
        </div>
        <PreviewPanel roots={roots} />
      </div>
    </div>
  );
}
