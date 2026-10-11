// Settings (spec 5.8): the few switches that exist, the school-year dates
// that unlock two chips, templates, and the walkthrough replay.

import { useState } from "react";
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
      toast("Desk will look in that folder after it restarts.");
    } catch (error) {
      fail(error);
    }
  };

  const removeFolder = async (path: string) => {
    try {
      await ipc.removeFolder(path);
      await loadInfo();
      setNeedsRestart();
      toast("Desk will stop looking in that folder after it restarts.");
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
      <div>
        <h1 className="view-title">Settings</h1>
        <p className="view-lead">Desk {info.version}. Only the few things worth deciding.</p>
      </div>

      <section className="card settings-list" aria-labelledby="general-title">
        <h2 className="card-title" id="general-title">
          General
        </h2>
        <label className="toggle">
          <span className="toggle-text">
            Start when the computer starts
            <span>So downloads are filed even before Desk is opened.</span>
          </span>
          <input type="checkbox" checked={draft.startAtLogin} onChange={(e) => update({ startAtLogin: e.target.checked })} />
        </label>
        <label className="toggle">
          <span className="toggle-text">
            Tell me when a download is filed
            <span>A small Windows message in the corner.</span>
          </span>
          <input type="checkbox" checked={draft.notifications} onChange={(e) => update({ notifications: e.target.checked })} />
        </label>
        <div className="toggle">
          <span className="toggle-text">
            Walkthrough
            <span>Five short steps that show how Desk works.</span>
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

      <section className="card settings-list" aria-labelledby="year-title">
        <h2 className="card-title" id="year-title">
          School year
        </h2>
        <p className="field-note">Set these once and the Find screen gets "This quarter" and "School year" filters. Dates are YYYY-MM-DD.</p>
        <div className="range-grid">
          <div className="field">
            <label htmlFor="sy-start">School year starts</label>
            <input id="sy-start" placeholder="2026-07-28" value={draft.schoolYear?.start ?? ""} onChange={(e) => update({ schoolYear: { name: "School year", start: e.target.value, end: draft.schoolYear?.end ?? "" } })} />
          </div>
          <div className="field">
            <label htmlFor="sy-end">Ends</label>
            <input id="sy-end" placeholder="2027-04-16" value={draft.schoolYear?.end ?? ""} onChange={(e) => update({ schoolYear: { name: "School year", start: draft.schoolYear?.start ?? "", end: e.target.value } })} />
          </div>
        </div>
        {QUARTERS.map((name, i) => (
          <div className="range-grid" key={name}>
            <div className="field">
              <label htmlFor={`q${i}-name`}>Name</label>
              <input id={`q${i}-name`} value={quarter(i).name} onChange={(e) => setQuarter(i, { name: e.target.value })} />
            </div>
            <div className="field">
              <label htmlFor={`q${i}-start`}>Starts</label>
              <input id={`q${i}-start`} placeholder="YYYY-MM-DD" value={quarter(i).start} onChange={(e) => setQuarter(i, { start: e.target.value })} />
            </div>
            <div className="field">
              <label htmlFor={`q${i}-end`}>Ends</label>
              <input id={`q${i}-end`} placeholder="YYYY-MM-DD" value={quarter(i).end} onChange={(e) => setQuarter(i, { end: e.target.value })} />
            </div>
          </div>
        ))}
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
              <span>{t.missing ? "The file is missing" : t.path}</span>
            </span>
            <button type="button" className="button button-quiet" onClick={() => void ipc.removeTemplate(t.id).then(loadTemplates)}>
              Remove
            </button>
          </div>
        ))}
        <button type="button" className="button" onClick={() => void addTemplate()} style={{ alignSelf: "flex-start" }} data-testid="add-template">
          Choose a file to add
        </button>
      </section>

      <section className="card settings-list" aria-labelledby="folders-title">
        <h2 className="card-title" id="folders-title">
          Folders Desk looks in
        </h2>
        {info.roots.map((root) => (
          <div key={root} className="toggle" data-testid="root-row">
            <span className="toggle-text">
              {root.split("\\").pop()}
              <span>{root}</span>
            </span>
            {info.removableRoots.includes(root) ? (
              <button type="button" className="button button-quiet" onClick={() => void removeFolder(root)}>
                Remove
              </button>
            ) : (
              <span className="field-note">always</span>
            )}
          </div>
        ))}
        <button type="button" className="button" onClick={() => void addFolder()} style={{ alignSelf: "flex-start" }} data-testid="add-folder">
          Add a folder
        </button>
        {needsRestart && (
          <div className="toggle" data-testid="restart-note">
            <span className="toggle-text">
              Folder changes take effect after a restart
              <span>Desk keeps filing downloads meanwhile.</span>
            </span>
            <button type="button" className="button button-primary" onClick={() => void ipc.restart().catch(fail)}>
              Restart Desk
            </button>
          </div>
        )}
        <p className="field-note">Downloads are filed into {info.schoolFiles}.{info.testMode ? " Test mode: these are the DESK_ROOTS folders." : ""}</p>
      </section>
    </div>
  );
}
