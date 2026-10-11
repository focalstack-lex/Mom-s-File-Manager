// The frame: green sidebar, one view at a time, toasts, and the walkthrough
// on first run.

import { BrushCleaning, FolderOpen, RotateCcw, Search, Settings } from "lucide-react";
import { useEffect } from "react";
import { Toasts } from "./components/Toasts";
import { Tour } from "./components/Tour";
import { CHANGED_EVENT, ipc, listen, SORTED_EVENT, type SortedEvent } from "./lib/ipc";
import { useApp, type View } from "./store/app";
import { useFind } from "./store/find";
import { useTour } from "./store/tour";
import { ActivityView } from "./views/ActivityView";
import { CleanupView } from "./views/CleanupView";
import { FindView } from "./views/FindView";
import { SettingsView } from "./views/SettingsView";

const NAV: { id: View; label: string; icon: typeof Search; tour?: string }[] = [
  { id: "find", label: "Find", icon: Search },
  { id: "cleanup", label: "Clean up", icon: BrushCleaning, tour: "nav-cleanup" },
  { id: "activity", label: "Activity", icon: RotateCcw, tour: "nav-activity" },
];

export default function App() {
  const view = useApp((s) => s.view);
  const setView = useApp((s) => s.setView);
  const info = useApp((s) => s.info);
  const loadInfo = useApp((s) => s.loadInfo);
  const saveSettings = useApp((s) => s.saveSettings);
  const toast = useApp((s) => s.toast);
  const fail = useApp((s) => s.fail);
  const refreshFind = useFind((s) => s.refresh);
  const startTour = useTour((s) => s.start);

  useEffect(() => {
    void loadInfo();
    const unlisten = [
      listen(CHANGED_EVENT, () => {
        void loadInfo();
        if (useApp.getState().view === "find") void refreshFind();
      }),
      listen<SortedEvent>(SORTED_EVENT, (e) => toast(`${e.name} is in ${e.folder}.`)),
    ];
    return () => {
      for (const u of unlisten) u.then((fn) => fn()).catch(() => undefined);
    };
  }, [loadInfo, refreshFind, toast]);

  useEffect(() => {
    if (info && !info.settings.tourDone) startTour();
  }, [info?.settings.tourDone, info, startTour]);

  const finishTour = () => {
    if (info && !info.settings.tourDone) void saveSettings({ ...info.settings, tourDone: true });
  };

  const openSchoolFiles = () => ipc.openSchoolFiles().catch(fail);

  return (
    <div className="app" data-testid="app">
      <nav className="sidebar" aria-label="Main" data-testid="sidebar">
        <div className="brand">
          <span className="brand-mark" aria-hidden>
            <Search size={18} strokeWidth={2.5} />
          </span>
          <span>Desk</span>
        </div>
        {NAV.map((item) => (
          <button key={item.id} type="button" className={`nav-button${view === item.id ? " is-active" : ""}`} onClick={() => setView(item.id)} aria-current={view === item.id ? "page" : undefined} data-tour={item.tour} data-testid={`nav-${item.id}`}>
            <item.icon size={22} aria-hidden />
            <span>{item.label}</span>
          </button>
        ))}
        <button type="button" className="nav-button" onClick={() => void openSchoolFiles()} data-testid="nav-folders">
          <FolderOpen size={22} aria-hidden />
          <span>Sorted folders</span>
        </button>
        <div className="sidebar-spacer" />
        <button type="button" className={`nav-button${view === "settings" ? " is-active" : ""}`} onClick={() => setView("settings")} aria-current={view === "settings" ? "page" : undefined} data-testid="nav-settings">
          <Settings size={22} aria-hidden />
          <span>Settings</span>
        </button>
      </nav>
      <main className="main" data-testid="main">
        {info === null ? (
          <p className="status-line">Starting Desk...</p>
        ) : view === "find" ? (
          <FindView info={info} />
        ) : view === "cleanup" ? (
          <CleanupView info={info} />
        ) : view === "activity" ? (
          <ActivityView info={info} />
        ) : (
          <SettingsView key={JSON.stringify(info.settings)} info={info} />
        )}
      </main>
      <Toasts />
      <Tour onFinish={finishTour} />
    </div>
  );
}
