// Settings (spec 5.8): the few switches that exist, the school-year dates
// that unlock two chips (picked from a calendar, stored as YYYY-MM-DD),
// templates, and the walkthrough replay.

import { CircleAlert, CircleCheck } from "lucide-react";
import { useState } from "react";
import { SubjectRules } from "../components/SubjectRules";
import { browserLabel } from "../lib/browsers";
import { ipc } from "../lib/ipc";
import type { AppInfo, DateRange, Settings } from "../lib/types";
import { useApp } from "../store/app";
import { useFind } from "../store/find";
import { useTour } from "../store/tour";

const QUARTERS = ["Quarter 1", "Quarter 2", "Quarter 3", "Quarter 4"];

export function SettingsView({ info }: { info: AppInfo }) {
  const save = useApp((s) => s.saveSettings);
  const setView = useApp((s) => s.setView);
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const startTour = useTour((s) => s.start);
  const templates = useFind((s) => s.templates);
  const loadTemplates = useFind((s) => s.loadTemplates);
  const loadInfo = useApp((s) => s.loadInfo);
  const browsers = useApp((s) => s.browsers);
  const checkBrowsers = useApp((s) => s.checkBrowsers);
  const [draft, setDraft] = useState<Settings>(info.settings);
  const needsRestart = useApp((s) => s.foldersChanged);
  const setNeedsRestart = useApp((s) => s.markFoldersChanged);

  const update = (patch: Partial<Settings>) => {
    const next = { ...draft, ...patch };
    setDraft(next);
    void save(next);
  };

  const addFolder = async () => {
    try {
      const picked = await ipc.pickFolder();
      if (!picked) return;
      await ipc.addFolder(picked);
      await loadInfo();
      setNeedsRestart();
      toast("Tote will look in that folder after it restarts.");
    } catch (error) {
      fail(error);
    }
  };

  const removeFolder = async (path: string) => {
    try {
      await ipc.removeFolder(path);
      await loadInfo();
      setNeedsRestart();
      toast("Tote will stop looking in that folder after it restarts.");
    } catch (error) {
      fail(error);
    }
  };

  const quarter = (i: number): DateRange => draft.quarters[i] ?? { name: QUARTERS[i], start: "", end: "" };
  const setQuarter = (i: number, patch: Partial<DateRange>) => {
    const quarters = QUARTERS.map((_, j) => (j === i ? { ...quarter(j), ...patch } : quarter(j)));
    update({ quarters });
  };

  const addTemplate = async () => {
    try {
      const picked = await ipc.pickFile();
      if (!picked) return;
      await ipc.addTemplate(picked);
      await loadTemplates();
      toast("Added to the template shelf.");
    } catch (error) {
      fail(error);
    }
  };

  return (
    <div className="main-inner" data-testid="settings-view">
      <div className="view-head">
        <h1 className="view-title">Settings</h1>
        <p className="view-lead">Only the few things worth deciding. Changes save as you make them.</p>
      </div>

      <section className="card settings-list" aria-labelledby="general-title">
        <h2 className="card-title" id="general-title">
          General
        </h2>
        <label className="toggle">
          <span className="toggle-text">
            Start when the computer starts
            <span>So downloads are filed even before Tote is opened.</span>
          </span>
          <input type="checkbox" role="switch" className="switch" checked={draft.startAtLogin} onChange={(e) => update({ startAtLogin: e.target.checked })} />
        </label>
        <label className="toggle">
          <span className="toggle-text">
            Tell me when a download is filed
            <span>A small Windows message in the corner.</span>
          </span>
          <input type="checkbox" role="switch" className="switch" checked={draft.notifications} onChange={(e) => update({ notifications: e.target.checked })} />
        </label>
        <label className="toggle">
          <span className="toggle-text">
            Check for new versions
            <span>Once a day Tote asks codewithlex.site if a newer version is out. Your files never leave this laptop.</span>
          </span>
          <input type="checkbox" role="switch" className="switch" checked={draft.checkUpdates} onChange={(e) => update({ checkUpdates: e.target.checked })} data-testid="check-updates" />
        </label>
        <div className="toggle">
          <span className="toggle-text">
            Walkthrough
            <span>Five short steps that show how Tote works.</span>
          </span>
          <button
            type="button"
            className="button"
            onClick={() => {
              setView("find");
              startTour();
            }}
            data-testid="replay-tour"
          >
            Show me again
          </button>
        </div>
      </section>

      <section className="card settings-list" aria-labelledby="browsers-title" data-testid="browsers-card">
        <h2 className="card-title" id="browsers-title">
          Browsers
        </h2>
        <p className="field-note">
          Tote files what lands in Downloads. A browser that asks where to save, or saves somewhere else, means those files stay where they were saved. Tote still lists them under Recent downloads.
        </p>
        {browsers === null ? (
          <p className="field-note">Checking...</p>
        ) : browsers.length === 0 ? (
          <p className="field-note">No Chrome, Edge, Brave or Firefox settings were found on this computer.</p>
        ) : (
          browsers.map((b) => (
            <div key={`${b.browser}-${b.profile}`} className={`browser-row${b.ok ? "" : " needs-change"}`} data-testid="browser-row">
              {b.ok ? <CircleCheck size={22} aria-hidden /> : <CircleAlert size={22} aria-hidden />}
              <span className="toggle-text">
                {browserLabel(b, browsers)}
                <span>{b.advice}</span>
              </span>
            </div>
          ))
        )}
        <button type="button" className="button align-start" onClick={() => void checkBrowsers()} data-testid="browsers-recheck">
          Check again
        </button>
      </section>

      <section className="card settings-list" aria-labelledby="year-title">
        <h2 className="card-title" id="year-title">
          School year
        </h2>
        <p className="field-note">Set these once and the Find screen gets &quot;This quarter&quot; and &quot;School year&quot; filters.</p>
        <div className="date-grid" role="group" aria-label="School year and quarters">
          <span className="date-grid-head" aria-hidden />
          <span className="date-grid-head" aria-hidden>
            Starts
          </span>
          <span className="date-grid-head" aria-hidden>
            Ends
          </span>
          <span className="date-grid-label" id="sy-label">
            School year
          </span>
          <input id="sy-start" type="date" aria-label="School year starts" value={draft.schoolYear?.start ?? ""} max={draft.schoolYear?.end || undefined} onChange={(e) => update({ schoolYear: { name: "School year", start: e.target.value, end: draft.schoolYear?.end ?? "" } })} />
          <input id="sy-end" type="date" aria-label="School year ends" value={draft.schoolYear?.end ?? ""} min={draft.schoolYear?.start || undefined} onChange={(e) => update({ schoolYear: { name: "School year", start: draft.schoolYear?.start ?? "", end: e.target.value } })} />
          <span className="date-grid-divider" aria-hidden />
          {QUARTERS.map((name, i) => (
            <QuarterRow key={name} index={i} range={quarter(i)} min={draft.schoolYear?.start} max={draft.schoolYear?.end} onChange={(patch) => setQuarter(i, patch)} />
          ))}
        </div>
      </section>

      <section className="card settings-list" aria-labelledby="organize-title">
        <h2 className="card-title" id="organize-title">
          Organize: subject folders
        </h2>
        <p className="field-note">
          Clean up &gt; Organize puts a file in a subject folder when one of these words is in its name. The first match wins. Grades are read from the name on their own (Grade 7, G7, Baitang 7, Kinder).
        </p>
        <SubjectRules saved={draft.subjectRules ?? []} onSave={(subjectRules) => update({ subjectRules })} onError={fail} />
      </section>

      <section className="card settings-list" aria-labelledby="templates-title">
        <h2 className="card-title" id="templates-title">
          Templates
        </h2>
        <p className="field-note">Master files that are copied, never edited. Clicking one on the Find screen makes a dated copy.</p>
        {templates.map((t) => (
          <div key={t.id} className="toggle" data-testid="template-row">
            <span className="toggle-text">
              {t.name}
              <span title={t.path}>{t.missing ? "The file is missing" : `in ${t.path.split("\\").slice(-2, -1)[0] ?? "its folder"}`}</span>
            </span>
            <button type="button" className="button button-subtle" onClick={() => void ipc.removeTemplate(t.id).then(loadTemplates)}>
              Remove
            </button>
          </div>
        ))}
        <button type="button" className="button align-start" onClick={() => void addTemplate()} data-testid="add-template">
          Choose a file to add
        </button>
      </section>

      <section className="card settings-list" aria-labelledby="folders-title">
        <h2 className="card-title" id="folders-title">
          Folders Tote looks in
        </h2>
        {info.roots.map((root) => (
          <div key={root} className="toggle" data-testid="root-row">
            <span className="toggle-text" title={root}>
              {root.split("\\").pop()}
              <span>{root.split("\\").slice(0, -1).join("\\")}</span>
            </span>
            {info.removableRoots.includes(root) ? (
              <button type="button" className="button button-subtle" onClick={() => void removeFolder(root)}>
                Remove
              </button>
            ) : (
              <span className="field-note">Always included</span>
            )}
          </div>
        ))}
        <button type="button" className="button align-start" onClick={() => void addFolder()} data-testid="add-folder">
          Add a folder
        </button>
        {needsRestart && (
          <div className="toggle" data-testid="restart-note">
            <span className="toggle-text">
              Folder changes take effect after a restart
              <span>Tote keeps filing downloads meanwhile.</span>
            </span>
            <button type="button" className="button button-primary" onClick={() => void ipc.restart().catch(fail)}>
              Restart Tote
            </button>
          </div>
        )}
        <p className="field-note">Downloads are filed into {info.schoolFiles}.{info.testMode ? " Test mode: these are the TOTE_ROOTS folders." : ""}</p>
      </section>
      <p className="view-foot">Tote {info.version}</p>
    </div>
  );
}

/** One quarter: an editable name, then its two dates, kept inside the school year. */
function QuarterRow({ index, range, min, max, onChange }: { index: number; range: DateRange; min?: string; max?: string; onChange(patch: Partial<DateRange>): void }) {
  const label = range.name || QUARTERS[index];
  return (
    <>
      <input id={`q${index}-name`} className="date-grid-name" aria-label={`Name for ${QUARTERS[index]}`} value={range.name} onChange={(e) => onChange({ name: e.target.value })} />
      <input id={`q${index}-start`} type="date" aria-label={`${label} starts`} value={range.start} min={min || undefined} max={range.end || max || undefined} onChange={(e) => onChange({ start: e.target.value })} />
      <input id={`q${index}-end`} type="date" aria-label={`${label} ends`} value={range.end} min={range.start || min || undefined} max={max || undefined} onChange={(e) => onChange({ end: e.target.value })} />
    </>
  );
}
